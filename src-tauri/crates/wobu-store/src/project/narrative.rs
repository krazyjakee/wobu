//! The narrative half of a project folder, on the same terms as the rest of it.
//!
//! Everything here is a thin layer over [`crate::narrative`]: the write
//! preconditions (`ensure_writable`), this installation's peer alias, and the
//! one place that knows both a scene and its layout — deleting one.
//!
//! Canonical content stamps refresh derived locators; only changed scenes are
//! parsed again. A selected scene is then read from its unique canonical path
//! with its actual stamp. No cached document grants permission to save.

use std::collections::BTreeSet;

use wobu_narrative::{
    Name, Scene, SceneId, StateDocument, StateSchema, TextAsset, TextAssetId, TextKind,
};

use super::*;
use crate::atomic::Stamp;
use crate::error::{Error, Result};
use crate::narrative::layout::{self, GraphKey, Layout, LayoutLoad, LayoutSave};
use crate::narrative::{self as source, Catalog, SceneFile, SourceSave, TextCatalog, TextFile};

impl Project {
    /* ── source ──────────────────────────────────────────────────────── */

    /// Every scene in the folder, plus any file in the scenes directory that
    /// could not be identified.
    pub fn scene_catalog(&self) -> Result<Catalog> {
        Ok(self.scene_inventory()?.catalog)
    }

    /// The scene ids the destination checker in `wobu-narrative` needs.
    pub fn scene_ids(&self) -> Result<BTreeSet<SceneId>> {
        Ok(self.scene_catalog()?.ids())
    }

