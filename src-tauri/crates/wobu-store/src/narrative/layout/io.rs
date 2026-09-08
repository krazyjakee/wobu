//! Strict presentation IO. It is deliberately outside the source registry.
use super::*;

pub const MAX_LAYOUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_DECORATIONS: usize = 1_000;

impl Layout {
    pub fn validate(&self) -> Result<()> {
        let bad = |reason: &str| Error::Malformed {
            path: self.graph.rel().unwrap_or_default().into(),
            reason: reason.into(),
        };
        if self.schema_version > LAYOUT_SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                found: self.schema_version,
                supported: LAYOUT_SCHEMA_VERSION,
            });
        }
        if self.schema_version == 0 {
            return Err(bad("Unsupported layout schema version."));
        }
        self.graph.rel()?;
        if self.nodes.len() > MAX_ENTRIES
            || self.groups.len() > MAX_DECORATIONS
            || self.annotations.len() > MAX_DECORATIONS
            || self.removed_groups.len() > MAX_ENTRIES
            || self.removed_annotations.len() > MAX_ENTRIES
        {
            return Err(bad("The arrangement exceeds its entry limit."));
        }
        let coordinate = |v: f64| v.is_finite() && v.abs() <= 10_000_000.0;
        let size =
            |v: Option<f64>| v.is_none_or(|v| v.is_finite() && (1.0..=10_000.0).contains(&v));
        let valid_key = |key: &NodeKey| {
            matches!(
                (&self.graph, key),
                (
                    GraphKey::Scene { .. },
                    NodeKey::Scene(_) | NodeKey::Beat(_) | NodeKey::Choice(_) | NodeKey::Outcome(_)
                ) | (GraphKey::Arc { .. } | GraphKey::Quest { .. }, NodeKey::Scene(_))
            )
        };
        for (key, node) in &self.nodes {
            if !coordinate(node.x) || !coordinate(node.y) {
                return Err(bad("Node coordinates must be finite and within the canvas bounds."));
            }
            if !valid_key(key) {
                return Err(bad("A node key does not belong to this graph level."));
            }
        }
        for (id, group) in &self.groups {
            if id != &group.id
                || group.label.len() > 256
                || group.members.len() > MAX_ENTRIES
                || group.members.iter().any(|key| !valid_key(key))
            {
                return Err(bad("Invalid group identity, label or membership count."));
            }
        }
        for (id, note) in &self.annotations {
            if id != &note.id
                || note.attached_to.is_some_and(|key| !valid_key(&key))
                || note.body.len() > 8192
                || !coordinate(note.x)
                || !coordinate(note.y)
                || !size(note.width)
                || !size(note.height)
            {
                return Err(bad("Invalid note identity, text length or geometry."));
            }
        }
        if self.to_json()?.len() > MAX_LAYOUT_BYTES {
            return Err(bad("The arrangement exceeds its byte limit."));
        }
        Ok(())
    }
    pub fn remove_group(&mut self, id: GroupId) {
        self.removed_groups.insert(id, Utc::now());
        self.prune_removed();
    }
    pub fn remove_annotation(&mut self, id: AnnotationId) {
        self.removed_annotations.insert(id, Utc::now());
        self.prune_removed();
    }
    pub(super) fn prune_removed(&mut self) {
        self.groups
            .retain(|id, item| self.removed_groups.get(id).is_none_or(|at| item.updated_at > *at));
        self.annotations.retain(|id, item| {
            self.removed_annotations.get(id).is_none_or(|at| item.updated_at > *at)
        });
        for node in self.nodes.values_mut() {
            if node.group.is_some_and(|id| {
                self.removed_groups.contains_key(&id) && !self.groups.contains_key(&id)
            }) {
                node.group = None;
            }
        }
    }
}

