//! Narrative source in the project folder, and the presentation sidecars that
//! sit beside it without ever getting into it.
//!
//! Two file trees, and the whole of #185 is the wall between them:
//!
//! ```text
//! narrative/
//! ├── world.yaml                    canonical facts, beliefs and quests
//! ├── state.yaml                    the declared variables, project-wide
//! ├── scenes/<slug>.yaml            one SceneDocument each — canonical source
//! └── layout/
//!     ├── scenes/<scene-ulid>.json  where the boxes are, per scene
//!     └── arcs/<slug>.json          where the boxes are, per arc graph
//! ```
//!
//! ## Why `narrative/` and not `nodes/`
//!
//! A scene is not a [`Node`](wobu_core::Node). Nodes are one record type with a
//! kind, Markdown frontmatter and a body of prose, walked by
//! `Project::reconcile` and indexed for the influence engine; a scene is a
//! structured YAML document with a beat graph inside it. Filing scenes under
//! `nodes/<kind>/` would put a document the Markdown parser cannot read into
//! the tree the Markdown parser walks, and every art-only project would open
//! with a folder full of "malformed node file". Separating the trees is also
//! what makes the promise in #153 cheap to keep: an existing project that has
//! no `narrative/` directory reads exactly as it did before, because nothing
//! that runs at open looks for one.
//!
//! ## Why layout is a separate tree rather than an adjacent file
//!
//! `narrative/scenes/kiln.layout.json` would be a sidecar in the literal sense
//! and would be wrong in the way that matters. Everything that must never see
//! layout — a compiler input set, a build fingerprint, an exported package, a
//! `git add narrative/scenes` — is naturally expressed as a directory. Put the
//! two in one directory and each of those becomes a filename rule that some
//! future caller will write slightly differently, and the first one to get it
//! wrong ships a coordinate inside a game package. One directory boundary is a
//! rule that cannot be got subtly wrong. [`source_fingerprint`] is the proof:
//! it walks source paths and structurally cannot reach `narrative/layout/`.
//!
//! ## Why layout files are named by ULID and source files by slug
//!
//! Source files are named by a slug because a person reads that directory —
//! in git, in Obsidian, in a file browser — and `kiln-interrogation.yaml` is
//! worth more there than a ULID. Layout files are named by the scene id
//! because *nothing* reads that directory, and because the layout is keyed by
//! stable ids in every other respect. Naming it by slug would make the file
//! name the one part of the arrangement that depends on a display name, and
//! the day something renames a scene's file to follow its title, every
//! arrangement in the project is orphaned at once. It also makes garbage
//! collection a set difference between two sets of ids rather than a join
//! through a name.

pub mod layout;
pub mod library;
pub mod publication;
pub mod records;
pub mod registry;
pub mod world;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use wobu_narrative::{Scene, SceneDocument, SceneId, StateDocument};

use crate::atomic::{self, Stamp, WriteOutcome};
use crate::error::{Error, Result};
use crate::paths;

/// The narrative tree, relative to the project root.
pub const NARRATIVE_DIR: &str = "narrative";
/// Where the canonical scene documents live.
pub const SCENES_DIR: &str = "narrative/scenes";
/// The project-wide declared variables (#155's seam — see [`StateDocument`]).
pub const STATE_FILE: &str = "narrative/state.yaml";
/// Source files carry `.yaml` and layout files carry `.json`, which is not
/// decoration: the extension is the second, redundant signal that a file is or
/// is not something the compiler may read.
pub const SOURCE_EXT: &str = "yaml";

pub fn narrative_dir(root: &Path) -> PathBuf {
    root.join(NARRATIVE_DIR)
}

pub fn scenes_dir(root: &Path) -> PathBuf {
    paths::from_rel_string(root, SCENES_DIR)
}

/// `narrative/scenes/kiln-interrogation.yaml`
pub fn scene_rel(slug: &str) -> String {
    format!("{SCENES_DIR}/{slug}.{SOURCE_EXT}")
}

/// What happened to a source write.
///
/// Source keeps Wobu's never-merge rule in full: the loser of a race lands
/// beside the winner as a `.conflict-*.yaml` sibling and a human decides. The
/// merging write in [`layout`] is the exception, and it is an exception for
/// files that contain no words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSave {
    Saved(Stamp),
    Conflict { conflict_path: String },
}

