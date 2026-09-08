//! Dependency tracking against a real project folder (#168).
//!
//! [`wobu_narrative_deps`] is pure: give it scenes, texts, a world, a schema and
//! some character voices and it will tell you what each line depends on. This
//! module is the half that has to deal with a folder on a network share — which
//! means it has exactly two jobs the pure crate cannot do, and both of them are
//! about honesty rather than about IO.
//!
//! **Read a coherent snapshot.** A capture that read half a project before a
//! colleague's save landed and half after would produce a dependency set for a
//! project that never existed, and then confidently mark the wrong lines out of
//! date. So the whole read is bracketed: the source fingerprint and every
//! character node's stamp are taken before and compared after, and a snapshot
//! that moved under the reader is refused rather than published. This is the
//! same guard [`narrative_context::capture`](crate::project::narrative_context)
//! and [`ReviewSnapshot`](crate::project::narrative_review::ReviewSnapshot) use,
//! for the same reason and with the same words.
//!
//! **Never overwrite content.** Propagating an invalidation means setting one
//! flag. [`Project::mark_narrative_affected`] writes
//! [`Freshness::OutOfDate`](wobu_narrative::Freshness) and nothing else: the
//! body, the revision, the provenance, the generation policy and the review
//! state are all left exactly as they were, so a locked line stays locked, an
//! approved line stays recorded as approved while ceasing to be release-ready,
//! and every receipt that named the old fingerprint is still on disk naming it.
//!
//! ## What is not here
//!
//! Deciding what to *rebuild* — batching, ordering, cost, provider calls — is
//! #169. This module reports and it flags; it queues nothing.

use std::collections::{BTreeMap, BTreeSet};

use wobu_core::NodeKind;
use wobu_narrative::{
    EntityId, Provenance, Revision, Scene, StateSchema, TextAsset, VariantId, WorldDocument,
};
use wobu_narrative_context::Character;
use wobu_narrative_deps::{
    Affected, DependencyIndex, DependencySet, Producer, Snapshot, ToolVersions, capture,
};

use super::Project;
use crate::SceneFile;
use crate::atomic::Stamp;
use crate::error::{Error, Result};
use crate::narrative::records::NarrativeRecordKind;
use crate::narrative::{SourceSave, TextFile};

/// A character as read, beside the stamp of the file it was read from.
///
/// The stamp is the coherence check's half of the pair: `None` for a character
/// that does not exist, so that an entity appearing and an entity being edited
/// are both visible to a re-read.
type CharacterRead = BTreeMap<EntityId, (Option<Character>, Option<Stamp>)>;

fn invalid(message: impl Into<String>) -> Error {
    Error::Malformed { path: "narrative/dependencies".into(), reason: message.into() }
}

/// One coherent read of everything a capture is allowed to see.
///
/// Holds owned values rather than borrows because the read and the capture are
/// separated by the coherence check: the point of the exercise is that the
/// bytes being hashed are the bytes that were verified, and a borrow of a file
/// handle would not give that.
pub struct DependencySnapshot {
    pub scenes: Vec<Scene>,
    pub texts: Vec<TextAsset>,
    pub world: WorldDocument,
    pub schema: StateSchema,
    pub characters: BTreeMap<EntityId, Character>,
    pub producers: BTreeMap<VariantId, Producer>,
    pub versions: ToolVersions,
    /// The source fingerprint this snapshot was read at, kept so a caller can
    /// prove a later publication is describing the same project.
    pub fingerprint: String,
    observed: CharacterRead,
}

/// Immutable evidence of the input revision a particular wording was authored against.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DependencyReceipt {
    #[serde(rename = "type")]
    kind: String,
    entries: Vec<DependencyBaseline>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DependencyBaseline {
    parent: Option<wobu_core::Id>,
    revision: Revision,
    dependencies: DependencySet,
}