pub fn graph_at(rel: &str) -> Result<GraphKey> {
    let parts: Vec<_> = rel.split('/').collect();
    let invalid = || Error::Malformed {
        path: rel.into(),
        reason: "Not a registered Flow layout path.".into(),
    };
    if parts.len() != 4 || parts[0] != "narrative" || parts[1] != "layout" {
        return Err(invalid());
    }
    let stem = parts[3].strip_suffix(".json").ok_or_else(invalid)?;
    let graph = match parts[2] {
        "scenes" => GraphKey::Scene { scene: stem.parse().map_err(|_| invalid())? },
        "quests" => GraphKey::Quest { quest: stem.parse().map_err(|_| invalid())? },
        "arcs" => GraphKey::Arc { arc: stem.into() },
        _ => return Err(invalid()),
    };
    if graph.rel()? != rel {
        return Err(invalid());
    }
    Ok(graph)
}
pub(super) fn safe_path(root: &Path, rel: &str) -> Result<PathBuf> {
    graph_at(rel)?;
    let mut path = root.to_path_buf();
    // graph_at already validates the exact normalized four-segment path.
    for segment in rel.split('/') {
        path.push(segment);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::Malformed {
                    path,
                    reason: "Layout paths cannot contain symbolic links.".into(),
                });
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(&path, e)),
        }
    }
    Ok(path)
}
pub(super) fn read_bounded(path: &Path) -> Result<Option<(String, Stamp)>> {
    atomic::read_stamped_bounded(path, MAX_LAYOUT_BYTES)
}
pub fn read_raw(root: &Path, rel: &str) -> Result<Option<(String, Stamp)>> {
    read_bounded(&safe_path(root, rel)?)
}
pub(super) fn parse(text: &str) -> Result<Layout> {
    if let Some(found) = probe_version(text)
        && found > LAYOUT_SCHEMA_VERSION
    {
        return Err(Error::SchemaTooNew { found, supported: LAYOUT_SCHEMA_VERSION });
    }
    let mut layout: Layout = serde_json::from_str(text)?;
    layout.validate()?;
    layout.prune_removed();
    Ok(layout)
}
pub fn read_document(root: &Path, rel: &str) -> Result<Option<(Layout, Stamp)>> {
    let path = safe_path(root, rel)?;
    let Some((text, stamp)) = read_bounded(&path)? else { return Ok(None) };
    let layout = parse(&text)?;
    if layout.graph.rel()? != rel {
        return Err(Error::Malformed {
            path,
            reason: "The arrangement describes another graph.".into(),
        });
    }
    Ok(Some((layout, stamp)))
}
pub fn manifest(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut result = Vec::new();
    for directory in ["scenes", "quests", "arcs"] {
        let leaf = if directory == "arcs" { "probe" } else { "00000000000000000000000000" };
        let probe = safe_path(root, &format!("narrative/layout/{directory}/{leaf}.json"))?;
        let parent = probe.parent().unwrap();
        let entries = match std::fs::read_dir(parent) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(Error::io(parent, e)),
        };
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(parent, e))?;
            let rel =
                format!("narrative/layout/{directory}/{}", entry.file_name().to_string_lossy());
            if graph_at(&rel).is_ok() {
                result.push((rel.clone(), safe_path(root, &rel)?));
            }
        }
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}
pub(super) fn preserve_unreadable(root: &Path, path: &Path, text: &str, peer: &str) -> Result<()> {
    let hash = atomic::hash_bytes(text.as_bytes());
    let stem = path.file_stem().unwrap().to_string_lossy();
    let target = path.with_file_name(format!("{stem}.{CORRUPT_MARKER}-{}.json", &hash[..16]));
    if std::fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(Error::Malformed {
            path: target,
            reason: "Layout recovery cannot use symbolic links.".into(),
        });
    }
    let _ = peer;
    atomic::write_once(root, &target, text.as_bytes())?;
    Ok(())
}

/// Disposable watcher evidence, including malformed/future file bytes. It never
/// enters canonical source indexes or compiler/context dependency fingerprints.
pub fn observation(root: &Path) -> String {
    let mut digest = blake3::Hasher::new();
    for directory in ["scenes", "quests", "arcs"] {
        let leaf = if directory == "arcs" { "probe" } else { "00000000000000000000000000" };
        let entries = safe_path(root, &format!("narrative/layout/{directory}/{leaf}.json"))
            .and_then(|probe| {
                std::fs::read_dir(probe.parent().unwrap()).map_err(|e| Error::io(&probe, e))
            });
        match entries {
            Ok(entries) => {
                let mut paths = Vec::new();
                for entry in entries {
                    match entry {
                        Ok(entry) => {
                            let rel = format!(
                                "narrative/layout/{directory}/{}",
                                entry.file_name().to_string_lossy()
                            );
                            if graph_at(&rel).is_ok() {
                                paths.push(rel);
                            }
                        }
                        Err(error) => {
                            digest.update(error.to_string().as_bytes());
                        }
                    }
                }
                paths.sort();
                for rel in paths {
                    digest.update(rel.as_bytes());
                    digest.update(&[0]);
                    let result = safe_path(root, &rel).and_then(|path| {
                        use std::io::Read;
                        std::fs::File::open(&path)
                            .and_then(|file| {
                                let meta = file.metadata()?;
                                if meta.len() > MAX_LAYOUT_BYTES as u64 {
                                    digest.update(&meta.len().to_le_bytes());
                                    digest.update(format!("{:?}", meta.modified()).as_bytes());
                                }
                                digest
                                    .update_reader(file.take(MAX_LAYOUT_BYTES as u64 + 1))
                                    .map(|_| ())
                            })
                            .map_err(|e| Error::io(&path, e))
                    });
                    if let Err(error) = result {
                        digest.update(error.to_string().as_bytes());
                    }
                    digest.update(&[0]);
                }
            }
            Err(error) => {
                digest.update(error.to_string().as_bytes());
            }
        }
    }
    digest.finalize().to_hex().to_string()
}
