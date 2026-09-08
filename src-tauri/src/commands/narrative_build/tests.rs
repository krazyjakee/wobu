use super::*;
use crate::commands::narrative_generation::task;
use wobu_narrative::review::PolicyScope;
use wobu_narrative::{Beat, DialogueSlot, GenerationPolicy, Speaker, Text, Variant};
use wobu_narrative_build::Input;
use wobu_store::NarrativeRecordKind;

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("wobu-build-{}", wobu_core::new_id()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(temp: &Temp, count: usize) -> (Project, wobu_narrative::SceneId) {
    let mut project = Project::create(&temp.0, "Build fixture").unwrap();
    let mut file = project.create_scene("Harbour").unwrap();
    let mut beat = Beat::new("The beacon");
    beat.outcomes.push(wobu_narrative::Outcome::new(wobu_narrative::Destination::End {
        label: "End".into(),
    }));
    for _ in 0..count {
        let mut slot = DialogueSlot::new(Speaker::Narrator);
        slot.policy = GenerationPolicy::Generated;
        beat.dialogue.push(slot);
    }
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();
    (project, file.scene.id)
}
fn input(scope: Scope) -> Input {
    Input {
        scope,
        containers: BTreeSet::new(),
        state: BTreeMap::new(),
        commands: BTreeMap::new(),
        token_budget: 4000,
        max_output_tokens: 512,
    }
}
fn make(project: &mut Project, scope: Scope) -> Build {
    plan::build(project, input(scope), "fixture", "offline-mock").unwrap()
}
fn selected(build: &Build) -> BTreeSet<Id> {
    build.items.iter().filter(|i| i.request_id.is_some()).map(|i| i.id).collect()
}
fn success(request: &FrozenRequest) -> Receipt {
    let raw=serde_json::json!({"lines":[{"slot_id":request.target.slot,"variant_id":request.candidate_variant_id,"speaker":request.speaker,"text":"The beacon is dark."}]}).to_string();
    Receipt::NarrativeGenerationAttempt {
        version: 1,
        request_id: request.request_id,
        request_hash: request.hash(),
        attempt: 1,
        status: AttemptStatus::Succeeded,
        usage: Default::default(),
        billing_unknown: false,
        error_code: None,
        candidate: Some(request.validate_output(&raw).unwrap()),
        raw_accepted_output: Some(raw),
    }
}
#[test]
fn keyless_483_item_plan_is_durable_and_portable() {
    let temp = Temp::new();
    let (mut project, _) = fixture(&temp, 483);
    let build = make(&mut project, Scope::Missing);
    assert_eq!(build.items.len(), 483);
    assert!(build.items.iter().all(|i| i.action == Action::Generate));
    assert!(generation::history(&project).unwrap().iter().all(|i| i.attempts == 0));
    for file in project.narrative_records(NarrativeRecordKind::Receipt).unwrap() {
        let bytes = std::fs::read(project.root().join(file.document.rel())).unwrap();
        assert!(bytes.len() <= wobu_store::project::narrative_sync::MAX_NARRATIVE_FILE_BYTES);
    }
    drop(project);
    let reopened = Project::open(&temp.0.join("build-fixture.wobu")).unwrap();
    let restored = reopened.narrative_build(build.id).unwrap();
    assert_eq!(
        restored.items.iter().map(|i| i.id).collect::<Vec<_>>(),
        build.items.iter().map(|i| i.id).collect::<Vec<_>>()
    );
    assert_eq!(reopened.narrative_build_ids().unwrap(), vec![build.id]);
}
#[test]
fn generated_siblings_complete_then_resume_without_resubmission() {
    let temp = Temp::new();
    let (mut project, scene) = fixture(&temp, 2);
    let build = make(&mut project, Scope::Missing);
    let requests = prepare(&mut project, build.id, &selected(&build)).unwrap();
    for request in &requests {
        assert!(task::prepare(&project, request).is_ok());
        task::persist(&mut project, request, wobu_core::new_id(), &success(request)).unwrap();
    }
    let file = project.load_scene(scene).unwrap();
    assert!(
        file.scene.beats[0]
            .dialogue
            .iter()
            .all(|s| s.variants.len() == 1 && s.variants[0].text.body == "The beacon is dark.")
    );
    drop(project);
    let mut project = Project::open(&temp.0.join("build-fixture.wobu")).unwrap();
    assert!(prepare(&mut project, build.id, &selected(&build)).unwrap().is_empty());
    assert_eq!(
        status(&project, build.id)
            .unwrap()
            .history
            .iter()
            .filter(|h| h.status == "succeeded")
            .count(),
        2
    );
    assert!(
        super::super::narrative_preview::compile_project(&project, BTreeMap::new())
            .unwrap()
            .graph
            .is_some()
    );
}
#[test]
fn successful_unpublished_output_is_reused_without_a_new_attempt() {
    let temp = Temp::new();
    let (mut project, _) = fixture(&temp, 1);
    let first = make(&mut project, Scope::Missing);
    let request = prepare(&mut project, first.id, &selected(&first)).unwrap().remove(0);
    let receipt = success(&request);
    let receipt_id = wobu_core::new_id();
    records::save_receipt(&mut project, receipt_id, "Narrative generation attempt", &receipt)
        .unwrap();
    let second = make(&mut project, Scope::Missing);
    assert!(second.items[0].reusable);
    assert_eq!(second.items[0].request_id, Some(request.request_id));
    assert!(prepare(&mut project, second.id, &selected(&second)).unwrap().is_empty());
    assert!(project.narrative_publication(receipt_id).unwrap().is_some());
    assert_eq!(records::attempts(&project, &request).unwrap().len(), 1);
}
#[test]
fn execution_rejects_source_changes_and_new_locks() {
    for lock in [true, false] {
        let temp = Temp::new();
        let (mut project, scene) = fixture(&temp, 1);
        let build = make(&mut project, Scope::Missing);
        let request = prepare(&mut project, build.id, &selected(&build)).unwrap().remove(0);
        let mut file = project.load_scene(scene).unwrap();
        if lock {
            policy(&mut project, scene, 0, PolicyScope::Slot, GenerationPolicy::Locked);
        } else {
            file.scene.summary = "The changed intent".into();
            project.save_scene(&mut file).unwrap();
        }
        assert!(prepare(&mut project, build.id, &selected(&build)).is_err());
        assert!(task::prepare(&project, &request).is_err());
        task::persist(&mut project, &request, wobu_core::new_id(), &success(&request)).unwrap();
        assert!(project.load_scene(scene).unwrap().scene.beats[0].dialogue[0].variants.is_empty());
        assert_eq!(
            generation::history(&project).unwrap()[0].proposal_current_at_publication,
            Some(false)
        );
    }
}
#[test]
fn edited_proposals_and_locked_rows_keep_authored_words() {
    let temp = Temp::new();
    let (mut project, scene) = fixture(&temp, 2);
    let mut file = project.load_scene(scene).unwrap();
    for slot in &mut file.scene.beats[0].dialogue {
        slot.variants.push(Variant::new(Text::written("Human wording")));
    }
    project.save_scene(&mut file).unwrap();
    for index in 0..2 {
        policy(
            &mut project,
            scene,
            index,
            PolicyScope::Slot,
            if index == 0 { GenerationPolicy::Edited } else { GenerationPolicy::Locked },
        );
    }
    let build = make(&mut project, Scope::AllSelected);
    assert_eq!(
        build.items.iter().map(|i| i.action).collect::<Vec<_>>(),
        vec![Action::Propose, Action::Locked]
    );
    let request = prepare(&mut project, build.id, &selected(&build)).unwrap().remove(0);
    task::persist(&mut project, &request, wobu_core::new_id(), &success(&request)).unwrap();
    assert!(
        project.load_scene(scene).unwrap().scene.beats[0]
            .dialogue
            .iter()
            .all(|s| s.variants[0].text.body == "Human wording")
    );
    assert_eq!(
        generation::history(&project).unwrap()[0].proposal_current_at_publication,
        Some(true)
    );
}

#[test]
fn voice_change_plans_only_dependent_wording_and_preserves_policies() {
    let temp = Temp::new();
    let (mut project, _) = fixture(&temp, 0);
    let mut actor = project.create_node(wobu_core::NodeKind::Character, "Speaker", None).unwrap();
    actor.attributes.insert("narrative_voice".into(), serde_json::json!("Quiet"));
    project.save_node(actor.clone()).unwrap();
    let mut file = project.create_scene("Spoken scene").unwrap();
    file.scene
        .participants
        .push(wobu_narrative::Participant { entity: actor.id, role: String::new() });
    let mut beat = Beat::new("Listen");
    beat.outcomes.push(wobu_narrative::Outcome::new(wobu_narrative::Destination::End {
        label: "End".into(),
    }));
    for policy in [GenerationPolicy::Generated, GenerationPolicy::Edited, GenerationPolicy::Locked]
    {
        let mut slot = DialogueSlot::new(Speaker::Entity(actor.id));
        slot.policy = policy;
        let mut variant = Variant::new(Text::written("A quiet line."));
        variant.text.lifecycle.policy =
            if policy == GenerationPolicy::Generated { GenerationPolicy::Edited } else { policy };
        slot.variants.push(variant);
        beat.dialogue.push(slot);
    }
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();
    file =
        policy(&mut project, file.scene.id, 0, PolicyScope::Variant, GenerationPolicy::Generated);
    let before = file.scene.clone();
    assert!(make(&mut project, Scope::Affected).items.is_empty());
    actor.attributes.insert("narrative_voice".into(), serde_json::json!("Sharp"));
    project.save_node(actor).unwrap();
    let build = make(&mut project, Scope::Affected);
    assert_eq!(build.items.len(), 3);
    assert_eq!(
        build.items.iter().map(|i| i.action).collect::<Vec<_>>(),
        vec![Action::Generate, Action::Propose, Action::Locked]
    );
    assert!(
        build.items.iter().all(|i| i.reasons.iter().any(|r| r.source.ends_with("narrative_voice")))
    );
    let current = project.load_scene(file.scene.id).unwrap();
    for (old, new) in before.beats[0].dialogue.iter().zip(&current.scene.beats[0].dialogue) {
        assert_eq!(old.variants[0].text.body, new.variants[0].text.body);
        assert_eq!(old.variants[0].text.revision, new.variants[0].text.revision);
        assert_eq!(old.policy, new.policy);
    }
}

#[test]
fn partial_failure_and_cancel_resume_survive_a_fresh_index_without_losing_evidence() {
    let temp = Temp::new();
    let (mut project, scene) = fixture(&temp, 3);
    let build = make(&mut project, Scope::Missing);
    let requests = prepare(&mut project, build.id, &selected(&build)).unwrap();
    task::persist(&mut project, &requests[0], wobu_core::new_id(), &success(&requests[0])).unwrap();
    for (request, status) in
        requests[1..].iter().zip([AttemptStatus::Failed, AttemptStatus::Cancelled])
    {
        task::persist(
            &mut project,
            request,
            wobu_core::new_id(),
            &Receipt::NarrativeGenerationAttempt {
                version: 1,
                request_id: request.request_id,
                request_hash: request.hash(),
                attempt: 1,
                status,
                usage: Default::default(),
                billing_unknown: true,
                error_code: Some("provider.cancelled_or_failed".into()),
                raw_accepted_output: None,
                candidate: None,
            },
        )
        .unwrap();
    }
    let production_id = wobu_core::new_id();
    let production = wobu_store::NarrativeRecordDocument::new(
        NarrativeRecordKind::Production,
        production_id,
        "Retained production reference",
        serde_json::json!({"type":"build_fixture_production_reference", "request_id":requests[0].request_id}),
    );
    project
        .save_narrative_record(&mut wobu_store::NarrativeRecordFile {
            document: production.clone(),
            stamp: None,
        })
        .unwrap();
    let evidence = project
        .narrative_records(NarrativeRecordKind::Receipt)
        .unwrap()
        .into_iter()
        .map(|f| (f.document.rel(), std::fs::read(project.root().join(f.document.rel())).unwrap()))
        .collect::<Vec<_>>();
    let root = project.root().to_path_buf();
    drop(project);
    let mut project = Project::open_at_index(&root, &temp.0.join("fresh-index.db")).unwrap();
    assert_eq!(
        project
            .narrative_record(NarrativeRecordKind::Production, production_id)
            .unwrap()
            .unwrap()
            .document,
        production
    );
    let pending = prepare(&mut project, build.id, &selected(&build)).unwrap();
    assert_eq!(
        pending.iter().map(|r| r.request_id).collect::<BTreeSet<_>>(),
        requests[1..].iter().map(|r| r.request_id).collect()
    );
    assert_eq!(
        project.load_scene(scene).unwrap().scene.beats[0].dialogue[0].variants[0].text.body,
        "The beacon is dark."
    );
    for (rel, bytes) in evidence {
        assert_eq!(std::fs::read(root.join(rel)).unwrap(), bytes);
    }
    assert!(task::prepare(&project, &requests[0]).is_err());
    for request in pending {
        assert_eq!(task::prepare(&project, &request).unwrap(), 2);
    }
}

#[test]
fn planning_with_another_model_does_not_change_the_accepted_producer() {
    let temp = Temp::new();
    let (mut project, _) = fixture(&temp, 1);
    let build = make(&mut project, Scope::Missing);
    let request = prepare(&mut project, build.id, &selected(&build)).unwrap().remove(0);
    task::persist(&mut project, &request, wobu_core::new_id(), &success(&request)).unwrap();
    let before = project.narrative_dependency_snapshot().unwrap().producers;
    assert_eq!(before.len(), 1);
    plan::build(&mut project, input(Scope::AllSelected), "fixture", "different-model").unwrap();
    assert_eq!(project.narrative_dependency_snapshot().unwrap().producers, before);
}

#[test]
fn matching_edited_result_reuses_but_changed_model_and_context_do_not() {
    let temp = Temp::new();
    let (mut project, scene) = fixture(&temp, 1);
    policy(&mut project, scene, 0, PolicyScope::Slot, GenerationPolicy::Edited);
    let first = make(&mut project, Scope::Missing);
    let request = prepare(&mut project, first.id, &selected(&first)).unwrap().remove(0);
    task::persist(&mut project, &request, wobu_core::new_id(), &success(&request)).unwrap();
    assert!(make(&mut project, Scope::Missing).items[0].reusable);
    let changed_model =
        plan::build(&mut project, input(Scope::Missing), "fixture", "different-model").unwrap();
    assert!(!changed_model.items[0].reusable);
    let mut file = project.load_scene(scene).unwrap();
    file.scene.summary = "Different creative intent".into();
    project.save_scene(&mut file).unwrap();
    assert!(!make(&mut project, Scope::Missing).items[0].reusable);
}

#[test]
fn independent_codex_siblings_apply_but_ambient_context_changes_stop_later_items() {
    use wobu_narrative::{Name, TextEntry, TextKind};
    for kind in [TextKind::Codex, TextKind::Ambient] {
        let temp = Temp::new();
        let (mut project, _) = fixture(&temp, 0);
        let mut file = project
            .create_text_asset(kind, "Supporting build", Name::new("arrive").unwrap())
            .unwrap();
        file.asset.policy = GenerationPolicy::Generated;
        let mut entry = TextEntry::new("Arrival");
        for _ in 0..2 {
            let mut slot = DialogueSlot::new(Speaker::Narrator);
            slot.policy = GenerationPolicy::Generated;
            entry.lines.push(slot);
        }
        file.asset.entries.push(entry);
        project.save_text_asset(&mut file).unwrap();
        let build = make(&mut project, Scope::Missing);
        assert_eq!(build.items.len(), 2);
        assert!(build.items.iter().all(|i| i.asset && i.action == Action::Generate));
        let requests = prepare(&mut project, build.id, &selected(&build)).unwrap();
        task::persist(&mut project, &requests[0], wobu_core::new_id(), &success(&requests[0]))
            .unwrap();
        if kind == TextKind::Ambient {
            assert!(task::prepare(&project, &requests[1]).is_err());
            task::persist(&mut project, &requests[1], wobu_core::new_id(), &success(&requests[1]))
                .unwrap();
            assert!(
                project.load_text_asset(file.asset.id).unwrap().asset.entries[0].lines[1]
                    .variants
                    .is_empty()
            );
        } else {
            assert!(task::prepare(&project, &requests[1]).is_ok());
            task::persist(&mut project, &requests[1], wobu_core::new_id(), &success(&requests[1]))
                .unwrap();
            assert_eq!(
                project.load_text_asset(file.asset.id).unwrap().asset.entries[0].lines[1]
                    .variants
                    .len(),
                1
            );
        }
    }
}

#[test]
fn malformed_retained_output_and_unknown_selection_are_visible_plan_diagnostics() {
    let temp = Temp::new();
    let (mut project, _) = fixture(&temp, 1);
    let build = make(&mut project, Scope::Missing);
    let request = prepare(&mut project, build.id, &selected(&build)).unwrap().remove(0);
    let mut receipt = success(&request);
    if let Receipt::NarrativeGenerationAttempt { raw_accepted_output, .. } = &mut receipt {
        *raw_accepted_output = Some("malformed output".into());
    }
    records::save_receipt(
        &mut project,
        wobu_core::new_id(),
        "Narrative generation attempt",
        &receipt,
    )
    .unwrap();
    let next = make(&mut project, Scope::Missing);
    assert!(!next.items[0].reusable);
    assert!(next.diagnostics.iter().any(|d| d.contains("invalid retained output")));
    let mut spec = input(Scope::AllSelected);
    spec.containers.insert(wobu_narrative::SceneId::new());
    let missing = plan::build(&mut project, spec, "fixture", "offline-mock").unwrap();
    assert!(missing.items.is_empty());
    assert!(missing.diagnostics.iter().any(|d| d.contains("no longer exists")));
}

fn policy(
    project: &mut Project,
    scene: wobu_narrative::SceneId,
    index: usize,
    scope: PolicyScope,
    policy: GenerationPolicy,
) -> wobu_store::SceneFile {
    let mut view = project.review_scene(scene, None).unwrap();
    let line = view.lines.remove(index);
    project
        .apply_review(&wobu_store::project::narrative_review::ReviewRequest {
            guard: view.guard,
            target: line.target,
            context_revision: line.context_revision,
            state_json: view.state_json,
            action: wobu_narrative::review::EditorialAction::Policy { scope, policy },
        })
        .unwrap()
        .0
}

#[test]
fn resume_recovers_generated_acceptance_after_a_temporary_editorial_lock() {
    use wobu_narrative::review::EditorialAction;
    use wobu_store::project::narrative_review::ReviewRequest;
    let temp = Temp::new();
    let (mut project, scene) = fixture(&temp, 1);
    let build = make(&mut project, Scope::Missing);
    assert!(!build.items[0].reusable);
    let request = prepare(&mut project, build.id, &selected(&build)).unwrap().remove(0);
    let mut view = project.review_scene(scene, None).unwrap();
    let line = view.lines.remove(0);
    let held = project
        .begin_review(&ReviewRequest {
            guard: view.guard,
            target: line.target,
            context_revision: line.context_revision,
            state_json: view.state_json,
            action: EditorialAction::Policy {
                scope: PolicyScope::Slot,
                policy: GenerationPolicy::Generated,
            },
        })
        .unwrap();
    let receipt_id = wobu_core::new_id();
    task::persist(&mut project, &request, receipt_id, &success(&request)).unwrap();
    let pending = status(&project, build.id).unwrap();
    assert!(pending.history[0].proposal_published);
    assert!(!pending.decided.contains(&receipt_id));
    assert!(project.load_scene(scene).unwrap().scene.beats[0].dialogue[0].variants.is_empty());
    assert!(prepare(&mut project, build.id, &selected(&build)).is_err());
    assert_eq!(records::attempts(&project, &request).unwrap().len(), 1);
    drop(held);
    assert!(prepare(&mut project, build.id, &selected(&build)).unwrap().is_empty());
    assert_eq!(records::attempts(&project, &request).unwrap().len(), 1);
    assert!(status(&project, build.id).unwrap().decided.contains(&receipt_id));
    assert_eq!(
        project.load_scene(scene).unwrap().scene.beats[0].dialogue[0].variants[0].text.body,
        "The beacon is dark."
    );
    // Completion is historical: a later deliberate edit never restores old output.
    let mut view = project.review_scene(scene, None).unwrap();
    let line = view.lines.remove(0);
    project
        .apply_review(&ReviewRequest {
            guard: view.guard,
            target: line.target,
            context_revision: line.context_revision,
            state_json: view.state_json,
            action: EditorialAction::Edit { body: "Writer's revision".into() },
        })
        .unwrap();
    assert!(status(&project, build.id).unwrap().decided.contains(&receipt_id));
    assert!(prepare(&mut project, build.id, &selected(&build)).unwrap().is_empty());
    assert_eq!(
        project.load_scene(scene).unwrap().scene.beats[0].dialogue[0].variants[0].text.body,
        "Writer's revision"
    );
}

#[test]
fn linked_scene_v2_eligibility_reads_intent_but_not_dialogue_or_history() {
    use wobu_narrative::{Name, SourceLink, TextEntry, TextKind};
    use wobu_narrative_generation::context_matches;
    let temp = Temp::new();
    let (mut project, scene) = fixture(&temp, 1);
    let mut asset =
        project.create_text_asset(TextKind::Codex, "Codex", Name::new("read").unwrap()).unwrap();
    asset.asset.policy = GenerationPolicy::Generated;
    asset.asset.sources.push(SourceLink::Scene(scene));
    let mut entry = TextEntry::new("Summary");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.policy = GenerationPolicy::Generated;
    entry.lines.push(slot);
    asset.asset.entries.push(entry);
    project.save_text_asset(&mut asset).unwrap();
    let build = make(&mut project, Scope::Missing);
    let requests = prepare(&mut project, build.id, &selected(&build)).unwrap();
    let linked = requests.iter().find(|r| r.target.scene == scene).unwrap();
    let supporting = requests.iter().find(|r| r.target.scene != scene).unwrap();
    task::persist(&mut project, linked, wobu_core::new_id(), &success(linked)).unwrap();
    let current = wobu_store::project::narrative_context::capture(
        &project,
        supporting.context.options.clone(),
        || {},
    )
    .unwrap();
    assert_ne!(current.hash, supporting.context.hash);
    assert!(context_matches(supporting, &current));
    let mut tampered = supporting.clone();
    tampered.context.dependencies = current.dependencies.clone();
    assert!(tampered.validate().is_err(), "Full frozen evidence still binds aggregate reads");
    let mut legacy = supporting.clone();
    legacy.version = 1;
    assert!(!context_matches(&legacy, &current));
    assert!(task::prepare(&project, supporting).is_ok());
    let mut file = project.load_scene(scene).unwrap();
    file.scene.summary = "Changed intent".into();
    project.save_scene(&mut file).unwrap();
    assert!(task::prepare(&project, supporting).is_err());
    file.scene.summary.clear();
    project.save_scene(&mut file).unwrap();
    task::persist(&mut project, supporting, wobu_core::new_id(), &success(supporting)).unwrap();
    assert_eq!(
        project.load_text_asset(asset.asset.id).unwrap().asset.entries[0].lines[0].variants[0]
            .text
            .body,
        "The beacon is dark."
    );
}
