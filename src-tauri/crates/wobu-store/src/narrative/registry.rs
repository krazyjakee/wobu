//! One allowlist for canonical file discovery, indexing and replication.
use super::records::{NarrativeRecordDocument, NarrativeRecordKind};
use crate::{
    Error, Result,
    atomic::{self, Stamp},
};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use wobu_core::Id;
use wobu_narrative::{SceneDocument, StateDocument, WorldDocument};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrativeFileKind {
    Scene,
    State,
    World,
    Record(NarrativeRecordKind),
    Publication,
    Object,
    Tombstone,
    ReceiptBinding,
    Restoration,
}

pub fn classify(rel: &str) -> Option<NarrativeFileKind> {
    if rel.contains(['\\', ':'])
        || rel.chars().any(char::is_control)
        || rel.split('/').any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    match rel {
        "narrative/state.yaml" => return Some(NarrativeFileKind::State),
        "narrative/world.yaml" => return Some(NarrativeFileKind::World),
        _ => {}
    }
    let parts: Vec<_> = rel.split('/').collect();
    if parts.len() != 3 || parts[0] != "narrative" || crate::conflict::is_sibling(parts[2]) {
        return None;
    }
    if parts[1] == "scenes" && parts[2].ends_with(".yaml") && parts[2].len() > 5 {
        return Some(NarrativeFileKind::Scene);
    }
    if parts[1] == "objects" && parts[2].strip_suffix(".json").is_some_and(valid_hash) {
        return Some(NarrativeFileKind::Object);
    }
    let id = parts[2].strip_suffix(".json")?.parse::<Id>().ok()?;
    if id.to_string() != parts[2].trim_end_matches(".json") {
        return None;
    }
    match parts[1] {
        "publications" => Some(NarrativeFileKind::Publication),
        "deletions" => Some(NarrativeFileKind::Tombstone),
        "restorations" => Some(NarrativeFileKind::Restoration),
        "receipt-bindings" => Some(NarrativeFileKind::ReceiptBinding),
        directory => NarrativeRecordKind::ALL
            .into_iter()
            .find(|kind| kind.directory() == directory)
            .map(NarrativeFileKind::Record),
    }
}
pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Validate existing ancestors too: an in-project symlink is still mutable
/// redirection and must never turn a read/write into a different file's write.
pub fn safe_path(root: &Path, rel: &str) -> Result<PathBuf> {
    if classify(rel).is_none() {
        return Err(Error::Malformed {
            path: rel.into(),
            reason: "Not a registered canonical narrative path.".into(),
        });
    }
    let mut path = root.to_path_buf();
    for part in Path::new(rel).components() {
        if !matches!(part, Component::Normal(_)) {
            return Err(Error::NoSuchNode(rel.into()));
        }
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::Malformed {
                    path,
                    reason: "Canonical narrative paths cannot contain symbolic links.".into(),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io(&path, error)),
        }
    }
    Ok(path)
}