/// One scene as it exists on disk, with what we knew about the file when we
/// read it.
///
/// The stamp travels with the document rather than living in the index,
/// because there is no scene table in the index yet and inventing one here
/// would make the local cache the owner of the precondition that stops two
/// writers clobbering each other — which `docs/02-data-model.md` is explicit
/// that it must never be. Indexing scenes is the remaining half of #153.
#[derive(Debug, Clone)]
pub struct SceneFile {
    pub scene: Scene,
    /// Project-relative, `/`-separated. Stable across a rename: the slug is
    /// minted once, exactly as `Node::slug` is.
    pub rel: String,
    /// `None` means "we believe this file is new". A file that turns out to
    /// exist anyway is a conflict, not an overwrite.
    pub stamp: Option<Stamp>,
}

impl SceneFile {
    pub fn slug(&self) -> &str {
        self.rel
            .rsplit('/')
            .next()
            .and_then(|name| name.strip_suffix(&format!(".{SOURCE_EXT}")))
            .unwrap_or_default()
    }
}

/// A scene file the catalog could see but not understand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableSource {
    pub rel: String,
    pub reason: String,
}

/// Every scene in the project, and every file in the scenes directory that
/// could not be identified.
///
/// The unreadable list is returned rather than swallowed for the same reason
/// `Project::corrupt_files` exists: a scene that a sync client truncated has
/// no id, so it cannot appear as a scene, and a catalog that silently omitted
/// it would present the file as deleted.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub scenes: Vec<SceneEntry>,
    pub unreadable: Vec<UnreadableSource>,
}

impl Catalog {
    pub fn ids(&self) -> BTreeSet<SceneId> {
        self.scenes.iter().map(|entry| entry.id).collect()
    }

    pub fn find(&self, id: SceneId) -> Option<&SceneEntry> {
        self.scenes.iter().find(|entry| entry.id == id)
    }

    pub fn slugs(&self) -> BTreeSet<String> {
        self.scenes.iter().map(|entry| entry.slug.clone()).collect()
    }
}

/// What one scene file says about itself, without parsing the beats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneEntry {
    pub id: SceneId,
    pub name: String,
    pub slug: String,
    pub rel: String,
}

/// Read `scene.id` and `scene.name` and nothing else.
///
/// Deliberately permissive, in the same spirit as `SceneDocument`'s version
/// probe: listing the scenes in a project must keep working when one of them
/// is mid-edit in a text editor, and a catalog that refused to name a file it
/// could partly read would leave the user unable to open the very scene they
/// need to repair.
#[derive(serde::Deserialize)]
struct Probe {
    scene: ProbeScene,
}

#[derive(serde::Deserialize)]
struct ProbeScene {
    id: SceneId,
    #[serde(default)]
    name: String,
}

/// Files directly under `narrative/scenes/`, conflict siblings excluded.
///
/// Depth 1: the scenes directory is deliberately flat. Nesting would put the
/// project's structure into the filesystem, where a rename would move files
/// and orphan everything keyed to their paths — the same argument that keeps
/// `nodes/<kind>/` two levels deep and no more.
pub(crate) fn scene_paths(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let probe = registry::safe_path(root, "narrative/scenes/probe.yaml")?;
    let directory = probe.parent().expect("scene parent");
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(Error::io(directory, error)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(directory, error))?;
        let rel = format!("narrative/scenes/{}", entry.file_name().to_string_lossy());
        if registry::classify(&rel) == Some(registry::NarrativeFileKind::Scene) {
            paths.push((rel.clone(), registry::safe_path(root, &rel)?));
        }
    }
    paths.sort();
    Ok(paths)
}

/// Whether a project-relative path is canonical narrative source.
///
/// The complement of [`layout::is_layout_path`], and the one place anything
/// else should ask. A caller that spelled the prefix itself is a caller that
/// can get it subtly wrong.
///
/// [`layout::is_layout_path`]: crate::narrative::layout::is_layout_path
pub fn is_source_path(rel: &str) -> bool {
    matches!(
        registry::classify(rel),
        Some(
            registry::NarrativeFileKind::Scene
                | registry::NarrativeFileKind::State
                | registry::NarrativeFileKind::World
        )
    )
}

