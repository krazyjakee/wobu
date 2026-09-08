//! The narrative half of a project folder, on the same terms as the rest of it.
//!
//! Everything here is a thin layer over [`crate::narrative`]: the write
//! preconditions (`ensure_writable`), this installation's peer alias, and the
//! one place that knows both a scene and its layout — deleting one.
//!
//! Scenes are deliberately **not** in the SQLite index yet. That is the
//! remaining half of #153, and it is a cache question rather than a
//! correctness one: the folder is canonical, and every function here reads it.
//! The cost today is that listing scenes re-reads the scenes directory, which
//! on a share is the asymmetry `docs/07-file-shares.md` warns about; the thing
//! that must not happen in the meantime is a scene's guarded-write
//! precondition living only in a local cache, and it does not — the stamp
//! travels on [`SceneFile`].

use std::collections::BTreeSet;

use wobu_narrative::{Scene, SceneId, StateDocument, StateSchema};

use super::*;
use crate::atomic::Stamp;
use crate::error::{Error, Result};
use crate::narrative::layout::{self, GraphKey, Layout, LayoutLoad, LayoutSave};
use crate::narrative::{self as source, Catalog, SceneFile, SourceSave};

impl Project {
    /* ── source ──────────────────────────────────────────────────────── */

    /// Every scene in the folder, plus any file in the scenes directory that
    /// could not be identified.
    pub fn scene_catalog(&self) -> Result<Catalog> {
        source::catalog(&self.root)
    }

    /// The scene ids the destination checker in `wobu-narrative` needs.
    pub fn scene_ids(&self) -> Result<BTreeSet<SceneId>> {
        Ok(self.scene_catalog()?.ids())
    }

    pub fn load_scene(&self, id: SceneId) -> Result<SceneFile> {
        let catalog = self.scene_catalog()?;
        let entry = catalog.find(id).ok_or_else(|| {
            // The folder having gone is a different fact from the scene not
            // existing, and telling a writer their scene is missing when the
            // NAS is merely unplugged is both wrong and alarming — the same
            // distinction `get_node` makes.
            if self.is_present() { Error::NoSuchNode(id.to_string()) } else { Error::Disconnected }
        })?;
        source::read_scene(&self.root, &entry.rel)
    }

    /// Create a scene and write it.
    ///
    /// The slug is minted once from the name and never recomputed, exactly as
    /// `Node::slug` is: renaming a scene must not move its file, because a
    /// moved file is a delete and a create to every sync client, a watcher and
    /// a git history looking at the folder.
    pub fn create_scene(&mut self, name: &str) -> Result<SceneFile> {
        self.ensure_writable()?;
        let taken = self.scene_catalog()?.slugs();
        let base = wobu_core::slugify(name)?;
        let slug = wobu_core::unique_slug(&base, &|candidate| taken.contains(candidate));

        let mut file =
            SceneFile { scene: Scene::new(name), rel: source::scene_rel(&slug), stamp: None };
        match source::write_scene(&self.root, &mut file, &self.peer)? {
            SourceSave::Saved(_) => {
                self.index_narrative_path(&file.rel)?;
                Ok(file)
            }
            SourceSave::Conflict { conflict_path } => {
                Err(Error::AlreadyExists(paths::from_rel_string(&self.root, &conflict_path)))
            }
        }
    }

    /// Save an edited scene, refusing to clobber a concurrent edit.
    ///
    /// Source keeps the never-merge rule in full: a losing write is parked as
    /// a `.conflict-*.yaml` sibling for a human. Layout is the documented
    /// exception and lives in a different file, so nothing on this path can be
    /// blocked by an arrangement.
    pub fn save_scene(&mut self, file: &mut SceneFile) -> Result<SourceSave> {
        self.ensure_writable()?;
        let outcome = source::write_scene(&self.root, file, &self.peer)?;
        if matches!(outcome, SourceSave::Saved(_)) {
            self.index_narrative_path(&file.rel)?;
        }
        Ok(outcome)
    }

