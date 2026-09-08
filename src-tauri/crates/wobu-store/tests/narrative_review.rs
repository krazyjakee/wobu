use wobu_narrative::{
    Beat, DialogueSlot, Freshness, GenerationPolicy, ReviewState, Scene, Speaker, Text, Variant,
    review::{EditorialAction, PolicyScope, ReviewTarget},
};
use wobu_store::{Project, SceneFile, SourceSave, project::narrative_review::ReviewRequest};
fn fixture() -> (tempfile::TempDir, Project, SceneFile, ReviewTarget) {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::create(dir.path(), "Review").unwrap();
    let mut file = p.create_scene("Arrival").unwrap();
    let mut beat = Beat::new("Greet");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    let variant = Variant::new(Text::written("Welcome."));
    let target = ReviewTarget {
        scene: file.scene.id,
        beat: beat.id,
        slot: slot.id,
        variant: Some(variant.id),
    };
    slot.variants.push(variant);
    beat.dialogue.push(slot);
    file.scene.beats.push(beat);
    assert!(matches!(p.save_scene(&mut file).unwrap(), SourceSave::Saved(_)));
    (dir, p, file, target)
}
fn request(p: &Project, target: &ReviewTarget, action: EditorialAction) -> ReviewRequest {
    let view = p.review_scene(target.scene, None).unwrap();
    let line = view.lines.iter().find(|l| l.target == *target).unwrap();
    ReviewRequest {
        guard: view.guard,
        target: target.clone(),
        context_revision: line.context_revision.clone(),
        state_json: view.state_json,
        action,
    }
}
fn apply(p: &mut Project, t: &ReviewTarget, a: EditorialAction) -> SceneFile {
    let r = request(p, t, a);
    p.apply_review(&r).unwrap().0
}
#[test]
fn manual_approval_is_bound_and_lock_does_not_invalidate_it() {
    let (_d, mut p, _, t) = fixture();
    let approved = apply(&mut p, &t, EditorialAction::Approve);
    assert!(p.review_scene(t.scene, None).unwrap().lines[0].approval_valid);
    for scope in [PolicyScope::Slot, PolicyScope::Variant] {
        apply(&mut p, &t, EditorialAction::Policy { scope, policy: GenerationPolicy::Locked });
        assert!(p.review_scene(t.scene, None).unwrap().lines[0].approval_valid);
    }
    assert_eq!(
        approved.scene.beats[0].dialogue[0].variants[0].text.lifecycle.review,
        ReviewState::Approved
    );
    assert!(
        p.review_snapshot(t.scene, None).unwrap().evidence().unwrap()[&t.variant.unwrap()]
            .binding
            .approved
    );
}
#[test]
fn upstream_edit_keeps_locked_words_stale_until_explicit_attestation() {
    let (_d, mut p, _, t) = fixture();
    apply(&mut p, &t, EditorialAction::Approve);
    let mut file = apply(
        &mut p,
        &t,
        EditorialAction::Policy { scope: PolicyScope::Variant, policy: GenerationPolicy::Locked },
    );
    let text = file.scene.beats[0].dialogue[0].variants[0].text.clone();
    file.scene.summary = "Different authored context".into();
    p.save_scene(&mut file).unwrap();
    let view = p.review_scene(t.scene, None).unwrap();
    assert_eq!(view.lines[0].freshness, Freshness::OutOfDate);
    assert!(!view.lines[0].approval_valid);
    assert_eq!(view.lines[0].text.as_ref(), Some(&text));
    apply(&mut p, &t, EditorialAction::Attest);
    assert!(p.review_scene(t.scene, None).unwrap().lines[0].approval_valid);
    assert_eq!(p.review_scene(t.scene, None).unwrap().history[0].action, "attest");
}
#[test]
fn lost_reply_retry_and_second_writer_cannot_repeat_the_decision() {
    let (_d, mut p, _, t) = fixture();
    let r = request(&p, &t, EditorialAction::Approve);
    p.apply_review(&r).unwrap();
    assert!(p.apply_review(&r).is_err());
    let mut other = Project::open(p.root()).unwrap();
    assert!(other.apply_review(&r).is_err());
    other.rebuild_index().unwrap();
    assert!(other.review_scene(t.scene, None).unwrap().lines[0].approval_valid);
}
#[test]
fn raw_flags_head_and_undo_cannot_forge_or_revive_approval() {
    let (_d, mut p, mut file, t) = fixture();
    file.scene.beats[0].dialogue[0].variants[0].text.lifecycle.review = ReviewState::Approved;
    assert!(p.save_scene(&mut file).is_err());
    let approved = apply(&mut p, &t, EditorialAction::Approve);
    let edited = apply(&mut p, &t, EditorialAction::Edit { body: "Changed.".into() });
    let restored =
        p.restore_scene(approved.scene.clone(), Some(&edited.scene), approved.slug()).unwrap();
    assert!(!p.review_scene(t.scene, None).unwrap().lines[0].approval_valid);
    assert!(p.restore_scene(approved.scene.clone(), Some(&edited.scene), approved.slug()).is_err());
    let mut forged = restored;
    forged.scene.editorial_head = approved.scene.editorial_head;
    assert!(p.save_scene(&mut forged).is_err());
}
#[test]
fn duplicate_locked_line_keeps_protection_but_needs_own_review() {
    let (_d, mut p, _, t) = fixture();
    apply(&mut p, &t, EditorialAction::Approve);
    let file = apply(
        &mut p,
        &t,
        EditorialAction::Policy { scope: PolicyScope::Variant, policy: GenerationPolicy::Locked },
    );
    let copy = file.scene.duplicated();
    assert!(copy.editorial_head.is_none());
    assert_eq!(copy.beats[0].dialogue[0].variants[0].text.lifecycle.review, ReviewState::Draft);
    assert_eq!(
        copy.beats[0].dialogue[0].variants[0].text.lifecycle.policy,
        GenerationPolicy::Locked
    );
}
#[test]
fn lock_race_blocks_text_and_raw_unlock() {
    let (_d, mut p, mut file, t) = fixture();
    let edit = request(&p, &t, EditorialAction::Edit { body: "Race".into() });
    apply(
        &mut p,
        &t,
        EditorialAction::Policy { scope: PolicyScope::Slot, policy: GenerationPolicy::Locked },
    );
    assert!(p.apply_review(&edit).is_err());
    file = p.load_scene(file.scene.id).unwrap();
    file.scene.beats[0].dialogue[0].policy = GenerationPolicy::Edited;
    assert!(p.save_scene(&mut file).is_err());
}
#[test]
fn missing_receipt_and_external_edit_fail_closed_then_record_manual_change() {
    let (_d, mut p, _, t) = fixture();
    let mut approved = apply(&mut p, &t, EditorialAction::Approve);
    approved.scene.summary = "External edit".into();
    std::fs::write(
        p.root().join(&approved.rel),
        wobu_narrative::SceneDocument::new(approved.scene.clone()).to_yaml().unwrap(),
    )
    .unwrap();
    assert!(!p.review_scene(t.scene, None).unwrap().lines[0].approval_valid);
    let mut current = p.load_scene(t.scene).unwrap();
    p.save_scene(&mut current).unwrap();
    apply(&mut p, &t, EditorialAction::Approve);
    let file = p.load_scene(t.scene).unwrap();
    std::fs::remove_file(
        p.root().join(format!("narrative/receipts/{}.json", file.scene.editorial_head.unwrap())),
    )
    .unwrap();
    assert!(!p.review_scene(t.scene, None).unwrap().lines[0].approval_valid);
}
#[test]
fn grouped_actions_use_original_guard_and_single_publication() {
    let (_d, mut p, _, t) = fixture();
    let first = request(&p, &t, EditorialAction::Approve);
    let second = request(
        &p,
        &t,
        EditorialAction::Policy { scope: PolicyScope::Variant, policy: GenerationPolicy::Locked },
    );
    let mut tx = p.begin_review(&first).unwrap();
    p.stage_review(&mut tx, &first).unwrap();
    p.stage_review(&mut tx, &second).unwrap();
    p.commit_review(tx).unwrap();
    assert!(p.review_scene(t.scene, None).unwrap().lines[0].approval_valid);
}
#[test]
fn dropped_transaction_publishes_no_history_or_words() {
    let (_d, p, file, t) = fixture();
    let r = request(&p, &t, EditorialAction::Approve);
    let mut tx = p.begin_review(&r).unwrap();
    p.stage_review(&mut tx, &r).unwrap();
    drop(tx);
    assert_eq!(p.load_scene(t.scene).unwrap().scene, file.scene);
}
#[test]
fn missing_character_and_invalid_state_cannot_be_approved() {
    let (_d, mut p, mut file, t) = fixture();
    file.scene.beats[0].dialogue[0].speaker = Speaker::Entity(wobu_core::Id::generate());
    p.save_scene(&mut file).unwrap();
    let r = request(&p, &t, EditorialAction::Approve);
    assert!(p.apply_review(&r).is_err());
    assert!(p.review_scene(t.scene, Some("{\"undeclared\":true}")).is_err());
}
#[test]
fn semantic_projection_ignores_flags_but_not_other_words_or_identity() {
    let (_d, p, file, t) = fixture();
    let context = p.review_context(&t, None).unwrap();
    let mut scene = file.scene.clone();
    scene.editorial_head = None;
    scene.beats[0].dialogue[0].policy = GenerationPolicy::Locked;
    scene.beats[0].dialogue[0].variants[0].text.set_body("New", wobu_narrative::Provenance::Human);
    let capture = |scene: &Scene| {
        wobu_narrative::review::ReviewContext::capture(
            scene,
            &t,
            context.inputs["world"].clone(),
            context.inputs["schema"].clone(),
            context.inputs["characters"].clone(),
            context.state.clone(),
        )
    };
    assert_eq!(capture(&scene).revision, context.revision);
    scene.beats[0].title = "Changed intent".into();
    assert_ne!(capture(&scene).revision, context.revision);
}

