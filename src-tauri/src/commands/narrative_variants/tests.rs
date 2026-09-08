use super::*;
use wobu_narrative::{GenerationPolicy, StateDocument};
struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("wobu-variants-{}", Id::generate()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(temp: &Temp) -> (Project, Policy, DialogueSlotId) {
    let f = variants::example::fixture();
    let mut project = Project::create(&temp.0, "Variants").unwrap();
    project.save_state(&StateDocument::new(f.schema.iter().cloned().collect()), None).unwrap();
    project.save_world(&f.world, None).unwrap();
    let mut file = wobu_store::SceneFile {
        scene: f.scenes[0].clone(),
        rel: "narrative/scenes/hearing.yaml".into(),
        stamp: None,
    };
    file.scene.beats[0].outcomes.push(wobu_narrative::Outcome::new(
        wobu_narrative::Destination::End { label: "End".into() },
    ));
    // Keep authored predicates for relevance, plus a distinct empty Generated slot.
    let mut slot = wobu_narrative::DialogueSlot::new(wobu_narrative::Speaker::Narrator);
    slot.policy = GenerationPolicy::Generated;
    let id = slot.id;
    file.scene.beats[0].dialogue.push(slot);
    project.save_scene(&mut file).unwrap();
    (project, f.policy, id)
}
fn planned(project: &mut Project, policy: Policy) -> AnalysisReport {
    let guard = project.narrative_analysis_capture().unwrap().guard;
    plan(project, policy, &guard).unwrap()
}
fn selected(saved: &AnalysisReport) -> Vec<String> {
    saved
        .report
        .rows
        .iter()
        .filter(|r| r.classification == variants::Classification::Included)
        .map(|r| r.id.clone())
        .collect()
}
#[test]
fn keyless_matrix_materializes_seven_guarded_targets_then_freezes_per_item_states() {
    let temp = Temp::new();
    let (mut project, policy, slot) = fixture(&temp);
    let before = project.load_scene(policy.target.scene).unwrap();
    let saved = planned(&mut project, policy.clone());
    assert_eq!(saved.report.included.value, "7");
    assert_eq!(project.load_scene(policy.target.scene).unwrap().scene, before.scene);
    let rows = selected(&saved);
    let changed = project.materialize_narrative_analysis(saved.id, slot, &rows).unwrap();
    assert_eq!(changed.states.len(), 7);
    assert_eq!(changed.after.scene.beats[0].dialogue[1].variants.len(), 7);
    let build = build(&mut project, saved.id, slot, &rows).unwrap();
    assert_eq!(build.items.len(), 7);
    assert!(build.items.iter().all(|i| i.request_id.is_some()), "{:?}", build.diagnostics);
    let requests = super::super::narrative_generation::records::RecordSet::load(&project).unwrap();
    for item in &build.items {
        let request = &requests.requests[&item.request_id.unwrap()];
        assert_eq!(request.context.options.state, item.state);
        assert!(request.target.variant.is_some());
        assert_eq!(request.analysis.as_ref().unwrap().report, Some(saved.id));
        wobu_store::project::narrative_generation::checks(&project, request).unwrap();
    }
    let undone = project
        .restore_scene(
            changed.before.scene.clone(),
            Some(&changed.after.scene),
            changed.before.slug(),
        )
        .unwrap();
    assert!(undone.scene.beats[0].dialogue[1].variants.is_empty());
    let redone = project
        .restore_scene(changed.after.scene.clone(), Some(&undone.scene), changed.after.slug())
        .unwrap();
    assert!(
        redone.scene.beats[0].dialogue[1]
            .variants
            .iter()
            .all(|v| v.text.lifecycle.policy == GenerationPolicy::Generated)
    );
}
#[test]
fn policy_changes_reject_materialization_and_retained_requests_without_rewriting_source() {
    let temp = Temp::new();
    let (mut project, policy, slot) = fixture(&temp);
    let saved = planned(&mut project, policy.clone());
    let rows = selected(&saved);
    project.materialize_narrative_analysis(saved.id, slot, &rows).unwrap();
    let build = build(&mut project, saved.id, slot, &rows).unwrap();
    let requests = super::super::narrative_generation::records::RecordSet::load(&project).unwrap();
    let request = &requests.requests[&build.items[0].request_id.unwrap()];
    let before = project.load_scene(policy.target.scene).unwrap().scene;
    let capture = project.narrative_analysis_capture().unwrap();
    let mut changed = policy;
    changed.limits.states = 2;
    project.save_narrative_analysis_policy(changed, &capture).unwrap();
    assert!(wobu_store::project::narrative_generation::checks(&project, request).is_err());
    assert!(project.materialize_narrative_analysis(saved.id, slot, &rows).is_err());
    assert_eq!(project.load_scene(before.id).unwrap().scene, before);
    let mut legacy = request.clone();
    legacy.analysis = None;
    assert!(legacy.validate().is_ok());
    assert!(wobu_store::project::narrative_generation::checks(&project, &legacy).is_err());
}
#[test]
fn report_rebuild_is_canonical_and_equivalent_reports_keep_reuse_identity() {
    let temp = Temp::new();
    let (mut project, policy, slot) = fixture(&temp);
    let first = planned(&mut project, policy.clone());
    let rows = selected(&first);
    project.materialize_narrative_analysis(first.id, slot, &rows).unwrap();
    let a = build(&mut project, first.id, slot, &rows).unwrap();
    let records = super::super::narrative_generation::records::RecordSet::load(&project).unwrap();
    let request = records.requests[&a.items[0].request_id.unwrap()].clone();
    let key = wobu_narrative_build::reuse_key(&request);
    let second = planned(&mut project, policy);
    let mut same = request.clone();
    same.analysis.as_mut().unwrap().report = Some(second.id);
    assert_ne!(same.hash(), request.hash());
    assert_eq!(wobu_narrative_build::reuse_key(&same), key);
    wobu_store::project::narrative_generation::checks(&project, &same).unwrap();
    let root = project.root().to_path_buf();
    drop(project);
    let project = Project::open(&root).unwrap();
    assert_eq!(project.narrative_analysis_report(first.id).unwrap().report.included.value, "7");
    wobu_store::project::narrative_generation::checks(&project, &request).unwrap();
}
#[test]
fn locked_slot_and_unknown_rows_never_materialize() {
    let temp = Temp::new();
    let (mut project, policy, slot) = fixture(&temp);
    let view = project.review_scene(policy.target.scene, None).unwrap();
    let line = view.lines.iter().find(|l| l.target.slot == slot).unwrap();
    project
        .apply_review(&wobu_store::project::narrative_review::ReviewRequest {
            guard: view.guard.clone(),
            target: line.target.clone(),
            context_revision: line.context_revision.clone(),
            state_json: view.state_json.clone(),
            action: wobu_narrative::review::EditorialAction::Policy {
                scope: wobu_narrative::review::PolicyScope::Slot,
                policy: GenerationPolicy::Locked,
            },
        })
        .unwrap();
    let saved = planned(&mut project, policy.clone());
    assert!(project.materialize_narrative_analysis(saved.id, slot, &selected(&saved)).is_err());
    let row = saved
        .report
        .rows
        .iter()
        .find(|r| r.classification == variants::Classification::Excluded)
        .unwrap();
    assert!(
        project
            .materialize_narrative_analysis(saved.id, slot, std::slice::from_ref(&row.id))
            .is_err()
    );
}
fn success(
    request: &wobu_narrative_generation::FrozenRequest,
) -> wobu_narrative_generation::Receipt {
    let raw=serde_json::json!({"lines":[{"slot_id":request.target.slot,"variant_id":request.candidate_variant_id,"speaker":request.speaker,"text":"Synthetic prepared hearing line."}]}).to_string();
    wobu_narrative_generation::Receipt::NarrativeGenerationAttempt {
        version: 1,
        request_id: request.request_id,
        request_hash: request.hash(),
        attempt: 1,
        status: wobu_narrative_generation::AttemptStatus::Succeeded,
        usage: Default::default(),
        billing_unknown: false,
        error_code: None,
        candidate: Some(request.validate_output(&raw).unwrap()),
        raw_accepted_output: Some(raw),
    }
}
#[test]
fn successful_output_after_a_policy_race_is_retained_without_automatic_acceptance() {
    let temp = Temp::new();
    let (mut project, policy, slot) = fixture(&temp);
    let saved = planned(&mut project, policy.clone());
    let rows = selected(&saved);
    project.materialize_narrative_analysis(saved.id, slot, &rows).unwrap();
    let build = build(&mut project, saved.id, slot, &rows).unwrap();
    let records = super::super::narrative_generation::records::RecordSet::load(&project).unwrap();
    let request = records.requests[&build.items[0].request_id.unwrap()].clone();
    let receipt = success(&request);
    let receipt_id = Id::generate();
    let capture = project.narrative_analysis_capture().unwrap();
    let mut changed = policy;
    changed.external = true;
    project.save_narrative_analysis_policy(changed, &capture).unwrap();
    super::super::narrative_generation::records::publish(
        &mut project,
        &request,
        receipt_id,
        &receipt,
    )
    .unwrap();
    assert!(!project.apply_generated_proposal(receipt_id).unwrap_or(false));
    let file = project.load_scene(request.target.scene).unwrap();
    assert!(file.scene.beats[0].dialogue[1].variants.iter().all(|v| v.text.body.is_empty()));
    assert!(
        wobu_store::project::narrative_generation::published(
            &project, &request, receipt_id, &receipt
        )
        .unwrap()
        .is_some_and(|checks| !checks.current())
    );
}
#[test]
fn a_copy_with_new_mtimes_and_deleted_local_index_keeps_policy_and_request_evidence() {
    fn copy(source: &std::path::Path, dest: &std::path::Path) {
        std::fs::create_dir_all(dest).unwrap();
        for item in std::fs::read_dir(source).unwrap() {
            let item = item.unwrap();
            let target = dest.join(item.file_name());
            if item.file_type().unwrap().is_dir() {
                copy(&item.path(), &target);
            } else {
                std::fs::copy(item.path(), target).unwrap();
            }
        }
    }
    let temp = Temp::new();
    let (mut project, policy, slot) = fixture(&temp);
    let saved = planned(&mut project, policy);
    let rows = selected(&saved);
    project.materialize_narrative_analysis(saved.id, slot, &rows).unwrap();
    let build = build(&mut project, saved.id, slot, &rows).unwrap();
    let records = super::super::narrative_generation::records::RecordSet::load(&project).unwrap();
    let request = records.requests[&build.items[0].request_id.unwrap()].clone();
    let source = project.root().to_path_buf();
    let index = project.index_path();
    drop(project);
    std::fs::remove_file(index).unwrap();
    let copied = temp.0.join("copied.wobu");
    copy(&source, &copied);
    let project = Project::open(&copied).unwrap();
    assert_eq!(project.narrative_analysis_report(saved.id).unwrap().report.included.value, "7");
    assert!(
        wobu_store::project::narrative_generation::checks(&project, &request).unwrap().current()
    );
}
#[test]
fn a_grouped_review_staging_acceptance_later_keeps_the_policy_guard_until_commit() {
    use wobu_narrative::review::{EditorialAction, PolicyScope};
    use wobu_store::project::narrative_review::ReviewRequest;
    let temp = Temp::new();
    let (mut project, policy, slot) = fixture(&temp);
    let view = project.review_scene(policy.target.scene, None).unwrap();
    let line = view.lines.iter().find(|l| l.target.slot == slot).unwrap();
    project
        .apply_review(&ReviewRequest {
            guard: view.guard.clone(),
            target: line.target.clone(),
            context_revision: line.context_revision.clone(),
            state_json: view.state_json.clone(),
            action: EditorialAction::Policy {
                scope: PolicyScope::Slot,
                policy: GenerationPolicy::Edited,
            },
        })
        .unwrap();
    let saved = planned(&mut project, policy.clone());
    let rows = selected(&saved);
    project.materialize_narrative_analysis(saved.id, slot, &rows).unwrap();
    let build = build(&mut project, saved.id, slot, &rows).unwrap();
    let records = super::super::narrative_generation::records::RecordSet::load(&project).unwrap();
    let request = records.requests[&build.items[0].request_id.unwrap()].clone();
    let receipt = success(&request);
    let id = Id::generate();
    super::super::narrative_generation::records::publish(&mut project, &request, id, &receipt)
        .unwrap();
    let state_json = serde_json::to_string(&request.context.options.state).unwrap();
    let view = project.review_scene(policy.target.scene, Some(&state_json)).unwrap();
    let first = &view.lines[0];
    let first_request = ReviewRequest {
        guard: view.guard.clone(),
        target: first.target.clone(),
        context_revision: first.context_revision.clone(),
        state_json: state_json.clone(),
        action: EditorialAction::Policy {
            scope: PolicyScope::Variant,
            policy: GenerationPolicy::Edited,
        },
    };
    let mut tx = project.begin_review(&first_request).unwrap();
    project.stage_review(&mut tx, &first_request).unwrap();
    let target = view.lines.iter().find(|l| l.target.variant == request.target.variant).unwrap();
    let proposal = target.proposals.iter().find(|p| p.id == id).unwrap();
    project
        .stage_review(
            &mut tx,
            &ReviewRequest {
                guard: view.guard,
                state_json,
                target: target.target.clone(),
                context_revision: target.context_revision.clone(),
                action: EditorialAction::Accept {
                    proposal_id: id,
                    proposal_hash: proposal.hash.clone(),
                    reviewed_text: None,
                },
            },
        )
        .unwrap();
    let capture = project.narrative_analysis_capture().unwrap();
    let mut changed = policy;
    changed.external = true;
    project.save_narrative_analysis_policy(changed, &capture).unwrap();
    assert!(project.commit_review(tx).is_err());
    let scene = project.load_scene(request.target.scene).unwrap().scene;
    assert!(scene.beats[0].dialogue[1].variants.iter().all(|v| v.text.body.is_empty()));
}