    /// Delete a scene and the arrangement that described it.
    ///
    /// The layout goes with the source, and it goes *after* it: a crash
    /// between the two leaves an orphan cosmetic file that [`sweep`] collects,
    /// where the other order would leave a scene whose arrangement is gone.
    ///
    /// Nothing rewrites the scenes that link *to* this one. That is
    /// `wobu-narrative`'s stated position — a destination pointing at a
    /// removed scene becomes a diagnostic that names it, rather than a silent
    /// repair with nothing to undo.
    ///
    /// [`sweep`]: crate::narrative::layout::sweep
    pub fn delete_scene(&mut self, id: SceneId) -> Result<()> {
        self.ensure_writable()?;
        let catalog = self.scene_catalog()?;
        let entry = catalog.find(id).ok_or_else(|| Error::NoSuchNode(id.to_string()))?;
        let Some((_, stamp)) = source::registry::read(self.root(), &entry.rel)? else {
            return Err(Error::NoSuchNode(id.to_string()));
        };
        if !self.delete_narrative_file(&entry.rel, &stamp)? {
            return Err(Error::Malformed {
                path: entry.rel.clone().into(),
                reason: "Scene changed before deletion; reload it before trying again.".into(),
            });
        }
        layout::delete(&self.root, &GraphKey::of_scene(id))
    }

    /// The declared variables, or `None` in a project that has never had any.
    pub fn state_document(&self) -> Result<Option<(StateDocument, Stamp)>> {
        source::read_state(&self.root)
    }

    /// The checked schema conditions and effects are typed against. An absent
    /// file is an empty schema rather than an error: a project can have scenes
    /// long before it has state.
    pub fn state_schema(&self) -> Result<StateSchema> {
        match self.state_document()? {
            Some((document, _)) => document.schema().map_err(|error| Error::Malformed {
                path: paths::from_rel_string(&self.root, source::STATE_FILE),
                reason: error.to_string(),
            }),
            None => Ok(StateSchema::default()),
        }
    }

    pub fn save_state(
        &mut self,
        document: &StateDocument,
        expected: Option<&Stamp>,
    ) -> Result<SourceSave> {
        self.ensure_writable()?;
        let outcome = source::write_state(&self.root, document, expected, &self.peer)?;
        if matches!(outcome, SourceSave::Saved(_)) {
            self.index_narrative_path(source::STATE_FILE)?;
        }
        Ok(outcome)
    }

    /// A hash over narrative source and nothing else. Structurally unable to
    /// see `narrative/layout/` — see [`crate::narrative::source_fingerprint`].
    pub fn narrative_fingerprint(&self) -> Result<String> {
        source::source_fingerprint(&self.root)
    }

    /* ── layout ──────────────────────────────────────────────────────── */

    /// The arrangement for one scene, garbage-collected against it.
    ///
    /// Infallible on purpose. A scene opens whatever its sidecar says, or does
    /// not say, or cannot say.
    pub fn scene_layout(&self, scene: &Scene) -> LayoutLoad {
        layout::load_for_scene(&self.root, scene)
    }

    /// Save an arrangement, merging per node against whatever is on disk.
    ///
    /// Takes the scene as well as the layout so the write can prune positions
    /// for ids the scene no longer contains. The scene is the caller's — the
    /// one on the canvas — which is what makes the prune defensible; see
    /// [`layout::save`].
    pub fn save_scene_layout(&self, scene: &Scene, arrangement: &Layout) -> Result<LayoutSave> {
        self.ensure_writable()?;
        layout::save(&self.root, &self.peer, arrangement, Some(&layout::scene_keys(scene)))
    }

    /// The arrangement for one arc graph, whose nodes are scenes.
    ///
    /// Pruned against the scene catalog rather than against a source document,
    /// because arcs have no source document yet (#151, #155).
    pub fn arc_layout(&self, arc: &str) -> LayoutLoad {
        let mut loaded = layout::load(&self.root, &GraphKey::Arc { arc: arc.to_string() });
        if let Ok(catalog) = self.scene_catalog() {
            let present =
                catalog.ids().into_iter().map(layout::NodeKey::Scene).collect::<BTreeSet<_>>();
            layout::reconcile(&mut loaded, &present);
        }
        loaded
    }

    pub fn save_arc_layout(&self, arrangement: &Layout) -> Result<LayoutSave> {
        self.ensure_writable()?;
        let catalog = self.scene_catalog()?;
        let present =
            catalog.ids().into_iter().map(layout::NodeKey::Scene).collect::<BTreeSet<_>>();
        layout::save(&self.root, &self.peer, arrangement, Some(&present))
    }

    /// Collect scene layout files whose scene no longer exists.
    ///
    /// A maintenance action, not something that runs at open — a half-mounted
    /// share looks exactly like a project whose scenes were all deleted. See
    /// [`layout::sweep`].
    pub fn sweep_scene_layouts(&self) -> Result<Vec<String>> {
        self.ensure_writable()?;
        layout::sweep(&self.root, &self.scene_catalog()?.ids())
    }
}