#[test]
fn upstream_race_between_review_and_commit_preserves_text_and_head() {
    let (_d, mut p, file, t) = fixture();
    let r = request(&p, &t, EditorialAction::Approve);
    let mut tx = p.begin_review(&r).unwrap();
    p.stage_review(&mut tx, &r).unwrap();
    let mut world = wobu_narrative::WorldDocument::default();
    world.facts.push(wobu_narrative::Fact {
        id: wobu_core::Id::generate(),
        name: "New fact".into(),
        assertion: "A witness arrived".into(),
        sources: vec![],
        entity_ids: vec![],
    });
    std::fs::write(p.root().join("narrative/world.yaml"), world.to_yaml().unwrap()).unwrap();
    assert!(p.commit_review(tx).is_err());
    assert_eq!(p.load_scene(t.scene).unwrap().scene, file.scene);
}
#[test]
fn interrupted_unreachable_receipt_cannot_approve_and_discontinuous_history_fails_closed() {
    use wobu_narrative::review::EditorialEvent;
    use wobu_store::{NarrativeRecordDocument, NarrativeRecordFile, NarrativeRecordKind};
    let (_d, mut p, _, t) = fixture();
    let file = apply(&mut p, &t, EditorialAction::Approve);
    let head = file.scene.editorial_head.unwrap();
    let record = p.narrative_record(NarrativeRecordKind::Receipt, head).unwrap().unwrap();
    let mut event: EditorialEvent = serde_json::from_value(record.document.payload).unwrap();
    event.id = wobu_core::Id::generate();
    event.parent = Some(head);
    event.before = file.scene.clone();
    event.before.summary = "Forged continuity".into();
    event.bindings.get_mut(&t.variant.unwrap()).unwrap().event_id = event.id;
    let mut orphan = NarrativeRecordFile {
        document: NarrativeRecordDocument::new(
            NarrativeRecordKind::Receipt,
            event.id,
            "Interrupted decision",
            serde_json::to_value(&event).unwrap(),
        ),
        stamp: None,
    };
    p.save_narrative_record(&mut orphan).unwrap();
    assert_eq!(p.review_scene(t.scene, None).unwrap().guard.head, Some(head));
    let mut switched = file.scene;
    switched.editorial_head = Some(event.id);
    std::fs::write(
        p.root().join(&file.rel),
        wobu_narrative::SceneDocument::new(switched).to_yaml().unwrap(),
    )
    .unwrap();
    let view = p.review_scene(t.scene, None).unwrap();
    assert!(!view.lines[0].approval_valid);
    assert!(view.lines[0].reason.contains("discontinuity"));
}
#[test]
fn invalid_world_restriction_cannot_receive_an_attestation() {
    let (_d, mut p, _, t) = fixture();
    apply(&mut p, &t, EditorialAction::Approve);
    let mut world = wobu_narrative::WorldDocument::default();
    world.restrictions.push(wobu_narrative::FutureRestriction {
        id: wobu_core::Id::generate(),
        name: "Hidden event".into(),
        fact: wobu_core::Id::generate(),
        characters: vec![],
        until: wobu_narrative::Condition::Never,
    });
    std::fs::write(p.root().join("narrative/world.yaml"), world.to_yaml().unwrap()).unwrap();
    let r = request(&p, &t, EditorialAction::Attest);
    assert!(p.apply_review(&r).is_err());
    assert!(p.review_snapshot(t.scene, None).unwrap().evidence().unwrap().is_empty());
}
#[test]
fn locked_slots_reject_insertions_and_locked_wording_rejects_speaker_changes() {
    let (_d, mut p, _, t) = fixture();
    let mut file = apply(
        &mut p,
        &t,
        EditorialAction::Policy { scope: PolicyScope::Slot, policy: GenerationPolicy::Locked },
    );
    file.scene.beats[0].dialogue[0].variants.push(Variant::new(Text::written("Cannot insert")));
    assert!(p.save_scene(&mut file).is_err());
    apply(
        &mut p,
        &t,
        EditorialAction::Policy { scope: PolicyScope::Slot, policy: GenerationPolicy::Edited },
    );
    let mut file = apply(
        &mut p,
        &t,
        EditorialAction::Policy { scope: PolicyScope::Variant, policy: GenerationPolicy::Locked },
    );
    file.scene.beats[0].dialogue[0].speaker = Speaker::Player;
    assert!(p.save_scene(&mut file).is_err());
}

