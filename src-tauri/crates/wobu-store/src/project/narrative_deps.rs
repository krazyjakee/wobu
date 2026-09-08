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
    EntityId, Provenance, Scene, StateSchema, TextAsset, VariantId, WorldDocument,
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
}

impl DependencySnapshot {
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
    /// Rebuilding from canonical data after cache loss is exactly this call
    /// followed by [`DependencySnapshot::index`]; there is no second, cheaper
    /// path that could disagree with it.
    pub fn narrative_dependency_snapshot(&self) -> Result<DependencySnapshot> {
        let fingerprint = self.narrative_fingerprint()?;
        let scenes = self.scenes()?;
        let texts = self.text_assets()?;
        let world = self.world_document()?.map(|(document, _)| document).unwrap_or_default();
        let schema = self.state_schema()?;
        let ids = referenced_entities(&scenes, &texts, &world);
        let observed = self.read_characters(&ids)?;
        let producers = self.narrative_producers(&scenes, &texts)?;

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
            characters: observed
                .into_iter()
                .filter_map(|(id, (character, _))| character.map(|value| (id, value)))
                .collect(),
            producers,
            versions: self.narrative_versions(),
            fingerprint,
        })
    }

    /// What the local index remembers each line was written against.
    ///
    /// An empty answer is not "everything is current" — it is "nothing has been
    /// recorded", which [`DependencyIndex::diff`] reports as every line being
    /// untracked rather than as every line being stale.
    pub fn narrative_dependencies(&self) -> Result<DependencyIndex> {
        self.index.narrative_dependencies()
    }

    /// Forget what every line was written against.
    ///
    /// Exactly what deleting the local database would cost, spelled as an
    /// operation so that the recovery path is exercised rather than assumed.
    /// Afterwards every line reports as
    /// [`Untracked`](wobu_narrative_deps::AffectedKind::Untracked) — which is
    /// the correct answer and not a soft one: nothing is known about what these
    /// lines were written against, so nothing can be claimed to have gone stale,
    /// and the freshness flags already in the documents are left exactly as they
    /// are.
    pub fn forget_narrative_dependencies(&self) -> Result<()> {
        self.index.forget_narrative_dependencies()
    }

    /// Capture from canonical data and replace the stored index with it.
    ///
    /// The recovery path from cache loss, and also the way a caller says "I have
    /// acted on the current affected set; this is the new baseline". It writes
    /// no project file.
    pub fn rebuild_narrative_dependencies(&self) -> Result<DependencyIndex> {
        let index = self.narrative_dependency_snapshot()?.index();
        self.index.replace_narrative_dependencies(&index)?;
        Ok(index)
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
        self.index.narrative_dependency_candidates(keys)
    }

    /// Propagate invalidation into authored text, and record the new baseline.
    ///
    /// Returns the full affected report, including the lines it did not write
    /// to — an untracked line has nothing to invalidate and an absent line has
    /// no document left to write into, and both are still things a caller has
    /// to be told about.
    ///
    /// Nothing is written for a document whose affected lines are all already
    /// out of date, so running this twice touches no file the second time.
    pub fn mark_narrative_affected(&mut self) -> Result<Vec<Affected>> {
        let affected = self.narrative_affected()?;
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
                self.mark_scene_out_of_date(id, &variants)?;
            }
        }
        for (asset, variants) in texts {
            if let Ok(id) = asset.parse() {
                self.mark_text_out_of_date(id, &variants)?;
            }
        }
        self.rebuild_narrative_dependencies()?;
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
    ) -> Result<()> {
        self.ensure_writable()?;
        let mut file: TextFile = match self.load_text_asset(id) {
            Ok(file) => file,
            Err(Error::NoSuchNode(_)) => return Ok(()),
            Err(error) => return Err(error),
        };
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
        match self.save_text_asset(&mut file)? {
            SourceSave::Saved(_) => Ok(()),
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
