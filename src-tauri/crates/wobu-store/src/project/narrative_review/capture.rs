use super::*;
use crate::{NarrativeRecordKind, atomic};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use wobu_narrative::review::{ApprovalEvidence, EditorialEvent, ReviewBinding};
use wobu_narrative::{Name, StateDocument, Value, VarType, VariantId, WorldDocument};

pub struct ReviewSnapshot {
    pub(crate) file: SceneFile,
    pub(crate) world: WorldDocument,
    pub(crate) schema: StateDocument,
    pub(crate) characters: Json,
    pub(crate) linked_scenes: Vec<wobu_narrative::Scene>,
    pub(crate) state: BTreeMap<Name, Value>,
    pub(crate) history: Vec<EditorialEvent>,
    pub(crate) history_problem: Option<String>,
    pub(crate) input_problem: Option<String>,
    pub(crate) fingerprint: String,
    pub(crate) observations: BTreeMap<String, Option<Stamp>>,
    character_stamps: BTreeMap<wobu_narrative::EntityId, Option<Stamp>>,
}
impl Project {
    pub fn review_snapshot(&self, id: SceneId, state_json: Option<&str>) -> Result<ReviewSnapshot> {
        self.review_source(self.load_editorial_source(id)?, state_json)
    }
    pub(crate) fn review_source(
        &self,
        file: SceneFile,
        state_json: Option<&str>,
    ) -> Result<ReviewSnapshot> {
        let fingerprint = self.narrative_fingerprint()?;
        self.review_source_captured(file, state_json, fingerprint, true, &self.scene_ids()?)
    }
    /// Batch consumers freeze membership once, while retaining every document/character stamp.
    pub fn review_snapshots(
        &self,
        ids: &[SceneId],
        state_json: Option<&str>,
    ) -> Result<Vec<ReviewSnapshot>> {
        let fingerprint = self.narrative_fingerprint()?;
        let scenes = self.scene_catalog()?;
        let texts = self.text_catalog()?;
        let known_scenes = scenes.ids();
        let mut paths = BTreeMap::new();
        for entry in &scenes.scenes {
            if paths.insert(entry.id, (&entry.rel, false)).is_some() {
                return Err(invalid("Duplicate scene identity."));
            }
        }
        for entry in &texts.assets {
            if paths.insert(SceneId::from_raw(entry.id.raw()), (&entry.rel, true)).is_some() {
                return Err(invalid("Scene and text identity collision."));
            }
        }
        let snapshots = ids
            .iter()
            .map(|id| {
                let (rel, text) =
                    paths.get(id).ok_or_else(|| invalid("Missing editorial source."))?;
                let file = if *text {
                    let source = crate::narrative::read_text(self.root(), rel)?;
                    SceneFile {
                        scene: source.asset.editorial_scene(),
                        rel: source.rel,
                        stamp: source.stamp,
                    }
                } else {
                    crate::narrative::read_scene(self.root(), rel)?
                };
                if file.scene.id != *id {
                    return Err(invalid("Source identity changed during batch capture."));
                }
                self.review_source_captured(
                    file,
                    state_json,
                    fingerprint.clone(),
                    false,
                    &known_scenes,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        for snapshot in &snapshots {
            snapshot.check_observations(self)?;
        }
        if fingerprint != self.narrative_fingerprint()? {
            return Err(invalid("Narrative changed during batch review capture."));
        }
        Ok(snapshots)
    }
    fn review_source_captured(
        &self,
        file: SceneFile,
        state_json: Option<&str>,
        fingerprint: String,
        verify: bool,
        known_scenes: &BTreeSet<SceneId>,
    ) -> Result<ReviewSnapshot> {
        let world_file = self.world_document()?;
        let world = world_file.as_ref().map(|(w, _)| w.clone()).unwrap_or_default();
        let schema_file = self.state_document()?;
        let schema = schema_file
            .as_ref()
            .map(|(s, _)| s.clone())
            .unwrap_or_else(|| StateDocument::new(vec![]));
        let checked = schema.schema().map_err(|e| invalid(e.to_string()))?;
        let state = match state_json {
            Some(raw) => serde_json::from_str::<BTreeMap<Name, Value>>(raw)
                .map_err(|e| invalid(e.to_string()))?,
            None => checked.iter().map(|v| (v.name.clone(), v.default.clone())).collect(),
        };
        if state.len() != checked.len()
            || checked.iter().any(|d| {
                !state.get(&d.name).is_some_and(|v| match (&d.ty, v) {
                    (VarType::Bool, Value::Bool(_)) => true,
                    (VarType::Int { min, max }, Value::Int(v)) => (*min..=*max).contains(v),
                    (VarType::Enum { members }, Value::Enum(v)) => members.contains(v),
                    _ => false,
                })
            })
        {
            return Err(invalid(
                "Review state must contain every declared variable with a valid typed value.",
            ));
        }
        let mut observations = BTreeMap::from([
            (file.rel.clone(), file.stamp.clone()),
            ("narrative/world.yaml".into(), world_file.map(|(_, stamp)| stamp)),
            ("narrative/state.yaml".into(), schema_file.map(|(_, stamp)| stamp)),
        ]);
        let mut linked_scenes = Vec::new();
        for link in file.scene.supporting_text.iter().flat_map(|asset| &asset.sources) {
            let wobu_narrative::SourceLink::Scene(id) = link else { continue };
            match self.load_scene(*id) {
                Ok(linked) => {
                    observations.insert(linked.rel, linked.stamp);
                    // Only authored name/summary enter the supporting prompt.
                    // Freeze those inputs, not unrelated scene wording or layout.
                    let mut scene = wobu_narrative::Scene::new(&linked.scene.name);
                    scene.id = linked.scene.id;
                    scene.summary = linked.scene.summary;
                    linked_scenes.push(scene);
                }
                Err(Error::NoSuchNode(_)) => {}
                Err(error) => return Err(error),
            }
        }
        linked_scenes.sort_by_key(|scene| scene.id);
        linked_scenes.dedup_by_key(|scene| scene.id);
        let mut ids = file
            .scene
            .participants
            .iter()
            .map(|p| p.entity)
            .chain(file.scene.dialogue_slots().filter_map(|(_, s)| s.speaker.entity()))
            .collect::<BTreeSet<_>>();
        ids.extend(file.scene.supporting_text.iter().flat_map(|asset| {
            asset.sources.iter().filter_map(|link| match link {
                wobu_narrative::SourceLink::Character(id) => Some(*id),
                _ => None,
            })
        }));
        ids.extend(world.knowledge.iter().map(|k| k.character));
        ids.extend(world.knowledge.iter().filter_map(|k| match k.provenance {
            wobu_narrative::KnowledgeProvenance::Told { by } => Some(by),
            _ => None,
        }));
        ids.extend(world.relationships.iter().flat_map(|r| [r.from, r.to]));
        ids.extend(world.restrictions.iter().flat_map(|r| r.characters.iter().copied()));
        ids.extend(world.facts.iter().flat_map(|r| r.entity_ids.iter().copied()));
        ids.extend(world.events.iter().flat_map(|r| r.entity_ids.iter().copied()));
        let mut characters = BTreeMap::new();
        let mut character_stamps = BTreeMap::new();
        for id in ids {
            match self.get_node_stamped(id) {
                Ok((node, stamp)) => {
                    let rel = self
                        .index
                        .rel_path_of(id)?
                        .ok_or_else(|| invalid("Character path is missing."))?;
                    observations.insert(rel, Some(stamp.clone()));
                    character_stamps.insert(id, Some(stamp));
                    characters.insert(id,if node.kind==wobu_core::NodeKind::Character {json!({"id":id,"name":node.name,"voice":node.attributes.get("narrative_voice")})} else {Json::Null});
                }
                Err(Error::NoSuchNode(_)) => {
                    characters.insert(id, Json::Null);
                    character_stamps.insert(id, None);
                }
                Err(e) => return Err(e),
            }
        }
        let known_characters = characters
            .iter()
            .filter_map(|(id, c)| (!c.is_null()).then_some(*id))
            .collect::<BTreeSet<_>>();
        let known_entities =
            character_stamps.iter().filter_map(|(id, s)| s.is_some().then_some(*id)).collect();
        let mut input_problem = world
            .diagnose(&checked, &known_characters, &known_entities, known_scenes)
            .first()
            .map(|d| format!("{}: {}", d.field, d.message));
        if let Some(asset) = file.scene.editorial_text() {
            let scenes = known_scenes;
            if asset.sources.iter().any(|link| match link {
                wobu_narrative::SourceLink::Scene(id) => !scenes.contains(id),
                wobu_narrative::SourceLink::Fact(id) => {
                    !world.facts.iter().any(|record| record.id == *id)
                }
                wobu_narrative::SourceLink::Event(id) => {
                    !world.events.iter().any(|record| record.id == *id)
                }
                wobu_narrative::SourceLink::Quest(id) => {
                    !world.quests.iter().any(|record| record.id == *id)
                }
                wobu_narrative::SourceLink::Character(id) => !known_characters.contains(id),
            }) {
                input_problem = Some(
                    "A supporting-text source link is missing. Repair it before approving.".into(),
                );
            }
            if let Some(diagnostic) = asset.diagnostics(&checked).first() {
                input_problem = Some(diagnostic.problem.to_string());
            }
        }
        if let Some((diagnostic, _)) = file.scene.classification_diagnostics(&world).first() {
            input_problem = Some(diagnostic.to_string());
        }
        if file.scene.participants.iter().any(|p| !known_characters.contains(&p.entity))
            || file
                .scene
                .dialogue_slots()
                .any(|(_, s)| s.speaker.entity().is_some_and(|id| !known_characters.contains(&id)))
        {
            input_problem=Some("Review requires every participant and entity speaker to reference an existing character.".into());
        }
        if characters
            .values()
            .any(|c| !c.is_null() && !c["voice"].is_null() && !c["voice"].is_string())
        {
            input_problem = Some("Character narrative_voice must be text.".into());
        }
        for condition in file.scene.entry.iter().chain(
            file.scene
                .dialogue_slots()
                .flat_map(|(_, s)| s.variants.iter().filter_map(|v| v.when.as_ref())),
        ) {
            if let Err(e) = checked.check_condition(condition) {
                input_problem = Some(e.to_string());
            }
        }
        let mut history = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cursor = file.scene.editorial_head;
        let mut history_problem = None;
        while let Some(id) = cursor {
            if !seen.insert(id) || seen.len() > 100_000 {
                history_problem =
                    Some("Editorial history is cyclic or exceeds the supported limit.".into());
                break;
            }
            let record = match self.narrative_record(NarrativeRecordKind::Receipt, id) {
                Ok(Some(record)) => record,
                _ => {
                    history_problem = Some(
                        "An editorial receipt is missing or invalid. Approval cannot be verified."
                            .into(),
                    );
                    break;
                }
            };
            observations.insert(record.document.rel(), record.stamp);
            let event = match serde_json::from_value::<EditorialEvent>(record.document.payload) {
                Ok(event) if event.id == id && event.after.id == file.scene.id && event.valid() => {
                    event
                }
                _ => {
                    history_problem = Some(
                        "Editorial receipt identity, version or context binding is invalid.".into(),
                    );
                    break;
                }
            };
            cursor = event.parent;
            history.push(event);
        }
        for pair in history.windows(2) {
            let mut parent = pair[1].after.clone();
            parent.editorial_head = Some(pair[1].id);
            if super::write::without_freshness(&pair[0].before)
                != super::write::without_freshness(&parent)
            {
                history_problem = Some(
                    "Editorial history has a discontinuity. Approval cannot be verified.".into(),
                );
            }
        }
        if let Some(head) = history.first() {
            let mut current = file.scene.clone();
            current.editorial_head = None;
            if super::write::without_freshness(&head.after)
                != super::write::without_freshness(&current)
            {
                history_problem=Some("Scene changed outside its recorded editorial history. Save the manual change before reviewing.".into());
            }
            let by_id = history.iter().map(|event| (event.id, event)).collect::<BTreeMap<_, _>>();
            for binding in head.bindings.values() {
                let verified = by_id.get(&binding.event_id).is_some_and(|event| {
                    let Some(context) = &event.context else { return false };
                    let Some(id) = binding.target.variant else { return false };
                    let text = event
                        .after
                        .beat(binding.target.beat)
                        .and_then(|b| b.dialogue.iter().find(|s| s.id == binding.target.slot))
                        .and_then(|s| s.variants.iter().find(|v| v.id == id).map(|v| (s, v)));
                    event.bindings.get(&id) == Some(binding)
                        && origin_matches(event, binding)
                        && (!binding.approved
                            || matches!(
                                event.action,
                                wobu_narrative::review::EditorialAction::Approve
                                    | wobu_narrative::review::EditorialAction::Attest
                            ))
                        && text.is_some_and(|(s, v)| {
                            binding.matches(&binding.target, &s.speaker, &v.text)
                        })
                        && context.valid()
                        && context.state == binding.state
                        && binding.context_revision
                            == historical_context(
                                context,
                                &event.after,
                                &binding.target,
                                binding.state.clone(),
                            )
                            .revision
                });
                if !verified {
                    history_problem=Some("Approval context, target or wording does not match its immutable decision.".into());
                }
            }
        }
        let snapshot = ReviewSnapshot {
            file,
            world,
            schema,
            characters: serde_json::to_value(characters)?,
            linked_scenes,
            state,
            history,
            history_problem,
            input_problem,
            fingerprint,
            observations,
            character_stamps,
        };
        if verify {
            snapshot.check_current(self)?;
        }
        Ok(snapshot)
    }
}
impl ReviewSnapshot {
    pub fn scene(&self) -> &wobu_narrative::Scene {
        &self.file.scene
    }
    pub fn verify_current(&self, project: &Project) -> Result<()> {
        self.check_current(project)
    }
    pub fn guard(&self) -> ReviewGuard {
        ReviewGuard { stamp: self.file.stamp.clone(), head: self.file.scene.editorial_head }
    }
    pub(crate) fn check_current(&self, project: &Project) -> Result<()> {
        if project.narrative_fingerprint()? != self.fingerprint {
            return Err(invalid(
                "Narrative source changed during review capture. Reload before deciding.",
            ));
        }
        self.check_observations(project)
    }
    pub(crate) fn check_observations(&self, project: &Project) -> Result<()> {
        for (rel, stamp) in &self.observations {
            if atomic::read_stamped(&project.root().join(rel))?.map(|(_, s)| s) != *stamp {
                return Err(invalid(
                    "Review inputs changed during capture. Reload before deciding.",
                ));
            }
        }
        for (id, stamp) in &self.character_stamps {
            let current = match project.get_node_stamped(*id) {
                Ok((node, stamp)) if node.id == *id => Some(stamp),
                Err(Error::NoSuchNode(_)) => None,
                Ok(_) => return Err(invalid("Character identity changed during capture.")),
                Err(e) => return Err(e),
            };
            if &current != stamp {
                return Err(invalid(
                    "Character membership or wording changed during capture. Reload before deciding.",
                ));
            }
        }
        Ok(())
    }
    pub fn context(&self, target: &ReviewTarget) -> Result<ReviewContext> {
        let slot = self
            .file
            .scene
            .beat(target.beat)
            .and_then(|b| b.dialogue.iter().find(|s| s.id == target.slot))
            .filter(|_| target.scene == self.file.scene.id)
            .ok_or_else(|| invalid("Review target is missing."))?;
        if target.variant.is_some_and(|id| !slot.variants.iter().any(|v| v.id == id)) {
            return Err(invalid("Review variant is missing."));
        }
        Ok(capture_context(
            &self.file.scene,
            target,
            serde_json::to_value(&self.world)?,
            serde_json::to_value(&self.schema)?,
            self.characters.clone(),
            self.state.clone(),
            &self.linked_scenes,
        ))
    }
    pub(crate) fn bindings(&self) -> BTreeMap<VariantId, ReviewBinding> {
        if self.history_problem.is_some() || self.input_problem.is_some() {
            return BTreeMap::new();
        }
        self.history.first().map(|e| e.bindings.clone()).unwrap_or_default()
    }
    pub fn evidence(&self) -> Result<BTreeMap<VariantId, ApprovalEvidence>> {
        let mut proofs = BTreeMap::new();
        for (id, binding) in self.bindings() {
            // An approval retains its reviewed scenario, not whichever preview is selected now.
            let context = capture_context(
                &self.file.scene,
                &binding.target,
                serde_json::to_value(&self.world)?,
                serde_json::to_value(&self.schema)?,
                self.characters.clone(),
                binding.state.clone(),
                &self.linked_scenes,
            );
            proofs.insert(id, ApprovalEvidence { binding, current_context: context.revision });
        }
        Ok(proofs)
    }
    /// Convert only verified immutable shared decisions to native text proofs.
    pub fn text_evidence(
        &self,
    ) -> Result<BTreeMap<VariantId, wobu_narrative::review::TextApprovalEvidence>> {
        use wobu_narrative::review::{TextApprovalEvidence, TextBinding, TextTarget};
        let Some(asset) = self.file.scene.editorial_text() else { return Ok(BTreeMap::new()) };
        Ok(self
            .evidence()?
            .into_iter()
            .map(|(id, proof)| {
                let binding = proof.binding;
                (
                    id,
                    TextApprovalEvidence {
                        binding: TextBinding {
                            target: TextTarget {
                                asset: asset.id,
                                entry: wobu_narrative::TextEntryId::from_raw(
                                    binding.target.beat.raw(),
                                ),
                                slot: binding.target.slot,
                                variant: binding.target.variant,
                            },
                            speaker: binding.speaker,
                            text_revision: binding.text_revision,
                            context_revision: binding.context_revision,
                            state: binding.state,
                            approved: binding.approved,
                            event_id: binding.event_id,
                        },
                        current_context: proof.current_context,
                    },
                )
            })
            .collect())
    }
    pub fn view(&self, project: &Project) -> Result<ReviewSceneView> {
        self.view_with_proposals(
            project.review_proposals()?.remove(&self.file.scene.id).unwrap_or_default(),
        )
    }
    pub fn view_with_proposals(
        &self,
        mut proposals: Vec<ReviewProposal>,
    ) -> Result<ReviewSceneView> {
        let bindings = self.bindings();
        let decisions = self.history.first().map(|e| &e.decisions);
        for proposal in &mut proposals {
            proposal.status = match decisions.and_then(|d| d.get(&proposal.id)) {
                Some(wobu_narrative::review::ProposalDecision::Accepted) => "accepted",
                Some(wobu_narrative::review::ProposalDecision::Rejected) => "rejected",
                None => "pending",
            };
            if proposal.status != "pending" {
                proposal.reason =
                    format!("Proposal {} in canonical editorial history.", proposal.status);
            }
        }
        let mut lines = Vec::new();
        for (beat, slot) in self.file.scene.dialogue_slots() {
            let variants = if slot.variants.is_empty() {
                vec![None]
            } else {
                slot.variants.iter().map(Some).collect()
            };
            for variant in variants {
                let target = ReviewTarget {
                    scene: self.file.scene.id,
                    beat,
                    slot: slot.id,
                    variant: variant.map(|v| v.id),
                };
                let context = self.context(&target)?;
                let binding = variant.and_then(|v| bindings.get(&v.id)).filter(|b| {
                    variant.is_some_and(|v| b.matches(&target, &slot.speaker, &v.text))
                });
                let current = binding.is_some_and(|b| b.context_revision == context.revision);
                let approved = current && binding.is_some_and(|b| b.approved);
                let freshness = if current { Freshness::Current } else { Freshness::OutOfDate };
                let reason =
                    self.history_problem.clone().or(self.input_problem.clone()).unwrap_or_else(
                        || {
                            if binding.is_none() {
                                "This wording has no verified reviewed context.".into()
                            } else if !current {
                                "Authored context changed since the last review.".into()
                            } else {
                                "Wording and reviewed context match.".into()
                            }
                        },
                    );
                lines.push(ReviewLine {
                    target: target.clone(),
                    speaker: slot.speaker.clone(),
                    text: variant.map(|v| v.text.clone()),
                    slot_policy: if self
                        .file
                        .scene
                        .supporting_text
                        .as_ref()
                        .is_some_and(|a| a.policy == GenerationPolicy::Locked)
                    {
                        GenerationPolicy::Locked
                    } else {
                        slot.policy
                    },
                    review: if approved { ReviewState::Approved } else { ReviewState::Draft },
                    freshness,
                    approval_valid: approved,
                    reason,
                    context_revision: context.revision,
                    proposals: proposals
                        .iter()
                        .filter(|p| {
                            p.candidate.slot_id == slot.id
                                && target.variant.is_none_or(|id| p.candidate.variant_id == id)
                        })
                        .cloned()
                        .collect(),
                });
            }
        }
        Ok(ReviewSceneView {scene_id:self.file.scene.id,guard:self.guard(),state_json:serde_json::to_string(&self.state)?,lines,
            history:self.history.iter().map(|e|ReviewHistory {id:e.id,action:serde_json::to_value(&e.action).ok().and_then(|a|a["kind"].as_str().map(str::to_owned)).unwrap_or_default(),target:e.target.clone(),actor:e.actor.clone(),context_revision:e.context.as_ref().map(|c|c.revision.clone())}).collect(),
            context_summary:"Review binds authored scene structure, world records, character voices, variable declarations and the displayed scenario. Editorial flags and the selected wording are tracked separately.".into()})
    }
}

/// Manual saves capture several draft baselines; every other binding originates
/// in one explicit target decision. Empty-slot generation may introduce its id.
fn origin_matches(event: &EditorialEvent, binding: &ReviewBinding) -> bool {
    use wobu_narrative::review::EditorialAction;
    match &event.action {
        EditorialAction::ManualSave => event.target.is_none() && !binding.approved,
        EditorialAction::Approve | EditorialAction::Attest | EditorialAction::Edit { .. } => {
            event.target.as_ref() == Some(&binding.target)
        }
        EditorialAction::Accept { .. } | EditorialAction::Generated { .. } => {
            let Some(target) = &event.target else { return false };
            let mut assigned = target.clone();
            assigned.variant = binding.target.variant;
            if target.variant.is_some() {
                target == &binding.target
            } else {
                assigned == binding.target
                    && event
                        .before
                        .beat(target.beat)
                        .and_then(|b| b.dialogue.iter().find(|s| s.id == target.slot))
                        .is_some_and(|s| s.variants.is_empty())
                    && event
                        .after
                        .beat(target.beat)
                        .and_then(|b| b.dialogue.iter().find(|s| s.id == target.slot))
                        .is_some_and(|s| s.variants.len() == 1)
            }
        }
        _ => false,
    }
}