#[test]
fn a_second_cooperating_writer_cannot_enter_a_live_review_transaction() {
    let (_d, p, _, t) = fixture();
    let request = request(&p, &t, EditorialAction::Approve);
    let tx = p.begin_review(&request).unwrap();
    let peer = Project::open(p.root()).unwrap();
    assert!(peer.begin_review(&request).is_err());
    drop(tx);
    assert!(peer.begin_review(&request).is_ok());
}

#[test]
fn handwritten_drafts_have_independent_current_context_and_keep_copy_provenance() {
    let (_d, mut p, mut file, t) = fixture();
    let view = p.review_scene(t.scene, None).unwrap();
    assert_eq!(view.lines[0].freshness, Freshness::Current);
    assert!(!view.lines[0].approval_valid);
    file.scene.summary = "Upstream changed".into();
    p.save_scene(&mut file).unwrap();
    let view = p.review_scene(t.scene, None).unwrap();
    assert_eq!(view.lines[0].freshness, Freshness::OutOfDate);
    assert!(!view.lines[0].approval_valid);
    let mut copy = file.scene.beats[0].duplicated();
    let original = copy.dialogue[0].variants[0].text.clone();
    copy.title = "New copied beat".into();
    file.scene.beats.push(copy);
    p.save_scene(&mut file).unwrap();
    let copy = &file.scene.beats[1].dialogue[0].variants[0];
    assert_eq!(copy.text.provenance, original.provenance);
    assert_eq!(copy.text.revision, original.revision);
    let view = p.review_scene(t.scene, None).unwrap();
    assert_eq!(view.lines[1].freshness, Freshness::Current);
    assert_eq!(view.lines[0].freshness, Freshness::OutOfDate);
    assert!(view.lines.iter().all(|l| !l.approval_valid));
}