    pub fn load_scene(&self, id: SceneId) -> Result<SceneFile> {
        let inventory = self.scene_inventory()?;
        if inventory.ambiguous.contains(&id.to_string()) {
            return Err(Error::Malformed {
                path: self.root.join("narrative/scenes"),
                reason: format!(
                    "Scene ID {id} is ambiguous. Repair duplicate scene identities before opening it."
                ),
            });
        }
        let catalog = inventory.catalog;
        let entry = catalog.find(id).ok_or_else(|| {
            // The folder having gone is a different fact from the scene not
            // existing, and telling a writer their scene is missing when the
            // NAS is merely unplugged is both wrong and alarming — the same
            // distinction `get_node` makes.
            if self.is_present() { Error::NoSuchNode(id.to_string()) } else { Error::Disconnected }
        })?;
        let file = source::read_scene(&self.root, &entry.rel)?;
        if file.scene.id != id {
            return Err(Error::Malformed {
                path: entry.rel.clone().into(),
                reason: "Scene identity changed while opening it. Refresh the library.".into(),
            });
        }
        Ok(file)
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
        let outcome = self.save_editorial_scene(file)?;
        if matches!(outcome, SourceSave::Saved(_)) {
            let authored = file
                .scene
                .dialogue_slots()
                .flat_map(|(_, slot)| slot.variants.iter().map(|v| v.id))
                .collect();
            self.record_authored_narrative_dependencies_for(&authored)?;
            self.refresh_narrative_dependencies()?;
            *file = self.load_scene(file.scene.id)?;
            if let Some(stamp) = &file.stamp {
                return Ok(SourceSave::Saved(stamp.clone()));
            }
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
        let entry = self.scene_catalog()?.find(id).cloned();
        let rel = entry.ok_or_else(|| Error::NoSuchNode(id.to_string()))?.rel;
        self.delete_source(&rel, "Scene")?;
        layout::delete(&self.root, &GraphKey::of_scene(id))
    }

    /// Remove one canonical source document, refusing to delete a revision the
    /// caller has not seen.
    ///
    /// Shared by scenes and supporting text so the stamp check cannot be present
    /// on one path and absent on the other. `noun` only reaches the message a
    /// person reads.
    fn delete_source(&mut self, rel: &str, noun: &str) -> Result<()> {
        self.ensure_writable()?;
        let Some((_, stamp)) = source::registry::read(self.root(), rel)? else {
            return Err(Error::NoSuchNode(rel.to_string()));
        };
        if !self.delete_narrative_file(rel, &stamp)? {
            return Err(Error::Malformed {
                path: rel.into(),
                reason: format!("{noun} changed before deletion; reload it before trying again."),
            });
        }
        self.refresh_narrative_dependencies()?;
        Ok(())
    }

    /* ── supporting text (#167) ──────────────────────────────────────── */

    /// Every supporting text asset in the folder, plus any file in the texts
    /// directory that could not be identified.
    ///
    /// Read straight from disk rather than from the SQLite projection the scene
    /// library uses. The supporting-text directory is small enough that paging
    /// it would be premature, and the projection carries scene-shaped columns —
    /// beats, participants, coverage — that a bark has no answer for. Indexing
    /// it properly is the same work as adding it to the library search, and
    /// belongs with that.
    pub fn text_catalog(&self) -> Result<TextCatalog> {
        source::text_catalog(&self.root)
    }

    /// Every supporting text asset, parsed, in catalog order.
    ///
    /// The compiler's input. It refuses rather than skipping when one file is
    /// unreadable, because compiling the readable subset would produce a graph
    /// that silently omits content the author believes shipped.
    pub fn text_assets(&self) -> Result<Vec<TextAsset>> {
        let catalog = self.text_catalog()?;
        if let Some(bad) = catalog.unreadable.first() {
            return Err(Error::Malformed {
                path: bad.rel.clone().into(),
                reason: bad.reason.clone(),
            });
        }
        catalog
            .assets
            .iter()
            .map(|entry| source::read_text(&self.root, &entry.rel).map(|file| file.asset))
            .collect()
    }

    pub fn load_text_asset(&self, id: TextAssetId) -> Result<TextFile> {
        let catalog = self.text_catalog()?;
        let entry = catalog.find(id).ok_or_else(|| {
            if self.is_present() { Error::NoSuchNode(id.to_string()) } else { Error::Disconnected }
        })?;
        let file = source::read_text(&self.root, &entry.rel)?;
        if file.asset.id != id {
            return Err(Error::Malformed {
                path: entry.rel.clone().into(),
                reason: "Text asset identity changed while opening it. Refresh the library.".into(),
            });
        }
        Ok(file)
    }

    /// Create a supporting text asset and write it.
    ///
    /// The slug is minted once from the name, exactly as a scene's is, and for
    /// the same reason: renaming must not move the file.
    pub fn create_text_asset(
        &mut self,
        kind: TextKind,
        name: &str,
        event: Name,
    ) -> Result<TextFile> {
        self.ensure_writable()?;
        let taken = self.text_catalog()?.slugs();
        let base = wobu_core::slugify(name)?;
        let slug = wobu_core::unique_slug(&base, &|candidate| taken.contains(candidate));

        let mut file = TextFile {
            asset: TextAsset::new(kind, name, event),
            rel: source::text_rel(&slug),
            stamp: None,
        };
        match source::write_text(&self.root, &mut file, &self.peer)? {
            SourceSave::Saved(_) => {
                self.index_narrative_path(&file.rel)?;
                Ok(file)
            }
            SourceSave::Conflict { conflict_path } => {
                Err(Error::AlreadyExists(paths::from_rel_string(&self.root, &conflict_path)))
            }
        }
    }

    /// Save an edited supporting text asset, refusing to clobber a concurrent
    /// edit. Whole-document, guarded and never merged, exactly as a scene is.
    pub fn save_text_asset(&mut self, file: &mut TextFile) -> Result<SourceSave> {
        self.ensure_writable()?;
        let outcome = source::write_text(&self.root, file, &self.peer)?;
        if matches!(outcome, SourceSave::Saved(_)) {
            self.index_narrative_path(&file.rel)?;
            let authored = file
                .asset
                .lines()
                .flat_map(|(_, slot)| slot.variants.iter().map(|v| v.id))
                .collect();
            self.record_authored_narrative_dependencies_for(&authored)?;
            self.refresh_narrative_dependencies()?;
            *file = self.load_text_asset(file.asset.id)?;
            if let Some(stamp) = &file.stamp {
                return Ok(SourceSave::Saved(stamp.clone()));
            }
        }
        Ok(outcome)
    }

    /// Delete a supporting text asset.
    ///
    /// There is no layout to remove alongside it: a text asset has no canvas,
    /// which is the same absence that keeps [`Destination`] out of its model.
    ///
    /// [`Destination`]: wobu_narrative::Destination
    pub fn delete_text_asset(&mut self, id: TextAssetId) -> Result<()> {
        let entry = self.text_catalog()?.find(id).cloned();
        let rel = entry.ok_or_else(|| Error::NoSuchNode(id.to_string()))?.rel;
        self.delete_source(&rel, "Text asset")
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
            self.refresh_narrative_dependencies()?;
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

    pub fn quest_layout(&self, quest: wobu_narrative::EntityId) -> LayoutLoad {
        let mut loaded = layout::load(&self.root, &GraphKey::Quest { quest });
        if let Ok(present) = self.quest_layout_keys(quest) {
            layout::reconcile(&mut loaded, &present);
        }
        loaded
    }
    fn quest_layout_keys(
        &self,
        quest: wobu_narrative::EntityId,
    ) -> Result<BTreeSet<layout::NodeKey>> {
        let (world, _) =
            self.world_document()?.ok_or_else(|| crate::Error::NoSuchNode(quest.to_string()))?;
        let quest = world
            .quests
            .iter()
            .find(|record| record.id == quest)
            .ok_or_else(|| crate::Error::NoSuchNode(quest.to_string()))?;
        Ok(quest.scene_ids.iter().copied().map(layout::NodeKey::Scene).collect())
    }
    pub fn save_quest_layout(
        &self,
        arrangement: &Layout,
        quest: wobu_narrative::EntityId,
    ) -> Result<LayoutSave> {
        self.ensure_writable()?;
        layout::save(&self.root, &self.peer, arrangement, Some(&self.quest_layout_keys(quest)?))
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