/// Chunk below the shared sync limit, including pretty-printed envelope bytes.
/// The conservative per-entry estimate avoids repeatedly serializing a growing
/// batch; the exact final length is checked before any chunk is published.
fn dependency_documents(
    entries: Vec<DependencyBaseline>,
) -> Result<Vec<crate::NarrativeRecordDocument>> {
    let limit = super::narrative_sync::MAX_NARRATIVE_FILE_BYTES;
    let mut chunks = Vec::new();
    let mut chunk = Vec::new();
    let mut size = 1024;
    for entry in entries {
        let json = serde_json::to_string_pretty(&entry)?;
        let cost = json.len() + json.lines().count() * 8 + 4;
        if cost + 1024 > limit {
            return Err(invalid("One dependency entry exceeds the portable narrative file limit."));
        }
        if size + cost > limit {
            chunks.push(std::mem::take(&mut chunk));
            size = 1024;
        }
        size += cost;
        chunk.push(entry);
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    chunks
        .into_iter()
        .map(|entries| {
            let document = crate::NarrativeRecordDocument::new(
                NarrativeRecordKind::Receipt,
                wobu_core::new_id(),
                "Narrative dependency baseline",
                serde_json::to_value(DependencyReceipt {
                    kind: "narrative_dependency_baseline".into(),
                    entries,
                })?,
            );
            if crate::narrative::publication::canonical_record_text(&document)?.len() > limit {
                return Err(invalid(
                    "Dependency receipt exceeds the portable narrative file limit.",
                ));
            }
            Ok(document)
        })
        .collect()
}

impl DependencySnapshot {
    /// Refuse publication from a snapshot whose canonical inputs moved.
    pub fn check_current(&self, project: &Project) -> Result<()> {
        if project.narrative_fingerprint()? != self.fingerprint
            || project.read_characters(&self.observed.keys().copied().collect())? != self.observed
        {
            return Err(invalid(
                "Narrative source changed while reading dependencies. Reload and try again.",
            ));
        }
        Ok(())
    }

    fn revisions(&self) -> BTreeMap<VariantId, Revision> {
        self.scenes
            .iter()
            .flat_map(|s| s.dialogue_slots().map(|(_, slot)| slot))
            .chain(self.texts.iter().flat_map(|a| a.lines().map(|(_, slot)| slot)))
            .flat_map(|slot| slot.variants.iter())
            .map(|v| (v.id, v.text.revision.clone()))
            .collect()
    }

    pub fn sets(&self) -> Vec<DependencySet> {
        capture(&Snapshot {
            scenes: &self.scenes,
            texts: &self.texts,
            world: &self.world,
            schema: &self.schema,
            characters: &self.characters,
            producers: &self.producers,
            versions: self.versions,
        })
    }

    pub fn index(&self) -> DependencyIndex {
        DependencyIndex::rebuild(self.sets())
    }
}

impl Project {
    /// The versions this build's fingerprints are taken under.
    ///
    /// A method on `Project` rather than a bare constant so that a future
    /// project-scoped override — a pinned prompt version for a shipped
    /// title — has one place to live.
    pub fn narrative_versions(&self) -> ToolVersions {
        ToolVersions::current()
    }

    /// A hash over narrative source *and the toolchain that reads it*.
    ///
    /// Distinct from [`Project::narrative_fingerprint`], which is deliberately
    /// bytes-of-source-only and is therefore the right thing to compare a
    /// snapshot against: a coherence check must not fail because the app was
    /// upgraded mid-read. This one is the right thing to key a *build* on,
    /// because the same bytes compiled by a different compiler, resolved by a
    /// different resolver or prompted with a different prompt are not the same
    /// build. Both exclude `narrative/layout/` structurally — see
    /// [`crate::narrative::source_fingerprint`].
    pub fn narrative_build_fingerprint(&self) -> Result<String> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"wobu-store/narrative-build/1");
        let source = self.narrative_fingerprint()?;
        let versions = serde_json::to_vec(&self.narrative_versions())?;
        for part in [source.as_bytes(), versions.as_slice()] {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part);
        }
        Ok(hasher.finalize().to_hex().to_string())
    }

    /// Read every canonical input a dependency capture needs, and refuse to
    /// return one that moved while it was being read.
    ///
    /// This is today's source input, compared with the immutable historical
    /// baseline reconstructed separately from canonical receipts.
    pub fn narrative_dependency_snapshot(&self) -> Result<DependencySnapshot> {
        self.read_dependency_snapshot(|| Ok(()))
    }

    fn read_dependency_snapshot(
        &self,
        before_check: impl FnOnce() -> Result<()>,
    ) -> Result<DependencySnapshot> {
        let fingerprint = self.narrative_fingerprint()?;
        let scenes = self.scenes()?;
        let texts = self.text_assets()?;
        let world = self.world_document()?.map(|(document, _)| document).unwrap_or_default();
        let schema = self.state_schema()?;
        let ids = referenced_entities(&scenes, &texts, &world);
        let observed = self.read_characters(&ids)?;
        let producers = self.narrative_producers(&scenes, &texts)?;

        before_check()?;
        // The coherence check. Character nodes are Markdown files outside the
        // narrative tree, so the source fingerprint says nothing about them and
        // they have to be re-read by hand.
        if self.narrative_fingerprint()? != fingerprint || self.read_characters(&ids)? != observed {
            return Err(invalid(
                "Narrative source changed while reading dependencies. Reload and try again.",
            ));
        }

        Ok(DependencySnapshot {
            scenes,
            texts,
            world,
            schema,
            observed: observed.clone(),
            characters: observed
                .into_iter()
                .filter_map(|(id, (character, _))| character.map(|value| (id, value)))
                .collect(),
            producers,
            versions: self.narrative_versions(),
            fingerprint,
        })
    }

    /// Reconstruct historical baselines from immutable canonical receipts. SQLite
    /// is only an accelerator; deleting it cannot acknowledge an outstanding change.
    pub fn narrative_dependencies(&self) -> Result<DependencyIndex> {
        let index = DependencyIndex::rebuild(
            self.dependency_receipts()?.into_values().map(|(_, r)| r.dependencies),
        );
        if self.index.narrative_dependencies()? != index {
            self.index.replace_narrative_dependencies(&index)?;
        }
        Ok(index)
    }

    fn dependency_receipts(
        &self,
    ) -> Result<BTreeMap<VariantId, (wobu_core::Id, DependencyBaseline)>> {
        let mut histories: BTreeMap<VariantId, BTreeMap<wobu_core::Id, DependencyBaseline>> =
            BTreeMap::new();
        for file in self.narrative_records(NarrativeRecordKind::Receipt)? {
            if file.document.payload.get("type").and_then(serde_json::Value::as_str)
                != Some("narrative_dependency_baseline")
            {
                continue;
            }
            let receipt: DependencyReceipt = serde_json::from_value(file.document.payload)?;
            for entry in receipt.entries {
                if entry.dependencies.version != wobu_narrative_deps::DEPENDENCY_VERSION {
                    return Err(invalid(
                        "Unsupported dependency receipt version; do not downgrade canonical evidence.",
                    ));
                }
                if histories
                    .entry(entry.dependencies.variant())
                    .or_default()
                    .insert(file.document.id, entry)
                    .is_some()
                {
                    return Err(invalid("Duplicate variant in a dependency receipt."));
                }
            }
        }
        let mut result = BTreeMap::new();
        for (variant, mut history) in histories {
            let parents: BTreeSet<_> = history.values().filter_map(|entry| entry.parent).collect();
            if parents.iter().any(|id| !history.contains_key(id)) {
                return Err(invalid("A canonical dependency parent receipt is missing."));
            }
            let leaves: Vec<_> =
                history.keys().filter(|id| !parents.contains(id)).copied().collect();
            if leaves.len() != 1 {
                return Err(invalid(
                    "Concurrent or cyclic dependency baselines require reconciliation; no baseline was acknowledged.",
                ));
            }
            // A chain, not wall-clock/ULID ordering, decides what supersedes what.
            let mut cursor = Some(leaves[0]);
            let mut seen = BTreeSet::new();
            while let Some(id) = cursor {
                if !seen.insert(id) {
                    return Err(invalid("Cyclic dependency history."));
                }
                cursor = history[&id].parent;
            }
            if seen.len() != history.len() {
                return Err(invalid("Disconnected dependency history."));
            }
            result.insert(variant, (leaves[0], history.remove(&leaves[0]).unwrap()));
        }
        Ok(result)
    }

    /// Drop only the local projection. Canonical historical receipts are retained.
    pub fn forget_narrative_dependencies(&self) -> Result<()> {
        self.index.forget_narrative_dependencies()
    }

    /// Rebuild the historical index from canonical receipts, never from today's
    /// world. New authoring is recorded separately by `refresh_narrative_dependencies`.
    pub fn rebuild_narrative_dependencies(&self) -> Result<DependencyIndex> {
        self.narrative_dependencies()
    }

    /// Persist the inputs for explicitly authored/accepted wording. The caller's
    /// snapshot is checked immediately before publication, including characters
    /// outside narrative/. Immutable old receipts remain available for inspection.
    pub fn record_narrative_dependency_snapshot(
        &mut self,
        snapshot: &DependencySnapshot,
        variants: &BTreeSet<VariantId>,
    ) -> Result<()> {
        self.ensure_writable()?;
        snapshot.check_current(self)?;
        if variants.is_empty() {
            return Ok(());
        }
        let previous = self.dependency_receipts()?;
        let revisions = snapshot.revisions();
        let entries: Vec<_> = snapshot
            .sets()
            .into_iter()
            .filter(|s| variants.contains(&s.variant()))
            .map(|dependencies| DependencyBaseline {
                parent: previous.get(&dependencies.variant()).map(|(id, _)| *id),
                revision: revisions[&dependencies.variant()].clone(),
                dependencies,
            })
            .collect();
        let documents = dependency_documents(entries)?;
        snapshot.check_current(self)?;
        // Bounded chunks use the existing portable sync cap. Serialize and check
        // every chunk before publishing; do not recapture/rebuild per entry.
        for document in documents {
            let mut file = crate::NarrativeRecordFile { document, stamp: None };
            if !matches!(self.save_narrative_record(&mut file)?, SourceSave::Saved(_)) {
                return Err(invalid(
                    "Dependency receipt publication conflicted; no baseline was acknowledged.",
                ));
            }
        }
        self.rebuild_narrative_dependencies()?;
        Ok(())
    }

    /// Record a completed explicit edit, acceptance or context attestation.
    pub fn record_narrative_dependencies_for(
        &mut self,
        variants: &BTreeSet<VariantId>,
    ) -> Result<()> {
        let snapshot = self.narrative_dependency_snapshot()?;
        self.record_narrative_dependency_snapshot(&snapshot, variants)
    }

    /// Enroll only the wording this local authoring operation actually wrote.
    /// Watchers must never infer a remote line's baseline while its receipt is
    /// still in transit.
    pub fn record_authored_narrative_dependencies_for(
        &mut self,
        variants: &BTreeSet<VariantId>,
    ) -> Result<()> {
        let snapshot = self.narrative_dependency_snapshot()?;
        let previous = self.dependency_receipts()?;
        let new_wording = snapshot
            .revisions()
            .into_iter()
            .filter_map(|(id, revision)| {
                (variants.contains(&id)
                    && previous.get(&id).is_none_or(|(_, r)| r.revision != revision))
                .then_some(id)
            })
            .collect();
        self.record_narrative_dependency_snapshot(&snapshot, &new_wording)
    }

    /// Propagate changes against canonical evidence on saves and reconciliation.
    /// Unknown remote wording remains untracked until its receipt arrives.
    pub fn refresh_narrative_dependencies(&mut self) -> Result<()> {
        if self.is_read_only() {
            return Ok(());
        }
        // Reconciliation must still expose damaged files for repair. Explicit
        // reads report the error; automatic maintenance publishes no baseline
        // from a partial or invalid project.
        if self.narrative_index()?.iter().any(|entry| entry.error.is_some())
            || !self.index.corrupt_paths()?.is_empty()
        {
            return Ok(());
        }
        self.mark_narrative_affected()?;
        Ok(())
    }

    /// Which lines have been affected since the stored index was recorded, and
    /// why.
    ///
    /// A pure report: nothing is written, no content is touched, and calling it
    /// twice gives the same answer.
    pub fn narrative_affected(&self) -> Result<Vec<Affected>> {
        let stored = self.narrative_dependencies()?;
        Ok(stored.diff(&self.narrative_dependency_snapshot()?.index()))
    }

    /// Which lines a change at these reverse-index keys could reach.
    ///
    /// Answered from the stored edges without loading a single dependency set,
    /// which is what makes it usable from a save handler. A superset of the
    /// affected set by construction; see [`DependencyIndex::candidates`].
    pub fn narrative_dependency_candidates(
        &self,
        keys: &BTreeSet<String>,
    ) -> Result<BTreeSet<VariantId>> {
        self.narrative_dependencies()?;
        self.index.narrative_dependency_candidates(keys)
    }

    /// Propagate invalidation without acknowledging or replacing the historical baseline.
    ///
    /// Returns the full affected report, including the lines it did not write
    /// to — an untracked line has nothing to invalidate and an absent line has
    /// no document left to write into, and both are still things a caller has
    /// to be told about.
    ///
    /// Nothing is written for a document whose affected lines are all already
    /// out of date, so running this twice touches no file the second time.
    pub fn mark_narrative_affected(&mut self) -> Result<Vec<Affected>> {
        let snapshot = self.narrative_dependency_snapshot()?;
        let affected = self.narrative_dependencies()?.diff(&snapshot.index());
        snapshot.check_current(self)?;
        let mut scenes: BTreeMap<String, BTreeSet<VariantId>> = BTreeMap::new();
        let mut texts: BTreeMap<String, BTreeSet<VariantId>> = BTreeMap::new();
        for item in &affected {
            // Only a line whose recorded baseline moved. An untracked line has
            // never been measured against anything, so nothing about it can have
            // gone stale, and flagging one would turn a lost cache into a
            // project-wide false alarm. An absent line has no document left.
            if item.kind != wobu_narrative_deps::AffectedKind::Changed {
                continue;
            }
            match &item.target {
                wobu_narrative_deps::TargetRef::SceneLine { scene, variant, .. } => {
                    scenes.entry(scene.to_string()).or_default().insert(*variant);
                }
                wobu_narrative_deps::TargetRef::TextLine { asset, variant, .. } => {
                    texts.entry(asset.to_string()).or_default().insert(*variant);
                }
            }
        }
        for (scene, variants) in scenes {
            if let Ok(id) = scene.parse() {
                let expected =
                    snapshot.scenes.iter().find(|scene| scene.id == id).ok_or_else(|| {
                        invalid("Affected scene is absent from the captured snapshot.")
                    })?;
                self.mark_scene_out_of_date(id, &variants, expected)?;
            }
        }
        for (asset, variants) in texts {
            if let Ok(id) = asset.parse() {
                let expected =
                    snapshot.texts.iter().find(|asset| asset.id == id).ok_or_else(|| {
                        invalid("Affected text is absent from the captured snapshot.")
                    })?;
                self.mark_text_out_of_date(id, &variants, expected)?;
            }
        }
        Ok(affected)
    }

    /// Downstream production artifacts whose recorded line is in an affected
    /// set.
    ///
    /// Production records (#178–#183) are immutable, so invalidation reaches
    /// them as a report and never as a rewrite: the recording of a line that
    /// has gone stale is still the recording that was made, and deleting or
    /// editing it would destroy the evidence of what shipped. A record is
    /// matched by the `variant_id` its payload names, which is the identity
    /// every downstream artifact is keyed on precisely so that this join
    /// exists.
    pub fn narrative_affected_production(
        &self,
        affected: &[Affected],
    ) -> Result<Vec<crate::NarrativeRecordDocument>> {
        let variants: BTreeSet<String> =
            affected.iter().map(|item| item.target.variant().to_string()).collect();
        Ok(self
            .narrative_records(NarrativeRecordKind::Production)?
            .into_iter()
            .map(|file| file.document)
            .filter(|document| {
                document
                    .payload
                    .get("variant_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| variants.contains(id))
            })
            .collect())
    }

    /// Set [`Freshness::OutOfDate`](wobu_narrative::Freshness) on named lines of
    /// one scene, and nothing else.
    ///
    /// A dedicated writer rather than an ordinary
    /// [`save_scene`](Project::save_scene), and the difference is deliberate in
    /// two places.
    ///
    /// **It does not go through `validate_manual`.** That guard refuses any
    /// change to a locked variant's [`Text`](wobu_narrative::Text), and freshness
    /// lives inside `Text`. Routing this through it would make a locked line
    /// permanently un-markable — which is exactly the failure `ContentLifecycle`
    /// was split into three fields to prevent: locked wording that no longer
    /// fits its context is a problem a reviewer has to resolve, not one the lock
    /// makes disappear (US-06). The guard protects *words*; this writes no
    /// words.
    ///
    /// **It records no editorial event.** There is no decision here to record:
    /// nobody chose to make the line stale, an upstream fact moved. The
    /// editorial-history check in
    /// [`review_source`](crate::project::narrative_review) normalises freshness
    /// away for the same reason, so an invalidated scene is still a scene whose
    /// approvals were all recorded — approved, and no longer release-ready,
    /// which is the pair the reviewer needs to see.
    ///
    /// The scene lock and the stamp guard are both kept: this is still a
    /// whole-document write over a share, and a losing write is refused rather
    /// than merged.
    fn mark_scene_out_of_date(
        &mut self,
        id: wobu_narrative::SceneId,
        variants: &BTreeSet<VariantId>,
        expected: &Scene,
    ) -> Result<()> {
        self.ensure_writable()?;
        let _lock = super::narrative_review::scene_lock(self, id)?;
        let mut file: SceneFile = match self.load_scene(id) {
            Ok(file) => file,
            // An affected line whose scene has been deleted has nothing to flag.
            // It is still in the report, which is where it belongs.
            Err(Error::NoSuchNode(_)) => return Ok(()),
            Err(error) => return Err(error),
        };
        if &file.scene != expected {
            return Err(invalid(
                "Scene revision changed before dependency publication. Reload and try again.",
            ));
        }
        let mut touched = false;
        for beat in &mut file.scene.beats {
            for slot in &mut beat.dialogue {
                for variant in &mut slot.variants {
                    if variants.contains(&variant.id) {
                        touched |= wobu_narrative_deps::mark(&mut variant.text.lifecycle);
                    }
                }
            }
        }
        if !touched {
            return Ok(());
        }
        match crate::narrative::write_scene(self.root(), &mut file, &self.peer)? {
            SourceSave::Saved(_) => self.index_narrative_path(&file.rel),
            SourceSave::Conflict { .. } => Err(invalid(
                "The scene changed while marking it out of date. Reload and try again.",
            )),
        }
    }

    /// The supporting-text counterpart, on the same terms (#167).
    fn mark_text_out_of_date(
        &mut self,
        id: wobu_narrative::TextAssetId,
        variants: &BTreeSet<VariantId>,
        expected: &TextAsset,
    ) -> Result<()> {
        self.ensure_writable()?;
        let mut file: TextFile = match self.load_text_asset(id) {
            Ok(file) => file,
            Err(Error::NoSuchNode(_)) => return Ok(()),
            Err(error) => return Err(error),
        };
        if &file.asset != expected {
            return Err(invalid(
                "Text revision changed before dependency publication. Reload and try again.",
            ));
        }
        let mut touched = false;
        for entry in &mut file.asset.entries {
            for slot in &mut entry.lines {
                for variant in &mut slot.variants {
                    if variants.contains(&variant.id) {
                        touched |= wobu_narrative_deps::mark(&mut variant.text.lifecycle);
                    }
                }
            }
        }
        if !touched {
            return Ok(());
        }
        match crate::narrative::write_text(self.root(), &mut file, &self.peer)? {
            SourceSave::Saved(_) => self.index_narrative_path(&file.rel),
            SourceSave::Conflict { .. } => Err(invalid(
                "A supporting text asset changed while marking it out of date. Reload and try again.",
            )),
        }
    }

    /* ── reading the canonical inputs ────────────────────────────────── */

    fn scenes(&self) -> Result<Vec<Scene>> {
        let catalog = self.scene_catalog()?;
        let mut scenes = Vec::new();
        for entry in &catalog.scenes {
            scenes.push(crate::narrative::read_scene(self.root(), &entry.rel)?.scene);
        }
        scenes.sort_by_key(|scene| scene.id);
        Ok(scenes)
    }

    fn read_characters(&self, ids: &BTreeSet<EntityId>) -> Result<CharacterRead> {
        ids.iter()
            .map(|id| match self.get_node_stamped(*id) {
                Ok((node, stamp)) if node.id == *id => {
                    let character = (node.kind == NodeKind::Character).then(|| Character {
                        id: *id,
                        name: node.name.clone(),
                        // A non-text voice attribute is read as no voice rather
                        // than refused: this is a fingerprint input, and a
                        // capture that failed on one malformed attribute would
                        // stop tracking the whole project.
                        voice: node
                            .attributes
                            .get("narrative_voice")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                    });
                    Ok((*id, (character, Some(stamp))))
                }
                Ok(_) => Err(invalid("Character identity changed while reading dependencies.")),
                Err(Error::NoSuchNode(_)) => Ok((*id, (None, None))),
                Err(error) => Err(error),
            })
            .collect()
    }

    /// Who produced each generated line, from the frozen requests on disk.
    ///
    /// Only wording whose provenance actually says a model wrote it gets a
    /// producer. A line somebody generated, rejected and then typed by hand is
    /// a hand-written line, and attaching the provider to it would mark a
    /// writer's words stale the day the project switched models.
    fn narrative_producers(
        &self,
        scenes: &[Scene],
        texts: &[TextAsset],
    ) -> Result<BTreeMap<VariantId, Producer>> {
        let generated: BTreeSet<VariantId> = scenes
            .iter()
            .flat_map(|scene| scene.dialogue_slots().map(|(_, slot)| slot))
            .chain(texts.iter().flat_map(|asset| asset.lines().map(|(_, slot)| slot)))
            .flat_map(|slot| slot.variants.iter())
            .filter(|variant| matches!(variant.text.provenance, Provenance::Generated { .. }))
            .map(|variant| variant.id)
            .collect();
        if generated.is_empty() {
            return Ok(BTreeMap::new());
        }

        let mut producers = BTreeMap::new();
        // Receipts are read in id order, which for ULIDs is creation order, so
        // the most recent request for a slot is the one that survives.
        for file in self.narrative_records(NarrativeRecordKind::Receipt)? {
            if file.document.payload.get("type").and_then(serde_json::Value::as_str)
                != Some("narrative_generation_request")
            {
                continue;
            }
            let Ok(wobu_narrative_generation::Receipt::NarrativeGenerationRequest { request }) =
                serde_json::from_value(file.document.payload.clone())
            else {
                continue;
            };
            if !generated.contains(&request.candidate_variant_id) {
                continue;
            }
            producers.insert(
                request.candidate_variant_id,
                Producer::of(
                    &request.provider,
                    &request.model,
                    &serde_json::to_value(&request.settings)?,
                ),
            );
        }
        Ok(producers)
    }
}