/// Flat directories only; staging, recovery, layout and machine-local secrets
/// are not registry entries and cannot be offered over peer transport.
pub fn paths(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut found = Vec::new();
    for rel in ["narrative/state.yaml", "narrative/world.yaml"] {
        let path = safe_path(root, rel)?;
        if path.exists() {
            found.push((rel.into(), path));
        }
    }
    let directories = [
        "scenes",
        "scenarios",
        "proposals",
        "receipts",
        "policies",
        "production",
        "publications",
        "objects",
        "deletions",
        "restorations",
        "receipt-bindings",
    ];
    for directory in directories {
        let rel = format!("narrative/{directory}");
        // safe_path requires a registered leaf; validating an absent probe also
        // validates both existing ancestors without creating anything.
        let probe = if directory == "scenes" {
            format!("{rel}/probe.yaml")
        } else if directory == "objects" {
            format!("{rel}/{}.json", "0".repeat(64))
        } else {
            format!("{rel}/00000000000000000000000000.json")
        };
        safe_path(root, &probe)?;
        let dir = root.join(&rel);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(Error::io(&dir, e)),
        };
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(&dir, e))?;
            let rel = format!("{rel}/{}", entry.file_name().to_string_lossy());
            if classify(&rel).is_some() {
                let path = safe_path(root, &rel)?;
                if !path.is_file() {
                    return Err(Error::Malformed {
                        path,
                        reason: "Canonical narrative source must be a regular file.".into(),
                    });
                }
                found.push((rel, path));
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeIndexEntry {
    pub rel: String,
    pub kind: NarrativeFileKind,
    pub id: Option<String>,
    pub name: String,
    pub hash: String,
    pub document: Option<serde_json::Value>,
    pub error: Option<String>,
    /// Internal objects/bindings and incomplete publications are never semantic query results.
    pub visible: bool,
}

pub fn parse(rel: &str, text: &str) -> Result<(Option<String>, String, serde_json::Value)> {
    let kind = classify(rel).ok_or_else(|| Error::NoSuchNode(rel.into()))?;
    let malformed = |error: String| Error::Malformed { path: rel.into(), reason: error };
    match kind {
        NarrativeFileKind::Scene => {
            let doc = SceneDocument::parse(text).map_err(|e| malformed(e.to_string()))?;
            Ok((Some(doc.scene.id.to_string()), doc.scene.name.clone(), serde_json::to_value(doc)?))
        }
        NarrativeFileKind::State => {
            let doc = StateDocument::parse(text).map_err(|e| malformed(e.to_string()))?;
            Ok((None, "State variables".into(), serde_json::to_value(doc)?))
        }
        NarrativeFileKind::World => {
            let doc = WorldDocument::parse(text).map_err(|e| malformed(e.to_string()))?;
            Ok((None, "World".into(), serde_json::to_value(doc)?))
        }
        NarrativeFileKind::Record(kind) => {
            let doc: NarrativeRecordDocument = serde_json::from_str(text)?;
            doc.validate()?;
            if doc.kind != kind || doc.rel() != rel {
                return Err(malformed("Record identity/kind does not match its path.".into()));
            }
            Ok((Some(doc.id.to_string()), doc.name.clone(), serde_json::to_value(doc)?))
        }
        _ => super::publication::parse(rel, text, kind),
    }
}

pub fn observe(root: &Path) -> Result<Vec<NarrativeIndexEntry>> {
    let mut result = Vec::new();
    for (rel, path) in paths(root)? {
        if let Some((text, stamp)) = atomic::read_stamped(&path)? {
            result.push(entry(root, &rel, &text, stamp));
        }
    }
    Ok(result)
}

pub fn entry(root: &Path, rel: &str, text: &str, stamp: Stamp) -> NarrativeIndexEntry {
    let kind = classify(rel).expect("registered paths only");
    let parsed = parse(rel, text).and_then(|(id, name, document)| {
        validate_references(root, kind, &document)?;
        Ok((id, name, document))
    });
    let (id, name, document, error) = match parsed {
        Ok((id, name, doc)) => (id, name, Some(doc), None),
        Err(e) => (None, rel.into(), None, Some(e.to_string())),
    };
    let visible = error.is_none()
        && !matches!(
            kind,
            NarrativeFileKind::Object
                | NarrativeFileKind::ReceiptBinding
                | NarrativeFileKind::Tombstone
                | NarrativeFileKind::Restoration
        );
    NarrativeIndexEntry {
        rel: rel.into(),
        kind,
        id,
        name,
        hash: stamp.hash,
        document,
        error,
        visible,
    }
}

pub fn read(root: &Path, rel: &str) -> Result<Option<(String, Stamp)>> {
    atomic::read_stamped(&safe_path(root, rel)?)
}

pub fn validate_references(
    root: &Path,
    kind: NarrativeFileKind,
    document: &serde_json::Value,
) -> Result<()> {
    match kind {
        NarrativeFileKind::Publication => {
            let manifest = serde_json::from_value(document.clone())?;
            super::publication::load_objects(root, &manifest)?;
        }
        NarrativeFileKind::Record(NarrativeRecordKind::Receipt) => {
            let receipt = serde_json::from_value(document.clone())?;
            super::publication::verify_receipt_if_bound(root, &receipt)?;
        }
        _ => {}
    }
    Ok(())
}