/// Conflict siblings `guarded_write` parked beside a narrative source file.
///
/// Listed so the existing conflict card can show them. Source keeps the
/// never-merge rule, which is only worth anything if a losing version is
/// actually reachable from the UI — a sibling nobody can see is a sibling
/// nobody resolves, and eventually somebody deletes the folder full of them.
pub fn conflict_paths(root: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let directories = std::iter::once(narrative_dir(root)).chain(
        ["scenes", "scenarios", "proposals", "receipts", "policies", "production", "publications"]
            .into_iter()
            .map(|dir| root.join("narrative").join(dir)),
    );
    for dir in directories {
        let probe = if dir == narrative_dir(root) {
            "narrative/state.yaml".into()
        } else {
            let leaf = if dir.ends_with("scenes") {
                "probe.yaml"
            } else {
                "00000000000000000000000000.json"
            };
            paths::to_rel_string(dir.join(leaf).strip_prefix(root).unwrap())
        };
        if registry::safe_path(root, &probe).is_err() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for path in entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
        {
            let is_sibling = path
                .file_name()
                .is_some_and(|name| crate::conflict::is_sibling(&name.to_string_lossy()));
            let is_source = path.extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case(SOURCE_EXT) || ext.eq_ignore_ascii_case("json")
            });
            if is_sibling
                && is_source
                && let Ok(rel) = path.strip_prefix(root)
            {
                found.push((paths::to_rel_string(rel), path.clone()));
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// The display name of the scene in a source file, for a conflict card.
///
/// `None` for `state.yaml`, which has no name, and for anything that will not
/// probe — a card that says "someone, at some point" is still resolvable, and
/// refusing to list a sibling we cannot label would strand somebody's only
/// copy of a scene.
pub fn scene_name_at(root: &Path, rel: &str) -> Option<String> {
    let path = if registry::classify(rel).is_some() {
        registry::safe_path(root, rel).ok()?
    } else {
        conflict_paths(root).into_iter().find(|(candidate, _)| candidate == rel)?.1
    };
    let text = std::fs::read_to_string(path).ok()?;
    registry::parse(rel, &text)
        .ok()
        .map(|(_, name, _)| name)
        .or_else(|| serde_norway::from_str::<Probe>(&text).ok().map(|probe| probe.scene.name))
}

pub fn catalog(root: &Path) -> Result<Catalog> {
    let mut catalog = Catalog::default();
    for (rel, path) in scene_paths(root)? {
        let Some((text, _)) = atomic::read_stamped(&path)? else { continue };
        match serde_norway::from_str::<Probe>(&text) {
            Ok(probe) => catalog.scenes.push(SceneEntry {
                id: probe.scene.id,
                name: probe.scene.name,
                slug: slug_of(&rel).to_string(),
                rel,
            }),
            Err(error) => {
                catalog.unreadable.push(UnreadableSource { rel, reason: error.to_string() })
            }
        }
    }
    Ok(catalog)
}

fn slug_of(rel: &str) -> &str {
    rel.rsplit('/').next().and_then(|name| name.split('.').next()).unwrap_or_default()
}

/// Read one scene, whole and strictly.
///
/// Strict where [`catalog`] is permissive, and the split is the point: naming
/// a file is a listing operation that must survive a broken file, while
/// opening one is an operation whose result a person is about to edit and save
/// back. Accepting a document we only half understood and then rewriting it is
/// how a mistyped key becomes silent data loss — see `wobu_narrative::source`.
pub fn read_scene(root: &Path, rel: &str) -> Result<SceneFile> {
    let path = registry::safe_path(root, rel)?;
    let Some((text, stamp)) = atomic::read_stamped(&path)? else {
        return Err(Error::io(&path, std::io::Error::from(std::io::ErrorKind::NotFound)));
    };
    let document = SceneDocument::parse(&text)
        .map_err(|error| Error::Malformed { path, reason: error.to_string() })?;
    Ok(SceneFile { scene: document.scene, rel: rel.to_string(), stamp: Some(stamp) })
}

/// Write a scene through the guarded path, refusing to clobber a concurrent
/// edit.
///
/// Takes `&mut SceneFile` and updates its stamp on success, so a caller that
/// saves twice in a row cannot accidentally present the first save's
/// precondition to the second and park its own work as a conflict.
pub fn write_scene(root: &Path, file: &mut SceneFile, peer: &str) -> Result<SourceSave> {
    let document = SceneDocument::new(file.scene.clone());
    let text = document.to_yaml().map_err(|error| Error::Malformed {
        path: paths::from_rel_string(root, &file.rel),
        reason: error.to_string(),
    })?;
    let path = registry::safe_path(root, &file.rel)?;
    match atomic::guarded_write(root, &path, &text, file.stamp.as_ref(), peer)? {
        WriteOutcome::Written(stamp) => {
            file.stamp = Some(stamp.clone());
            Ok(SourceSave::Saved(stamp))
        }
        WriteOutcome::Conflict { conflict_path, .. } => {
            Ok(SourceSave::Conflict { conflict_path: relative(root, &conflict_path) })
        }
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).map(paths::to_rel_string).unwrap_or_else(|_| paths::to_rel_string(path))
}

/// The declared state variables, or `None` when the project has never had any.
pub fn read_state(root: &Path) -> Result<Option<(StateDocument, Stamp)>> {
    let path = registry::safe_path(root, STATE_FILE)?;
    let Some((text, stamp)) = atomic::read_stamped(&path)? else { return Ok(None) };
    let document = StateDocument::parse(&text)
        .map_err(|error| Error::Malformed { path, reason: error.to_string() })?;
    Ok(Some((document, stamp)))
}

pub fn write_state(
    root: &Path,
    document: &StateDocument,
    expected: Option<&Stamp>,
    peer: &str,
) -> Result<SourceSave> {
    let path = registry::safe_path(root, STATE_FILE)?;
    let text = document
        .to_yaml()
        .map_err(|error| Error::Malformed { path: path.clone(), reason: error.to_string() })?;
    match atomic::guarded_write(root, &path, &text, expected, peer)? {
        WriteOutcome::Written(stamp) => Ok(SourceSave::Saved(stamp)),
        WriteOutcome::Conflict { conflict_path, .. } => {
            Ok(SourceSave::Conflict { conflict_path: relative(root, &conflict_path) })
        }
    }
}

/// A hash over every byte of narrative source in the project, and nothing else.
///
/// This is the artefact that makes #185's central claim checkable rather than
/// merely asserted. It walks `narrative/state.yaml`, `narrative/world.yaml` and `narrative/scenes/`,
/// so `narrative/layout/` is not excluded by a filter somebody could delete —
/// it is unreachable from here. A layout-only edit therefore cannot move this
/// value, and a test that hashes it before and after a drag is a proof rather
/// than a spot check.
///
/// The path is hashed alongside the bytes so that renaming a scene file
/// changes the fingerprint: two projects whose scenes have swapped filenames
/// are not the same project, and a build keyed only on contents would think
/// they were. Lengths are prefixed for the reason
/// `wobu_narrative::Revision::of` prefixes them — otherwise the boundary
/// between a path and its contents is guessable and two different trees can
/// collide.
pub fn source_fingerprint(root: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"wobu-store/narrative-source/1");

    let mut files: Vec<(String, PathBuf)> = scene_paths(root)?;
    let state = registry::safe_path(root, STATE_FILE)?;
    if state.is_file() {
        files.push((STATE_FILE.to_string(), state));
    }
    let world = registry::safe_path(root, world::WORLD_FILE)?;
    if world.is_file() {
        files.push((self::world::WORLD_FILE.to_string(), world));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));

    for (rel, path) in files {
        let bytes = std::fs::read(&path).map_err(|error| Error::io(&path, error))?;
        for part in [rel.as_bytes(), bytes.as_slice()] {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part);
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub(crate) fn probe_scene(text: &str) -> Option<(String, String)> {
    serde_norway::from_str::<Probe>(text)
        .ok()
        .map(|probe| (probe.scene.id.to_string(), probe.scene.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_path_is_project_relative_and_slash_separated() {
        assert_eq!(scene_rel("kiln-interrogation"), "narrative/scenes/kiln-interrogation.yaml");
    }

    #[test]
    fn a_project_with_no_narrative_tree_has_an_empty_catalog() {
        // The whole of "existing art-only projects open unchanged": nothing
        // here creates a directory, and a missing tree is not an error.
        let dir = tempfile::tempdir().unwrap();
        let catalog = catalog(dir.path()).unwrap();
        assert!(catalog.scenes.is_empty());
        assert!(!narrative_dir(dir.path()).exists());
    }

    #[test]
    fn the_fingerprint_of_an_empty_project_is_stable() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_eq!(source_fingerprint(a.path()).unwrap(), source_fingerprint(b.path()).unwrap());
    }
}