/// Every entity a capture might have to resolve a voice for.
///
/// Deliberately wider than the cast: a knowledge claim's `Told { by }` origin
/// and a supporting asset's character source link both reach a character that
/// is in no scene's participant list, and a capture that did not read them
/// would record their absence and then never notice them arriving.
fn referenced_entities(
    scenes: &[Scene],
    texts: &[TextAsset],
    world: &WorldDocument,
) -> BTreeSet<EntityId> {
    let mut ids = BTreeSet::new();
    for scene in scenes {
        ids.extend(scene.participants.iter().map(|p| p.entity));
        ids.extend(scene.dialogue_slots().filter_map(|(_, slot)| slot.speaker.entity()));
    }
    for asset in texts {
        ids.extend(asset.participants.iter().map(|p| p.entity));
        ids.extend(asset.speakers());
        ids.extend(asset.sources.iter().filter_map(|link| match link {
            wobu_narrative::SourceLink::Character(id) => Some(*id),
            _ => None,
        }));
    }
    ids.extend(world.knowledge.iter().filter_map(|claim| match claim.provenance {
        wobu_narrative::KnowledgeProvenance::Told { by } => Some(by),
        _ => None,
    }));
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_world_edit_between_capture_and_validation_refuses_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = Project::create(dir.path(), "Capture race").unwrap();
        project.save_world(&WorldDocument::default(), None).unwrap();
        let result = project.read_dependency_snapshot(|| {
            let mut changed = WorldDocument::default();
            changed.facts.push(wobu_narrative::Fact {
                id: wobu_core::new_id(),
                name: "Arrived during capture".into(),
                assertion: "This world did not exist when reading began.".into(),
                sources: vec![],
                entity_ids: vec![],
            });
            let path = project.root().join("narrative/world.yaml");
            std::fs::write(&path, changed.to_yaml().unwrap()).map_err(|e| Error::io(path, e))
        });
        assert!(
            matches!(result, Err(Error::Malformed { reason, .. }) if reason.contains("changed while reading dependencies"))
        );
    }
}
