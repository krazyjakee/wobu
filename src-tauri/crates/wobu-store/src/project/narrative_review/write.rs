use super::*;
use crate::{
    NarrativeRecordDocument, NarrativeRecordFile, NarrativeRecordKind, SourceSave, narrative,
};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
};
use wobu_narrative::review::{
    EditorialEvent, PolicyScope, ProposalDecision, REVIEW_VERSION, ReviewBinding,
};
use wobu_narrative::{Provenance, Scene, Variant, VariantId};

/// The lock is local coordination, never canonical content. Dropping it releases
/// the OS lock, including on a process crash; acquisition never blocks the UI.
pub(crate) fn scene_lock(project: &Project, id: SceneId) -> Result<File> {
    let local = project.root().join(".wobu");
    if std::fs::symlink_metadata(&local).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(invalid("Project metadata cannot be a symbolic link for editorial writes."));
    }
    let dir = local.join("locks");
    if std::fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(invalid("Editorial lock directory cannot be a symbolic link."));
    }
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let path = dir.join(format!("narrative-{id}.lock"));
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(invalid("Editorial lock cannot be a symbolic link."));
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| Error::io(&path, e))?;
    file.try_lock()
        .map_err(|_| invalid("Another writer is updating this scene. Reload and try again."))?;
    Ok(file)
}
pub struct ReviewTransaction {
    snapshot: ReviewSnapshot,
    scene: Scene,
    bindings: BTreeMap<VariantId, ReviewBinding>,
    decisions: BTreeMap<Id, ProposalDecision>,
    events: Vec<EditorialEvent>,
    _lock: File,
}
impl Project {
    /// Guarded document undo. Only review mirrors and the generated history head
    /// are ignored in expected-document comparison; words, identities, policies
    /// and every authored structural field must still match.
    pub fn restore_scene(
        &mut self,
        mut scene: Scene,
        expected: Option<&Scene>,
        slug: &str,
    ) -> Result<SceneFile> {
        self.ensure_writable()?;
        let _lock = scene_lock(self, scene.id)?;
        let current = match self.load_scene(scene.id) {
            Ok(f) => Some(f),
            Err(Error::NoSuchNode(_)) => None,
            Err(e) => return Err(e),
        };
        if current.as_ref().map(|f| undo_content(&f.scene)) != expected.map(undo_content) {
            return Err(invalid(
                "Scene changed since this undo entry. Your collaborator's changes were preserved.",
            ));
        }
        scene.editorial_head = current.as_ref().and_then(|f| f.scene.editorial_head);
        for beat in &mut scene.beats {
            for slot in &mut beat.dialogue {
                for variant in &mut slot.variants {
                    let old = current
                        .as_ref()
                        .into_iter()
                        .flat_map(|f| f.scene.dialogue_slots())
                        .find(|(_, s)| s.id == slot.id)
                        .and_then(|(_, s)| s.variants.iter().find(|v| v.id == variant.id));
                    variant.text.lifecycle.review = old
                        .filter(|v| {
                            v.text.body == variant.text.body
                                && v.text.provenance == variant.text.provenance
                        })
                        .map(|v| v.text.lifecycle.review)
                        .unwrap_or(ReviewState::Draft);
                    if old.is_none() && variant.text.lifecycle.policy == GenerationPolicy::Generated
                    {
                        variant.text.lifecycle.policy = GenerationPolicy::Edited;
                    }
                }
            }
        }
        let mut file = SceneFile {
            scene,
            rel: current.as_ref().map(|f| f.rel.clone()).unwrap_or_else(|| {
                narrative::scene_rel(&wobu_core::unique_slug(slug, &|candidate| {
                    self.scene_catalog().is_ok_and(|c| c.slugs().contains(candidate))
                }))
            }),
            stamp: current.and_then(|f| f.stamp),
        };
        match self.save_editorial_scene_locked(&mut file)? {
            SourceSave::Saved(_) => Ok(file),
            SourceSave::Conflict { .. } => {
                Err(invalid("Scene changed during undo. Reload before restoring."))
            }
        }
    }
    /// A protected, changed or already decided target retains its proposal.
    pub fn apply_generated_proposal(&mut self, id: Id) -> Result<bool> {
        let proposal = super::proposals::checked(self, id)?;
        if proposal.request.expected_slot_policy != GenerationPolicy::Generated
            || proposal.request.expected_policy.is_some_and(|p| p != GenerationPolicy::Generated)
        {
            return Ok(false);
        }
        let target = ReviewTarget {
            scene: proposal.request.target.scene,
            beat: proposal.request.target.beat,
            slot: proposal.request.target.slot,
            variant: proposal.request.target.variant,
        };
        let state_json = serde_json::to_string(&proposal.request.context.options.state)?;
        let snapshot = self.review_snapshot(target.scene, Some(&state_json))?;
        if snapshot.history.first().is_some_and(|e| e.decisions.contains_key(&id)) {
            return Ok(false);
        }
        let context = snapshot.context(&target)?;
        let request = ReviewRequest {
            guard: snapshot.guard(),
            target: target.clone(),
            context_revision: context.revision,
            state_json,
            action: EditorialAction::Generated { proposal_id: id, proposal_hash: proposal.hash },
        };
        let mut tx = self.begin_review(&request)?;
        self.stage_action(&mut tx, target, request.action)?;
        self.commit_review(tx)?;
        Ok(true)
    }
    pub fn begin_review(&self, request: &ReviewRequest) -> Result<ReviewTransaction> {
        self.ensure_writable()?;
        let lock = scene_lock(self, request.target.scene)?;
        let snapshot = self.review_snapshot(request.target.scene, Some(&request.state_json))?;
        if snapshot.guard() != request.guard {
            return Err(invalid("Scene or editorial history changed. Reload before deciding."));
        }
        if let Some(reason) = &snapshot.history_problem {
            return Err(invalid(reason.clone()));
        }
        let scene = snapshot.file.scene.clone();
        let bindings = snapshot.bindings();
        let decisions = snapshot.history.first().map(|e| e.decisions.clone()).unwrap_or_default();
        Ok(ReviewTransaction { snapshot, scene, bindings, decisions, events: vec![], _lock: lock })
    }
    /// All requests in a grouped decision use the original snapshot guard. A
    /// caller cannot silently refresh a conflicting guard midway through a batch.
    pub fn stage_review(&self, tx: &mut ReviewTransaction, request: &ReviewRequest) -> Result<()> {
        if request.guard != tx.snapshot.guard()
            || serde_json::from_str::<BTreeMap<wobu_narrative::Name, wobu_narrative::Value>>(
                &request.state_json,
            )? != tx.snapshot.state
        {
            return Err(invalid("Batch items must use the original scene and scenario snapshot."));
        }
        let context = tx.snapshot.context(&request.target)?;
        if context.revision != request.context_revision {
            return Err(invalid("Reviewed context changed. Inspect it again before deciding."));
        }
        if matches!(
            request.action,
            EditorialAction::ManualSave
                | EditorialAction::Restore
                | EditorialAction::Generated { .. }
        ) {
            return Err(invalid("This action is reserved for guarded storage operations."));
        }
        self.stage_action(tx, request.target.clone(), request.action.clone())
    }
    fn stage_action(
        &self,
        tx: &mut ReviewTransaction,
        target: ReviewTarget,
        action: EditorialAction,
    ) -> Result<()> {
        // Work on a clone so a rejected item cannot partly mutate a batch.
        let mut scene = tx.scene.clone();
        let mut bindings = tx.bindings.clone();
        let mut decisions = tx.decisions.clone();
        let before = scene.clone();
        let event_id = Id::generate();
        let slot = scene
            .beat_mut(target.beat)
            .and_then(|b| b.dialogue.iter_mut().find(|s| s.id == target.slot))
            .filter(|_| target.scene == before.id)
            .ok_or_else(|| invalid("Dialogue target is missing."))?;
        let index = target.variant.and_then(|id| slot.variants.iter().position(|v| v.id == id));
        if target.variant.is_some() && index.is_none() {
            return Err(invalid("Dialogue variant is missing."));
        }
        let locked = slot.policy == GenerationPolicy::Locked
            || index.is_some_and(|i| {
                slot.variants[i].text.lifecycle.policy == GenerationPolicy::Locked
            });
        let mut bind = false;
        let mut approved = false;
        match &action {
            EditorialAction::Policy { scope, policy } => match scope {
                PolicyScope::Slot => slot.policy = *policy,
                PolicyScope::Variant => {
                    let i = index
                        .ok_or_else(|| invalid("Choose a wording before changing its policy."))?;
                    slot.variants[i].text.lifecycle.policy = *policy;
                }
            },
            EditorialAction::Approve | EditorialAction::Attest => {
                if let Some(reason) = &tx.snapshot.input_problem {
                    return Err(invalid(reason.clone()));
                }
                let i = index.ok_or_else(|| invalid("Write a line before reviewing it."))?;
                let text = &mut slot.variants[i].text;
                if text.body.trim().is_empty() || !text.revision_matches() {
                    return Err(invalid(
                        "Review requires nonempty wording with a matching revision. Save a manual edit first.",
                    ));
                }
                if matches!(action, EditorialAction::Attest) {
                    let prior=bindings.get(&slot.variants[i].id).filter(|b|b.matches(&target,&slot.speaker,&slot.variants[i].text)).ok_or_else(||invalid("Attestation requires unchanged previously reviewed wording. Use Approve for a new line."))?;
                    approved = prior.approved;
                } else {
                    approved = true;
                }
                bind = true;
            }
            EditorialAction::Edit { body } => {
                if locked {
                    return Err(invalid("Unlock the slot and wording before editing it."));
                }
                let i = index.ok_or_else(|| invalid("Choose an existing wording to edit."))?;
                slot.variants[i].text.set_body(body, Provenance::Human);
                slot.variants[i].text.lifecycle.policy = GenerationPolicy::Edited;
                bind = true;
            }
            EditorialAction::Accept { proposal_id, proposal_hash, .. }
            | EditorialAction::Generated { proposal_id, proposal_hash } => {
                if locked {
                    return Err(invalid("Locked dialogue cannot accept generated wording."));
                }
                if decisions.contains_key(proposal_id) {
                    return Err(invalid("This proposal already has a recorded decision."));
                }
                let proposal = super::proposals::checked(self, *proposal_id)?;
                if proposal.hash != *proposal_hash
                    || proposal.request.target.scene != target.scene
                    || proposal.request.target.beat != target.beat
                    || proposal.request.target.slot != target.slot
                    || proposal.request.target.variant != target.variant
                {
                    return Err(invalid("Proposal identity, target or hash changed."));
                }
                super::proposals::unchanged(self, &tx.snapshot, &proposal)?;
                if slot.policy != proposal.request.expected_slot_policy
                    || index.map(|i| slot.variants[i].text.lifecycle.policy)
                        != proposal.request.expected_policy
                    || index.map(|i| &slot.variants[i].text.revision)
                        != proposal.request.expected_text_revision.as_ref()
                {
                    return Err(invalid("Wording or policy changed before proposal acceptance."));
                }
                let automatic = matches!(action, EditorialAction::Generated { .. });
                let generated = slot.policy == GenerationPolicy::Generated
                    && index.is_none_or(|i| {
                        slot.variants[i].text.lifecycle.policy == GenerationPolicy::Generated
                    });
                if automatic && !generated {
                    return Err(invalid(
                        "Human edited dialogue retains a proposal; it cannot be replaced automatically.",
                    ));
                }
                let override_body = if let EditorialAction::Accept { reviewed_text, .. } = &action {
                    reviewed_text.as_ref()
                } else {
                    None
                };
                let body = override_body.unwrap_or(&proposal.candidate.text);
                if body.trim().is_empty()
                    || body.chars().count() > wobu_narrative_generation::MAX_TEXT_CHARS
                {
                    return Err(invalid("Reviewed wording must contain 1–4000 characters."));
                }
                let edited = override_body.is_some_and(|s| s != &proposal.candidate.text);
                let provenance = if edited {
                    Provenance::Human
                } else {
                    Provenance::Generated { fingerprint: proposal.request.hash() }
                };
                let mut text = Text::written(body);
                text.set_body(body, provenance);
                text.lifecycle.policy = if generated && !edited {
                    GenerationPolicy::Generated
                } else {
                    GenerationPolicy::Edited
                };
                if let Some(i) = index {
                    slot.variants[i].text = text;
                } else {
                    slot.variants.push(Variant {
                        id: proposal.candidate.variant_id,
                        when: None,
                        text,
                    });
                }
                decisions.insert(*proposal_id, ProposalDecision::Accepted);
                bind = true;
            }
            EditorialAction::Reject { proposal_id, proposal_hash } => {
                if decisions.contains_key(proposal_id) {
                    return Err(invalid("This proposal already has a recorded decision."));
                }
                let proposal = super::proposals::checked(self, *proposal_id)?;
                if proposal.hash != *proposal_hash
                    || proposal.request.target.scene != target.scene
                    || proposal.request.target.slot != target.slot
                    || proposal.request.target.variant != target.variant
                {
                    return Err(invalid("Proposal does not match the reviewed target/hash."));
                }
                decisions.insert(*proposal_id, ProposalDecision::Rejected);
            }
            _ => return Err(invalid("Unsupported editorial transition.")),
        }
        let mut binding_target = target.clone();
        if bind && binding_target.variant.is_none() {
            binding_target.variant = slot.variants.last().map(|v| v.id);
        }
        // Selected wording is excluded from this semantic projection, while
        // variant identity and conditions remain. Capture after insertion.
        let context = ReviewContext::capture(
            &scene,
            &binding_target,
            serde_json::to_value(&tx.snapshot.world)?,
            serde_json::to_value(&tx.snapshot.schema)?,
            tx.snapshot.characters.clone(),
            tx.snapshot.state.clone(),
        );
        if bind {
            let slot = scene
                .beat_mut(binding_target.beat)
                .unwrap()
                .dialogue
                .iter_mut()
                .find(|s| s.id == binding_target.slot)
                .unwrap();
            let variant =
                slot.variants.iter_mut().find(|v| Some(v.id) == binding_target.variant).unwrap();
            variant.text.lifecycle.review =
                if approved { ReviewState::Approved } else { ReviewState::Draft };
            variant.text.lifecycle.freshness = Freshness::Current;
            bindings.insert(
                variant.id,
                ReviewBinding {
                    target: binding_target,
                    speaker: slot.speaker.clone(),
                    text_revision: variant.text.revision.clone(),
                    context_revision: context.revision.clone(),
                    state: context.state.clone(),
                    approved,
                    event_id,
                },
            );
        }
        let event = event(
            self,
            event_id,
            before,
            scene,
            Some(target),
            action,
            Some(context),
            bindings.clone(),
            decisions.clone(),
        );
        tx.scene = event.after.clone();
        tx.scene.editorial_head = Some(event.id);
        tx.bindings = bindings;
        tx.decisions = decisions;
        tx.events.push(event);
        Ok(())
    }
    pub fn commit_review(&mut self, tx: ReviewTransaction) -> Result<SceneFile> {
        tx.snapshot.check_current(self)?;
        if tx.events.is_empty() {
            return Err(invalid("No review decisions were staged."));
        }
        for event in &tx.events {
            self.write_editorial_event(event)?;
        }
        tx.snapshot.check_current(self)?;
        let mut file = tx.snapshot.file;
        file.scene = tx.scene;
        match self.write_review_scene(&mut file)? {
            SourceSave::Saved(_) => Ok(file),
            SourceSave::Conflict { .. } => Err(invalid(
                "Scene changed before review publication. Receipts remain uncommitted; reload before deciding.",
            )),
        }
    }
    fn write_editorial_event(&mut self, event: &EditorialEvent) -> Result<()> {
        let mut file = NarrativeRecordFile {
            document: NarrativeRecordDocument::new(
                NarrativeRecordKind::Receipt,
                event.id,
                "Narrative editorial decision",
                serde_json::to_value(event)?,
            ),
            stamp: None,
        };
        match self.save_narrative_record(&mut file)? {
            SourceSave::Saved(_) => Ok(()),
            SourceSave::Conflict { .. } => {
                Err(invalid("Editorial receipt identity conflict. No scene was changed."))
            }
        }
    }
    fn write_review_scene(&mut self, file: &mut SceneFile) -> Result<SourceSave> {
        let result = narrative::write_scene(self.root(), file, &self.peer)?;
        if matches!(result, SourceSave::Saved(_)) {
            self.index_narrative_path(&file.rel)?;
        }
        Ok(result)
    }
    pub(crate) fn save_editorial_scene(&mut self, file: &mut SceneFile) -> Result<SourceSave> {
        self.ensure_writable()?;
        let _lock = scene_lock(self, file.scene.id)?;
        self.save_editorial_scene_locked(file)
    }
    fn save_editorial_scene_locked(&mut self, file: &mut SceneFile) -> Result<SourceSave> {
        let previous = if narrative::registry::read(self.root(), &file.rel)?.is_some() {
            Some(narrative::read_scene(self.root(), &file.rel)?)
        } else {
            None
        };
        if previous.as_ref().and_then(|f| f.stamp.as_ref()) != file.stamp.as_ref() {
            let yaml = wobu_narrative::SceneDocument::new(file.scene.clone())
                .to_yaml()
                .map_err(|e| invalid(e.to_string()))?;
            let path = narrative::registry::safe_path(self.root(), &file.rel)?;
            let (conflict_path, _) =
                crate::atomic::park_conflict(self.root(), &path, &yaml, &self.peer)?;
            return Ok(SourceSave::Conflict {
                conflict_path: crate::paths::to_rel_string(
                    conflict_path.strip_prefix(self.root()).unwrap(),
                ),
            });
        }
        validate_manual(previous.as_ref().map(|f| &f.scene), &file.scene)?;
        let mut bindings = BTreeMap::new();
        let mut drafted = std::collections::BTreeSet::new();
        let mut decisions = BTreeMap::new();
        if let Some(old) = &previous {
            if let Ok(snapshot) = self.review_snapshot(old.scene.id, None) {
                bindings = snapshot.bindings();
                decisions =
                    snapshot.history.first().map(|e| e.decisions.clone()).unwrap_or_default();
            }
            for beat in &mut file.scene.beats {
                for slot in &mut beat.dialogue {
                    for variant in &mut slot.variants {
                        let old_text = old
                            .scene
                            .dialogue_slots()
                            .find(|(_, s)| s.id == slot.id)
                            .and_then(|(_, s)| s.variants.iter().find(|v| v.id == variant.id))
                            .map(|v| &v.text);
                        if old_text.is_none_or(|t| {
                            t.body != variant.text.body
                                || t.provenance != variant.text.provenance
                                || t.revision != variant.text.revision
                        }) {
                            if old_text.is_some() || !variant.text.revision_matches() {
                                variant.text.set_body(variant.text.body.clone(), Provenance::Human);
                            }
                            variant.text.lifecycle.review = ReviewState::Draft;
                            drafted.insert(variant.id);
                            if variant.text.lifecycle.policy != GenerationPolicy::Locked {
                                variant.text.lifecycle.policy = GenerationPolicy::Edited;
                            }
                            bindings.remove(&variant.id);
                        } else if old_text
                            .is_some_and(|t| t.lifecycle.review != variant.text.lifecycle.review)
                        {
                            bindings.remove(&variant.id);
                            drafted.insert(variant.id);
                        }
                    }
                }
            }
        } else {
            drafted.extend(
                file.scene
                    .dialogue_slots()
                    .flat_map(|(_, slot)| slot.variants.iter().map(|v| v.id)),
            );
        }
        bindings.retain(|id, b| {
            file.scene.dialogue_slots().any(|(beat, s)| {
                s.variants.iter().any(|v| {
                    v.id == *id
                        && b.matches(
                            &ReviewTarget {
                                scene: file.scene.id,
                                beat,
                                slot: s.id,
                                variant: Some(*id),
                            },
                            &s.speaker,
                            &v.text,
                        )
                })
            })
        });
        let event_id = Id::generate();
        let mut manual_context = None;
        if !drafted.is_empty() {
            // One shared environment per receipt, individual target hashes per
            // wording. No approval is inferred from authoring a draft.
            if let Ok(snapshot) = self.review_source(file.clone(), None)
                && snapshot.input_problem.is_none()
            {
                for (beat, slot) in file.scene.dialogue_slots() {
                    for variant in &slot.variants {
                        if !drafted.contains(&variant.id) || !variant.text.revision_matches() {
                            continue;
                        }
                        let target = ReviewTarget {
                            scene: file.scene.id,
                            beat,
                            slot: slot.id,
                            variant: Some(variant.id),
                        };
                        let context = snapshot.context(&target)?;
                        bindings.insert(
                            variant.id,
                            ReviewBinding {
                                target,
                                speaker: slot.speaker.clone(),
                                text_revision: variant.text.revision.clone(),
                                context_revision: context.revision.clone(),
                                state: context.state.clone(),
                                approved: false,
                                event_id,
                            },
                        );
                        if manual_context.is_none() {
                            manual_context = Some(context);
                        }
                    }
                }
                snapshot.check_current(self)?;
            }
        }
        for beat in &mut file.scene.beats {
            for slot in &mut beat.dialogue {
                for variant in &mut slot.variants {
                    if bindings.get(&variant.id).is_some_and(|b| b.event_id == event_id) {
                        variant.text.lifecycle.freshness = Freshness::Current;
                    }
                }
            }
        }
        let before = if let Some(old) = previous {
            if let Some(id) = old.scene.editorial_head {
                let record =
                    self.narrative_record(NarrativeRecordKind::Receipt, id)?.ok_or_else(|| {
                        invalid("Restore the missing editorial receipt before saving this scene.")
                    })?;
                let event: EditorialEvent = serde_json::from_value(record.document.payload)?;
                if !event.valid() || event.id != id || event.after.id != old.scene.id {
                    return Err(invalid(
                        "Invalid editorial history must be repaired before saving.",
                    ));
                }
                let mut recorded = event.after;
                recorded.editorial_head = Some(id);
                recorded
            } else {
                old.scene
            }
        } else {
            let mut scene = file.scene.clone();
            scene.editorial_head = None;
            scene
        };
        let event = event(
            self,
            event_id,
            before,
            file.scene.clone(),
            None,
            EditorialAction::ManualSave,
            manual_context,
            bindings,
            decisions,
        );
        self.write_editorial_event(&event)?;
        file.scene = event.after;
        file.scene.editorial_head = Some(event.id);
        self.write_review_scene(file)
    }
}
#[allow(clippy::too_many_arguments)]
fn event(
    project: &Project,
    id: Id,
    before: Scene,
    mut after: Scene,
    target: Option<ReviewTarget>,
    action: EditorialAction,
    context: Option<ReviewContext>,
    bindings: BTreeMap<VariantId, ReviewBinding>,
    decisions: BTreeMap<Id, ProposalDecision>,
) -> EditorialEvent {
    after.editorial_head = None;
    EditorialEvent {
        record_type: "narrative_editorial".into(),
        version: REVIEW_VERSION,
        id,
        parent: before.editorial_head,
        target,
        action,
        actor: project.peer.to_string(),
        context,
        before,
        after,
        bindings,
        decisions,
    }
}
/// Writable scene flags are mirrors only. Normal source and undo may preserve
/// existing metadata, but cannot introduce an approval, move a head or unlock.
pub fn validate_manual(previous: Option<&Scene>, next: &Scene) -> Result<()> {
    if previous.is_some_and(|s| s.id != next.id) {
        return Err(invalid("Keep the original scene identity when saving."));
    }
    if next.editorial_head != previous.and_then(|s| s.editorial_head) {
        return Err(invalid("Editorial history can only change through review commands."));
    }
    for (_, old) in previous.into_iter().flat_map(Scene::dialogue_slots) {
        let next_slot = next.dialogue_slots().find(|(_, s)| s.id == old.id).map(|(_, s)| s);
        if next_slot.is_none()
            && (old.policy == GenerationPolicy::Locked
                || old.variants.iter().any(|v| v.text.lifecycle.policy == GenerationPolicy::Locked))
        {
            return Err(invalid("Unlock protected dialogue before deleting it."));
        }
        if let Some(new) = next_slot {
            if old.policy == GenerationPolicy::Locked && new != old {
                return Err(invalid("Unlock the slot before changing protected dialogue."));
            }
            if old.speaker != new.speaker
                && old.variants.iter().any(|v| v.text.lifecycle.policy == GenerationPolicy::Locked)
            {
                return Err(invalid("Unlock protected wording before changing its speaker."));
            }
            if old.policy != new.policy {
                return Err(invalid(
                    "Change existing slot policy through its explicit review control.",
                ));
            }
            for old_variant in &old.variants {
                let new_variant = new.variants.iter().find(|v| v.id == old_variant.id);
                if (old.policy == GenerationPolicy::Locked
                    || old_variant.text.lifecycle.policy == GenerationPolicy::Locked)
                    && new_variant
                        .is_none_or(|v| v.text != old_variant.text || v.when != old_variant.when)
                {
                    return Err(invalid("Unlock protected dialogue before changing it."));
                }
                if new_variant.is_some_and(|v| {
                    v.text.lifecycle.policy != old_variant.text.lifecycle.policy
                        && !(v.text.lifecycle.policy == GenerationPolicy::Edited
                            && v.text.body != old_variant.text.body
                            && old_variant.text.lifecycle.policy != GenerationPolicy::Locked)
                }) {
                    return Err(invalid(
                        "Change existing wording policy through its explicit review control.",
                    ));
                }
            }
        }
    }
    for (_, slot) in next.dialogue_slots() {
        for variant in &slot.variants {
            let old = previous
                .into_iter()
                .flat_map(Scene::dialogue_slots)
                .find(|(_, s)| s.id == slot.id)
                .and_then(|(_, s)| s.variants.iter().find(|v| v.id == variant.id));
            if variant.text.lifecycle.review == ReviewState::Approved
                && old.is_none_or(|v| v.text != variant.text)
            {
                return Err(invalid(
                    "Only an explicit review can approve exact wording and context. Save manual wording as Draft.",
                ));
            }
            if old.is_none() && variant.text.lifecycle.policy == GenerationPolicy::Generated {
                return Err(invalid("New manual wording begins Edited or Locked."));
            }
        }
    }
    Ok(())
}

fn undo_content(scene: &Scene) -> Scene {
    let mut copy = scene.clone();
    copy.editorial_head = None;
    for beat in &mut copy.beats {
        for slot in &mut beat.dialogue {
            for variant in &mut slot.variants {
                variant.text.lifecycle.review = ReviewState::Draft;
                variant.text.lifecycle.freshness = Freshness::Current;
            }
        }
    }
    copy
}
