use wobu_narrative::{
    Beat, BeatId, DialogueSlot, Freshness, GenerationPolicy, Name, SceneId, SourceLink, Speaker,
    Text, TextEntry, TextKind, Variant,
    review::{EditorialAction, PolicyScope, ReviewTarget},
};
use wobu_store::{Project, TextFile, project::narrative_review::ReviewRequest};

fn codex(project: &mut Project, links: Vec<SourceLink>) -> (TextFile, ReviewTarget) {
    let mut file = project
        .create_text_asset(TextKind::Codex, "Harbour notes", Name::new("read").unwrap())
        .unwrap();
    file.asset.sources = links;
    let mut entry = TextEntry::new("Display label");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    let variant = Variant::new(Text::written("A lamp above the harbour."));
    let target = ReviewTarget {
        scene: SceneId::from_raw(file.asset.id.raw()),
        beat: BeatId::from_raw(entry.id.raw()),
        slot: slot.id,
        variant: Some(variant.id),
    };
    slot.variants.push(variant);
    entry.lines.push(slot);
    file.asset.entries.push(entry);
    project.save_text_asset(&mut file).unwrap();
    (file, target)
}

fn request(project: &Project, target: &ReviewTarget, action: EditorialAction) -> ReviewRequest {
    let view = project.review_scene(target.scene, None).unwrap();
    let line = view.lines.iter().find(|line| line.target == *target).unwrap();
    ReviewRequest {
        guard: view.guard,
        target: target.clone(),
        context_revision: line.context_revision.clone(),
        state_json: view.state_json,
        action,
    }
}

fn apply(project: &mut Project, target: &ReviewTarget, action: EditorialAction) {
    project.apply_review(&request(project, target, action)).unwrap();
}

#[test]
fn linked_scene_context_preserves_history_and_invalidates_locked_text_precisely() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Linked context").unwrap();
    let mut source = project.create_scene("Harbour").unwrap();
    source.scene.summary = "A lamp marks the harbour entrance.".into();
    let mut beat = Beat::new("A distant conversation");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written("Unrelated dialogue.")));
    beat.dialogue.push(slot);
    source.scene.beats.push(beat);
    project.save_scene(&mut source).unwrap();
    let (file, target) = codex(&mut project, vec![SourceLink::Scene(source.scene.id)]);
    apply(&mut project, &target, EditorialAction::Approve);
    apply(
        &mut project,
        &target,
        EditorialAction::Policy { scope: PolicyScope::Variant, policy: GenerationPolicy::Locked },
    );
    let original = project.load_text_asset(file.asset.id).unwrap();
    let context = project.review_context(&target, None).unwrap();
    assert_eq!(context.version, 2);
    assert!(context.inputs["dependencies"]["target"].get("text_line").is_some());
    assert_eq!(context.inputs["linked_scenes"][0]["summary"], source.scene.summary);

    source.scene.beats[0].dialogue[0].variants[0]
        .text
        .set_body("A corrected unrelated line.", wobu_narrative::Provenance::Human);
    project.save_scene(&mut source).unwrap();
    assert_eq!(project.review_context(&target, None).unwrap().revision, context.revision);
    assert!(project.review_scene(target.scene, None).unwrap().lines[0].approval_valid);

    source.scene.summary = "The harbour lamp has gone dark.".into();
    project.save_scene(&mut source).unwrap();
    let stale = project.load_text_asset(file.asset.id).unwrap();
    assert_eq!(stale.asset.editorial_head, original.asset.editorial_head);
    let old = &original.asset.entries[0].lines[0].variants[0].text;
    let text = &stale.asset.entries[0].lines[0].variants[0].text;
    assert_eq!(text.body, old.body);
    assert_eq!(text.revision, old.revision);
    assert_eq!(text.lifecycle.policy, GenerationPolicy::Locked);
    assert_eq!(text.lifecycle.freshness, Freshness::OutOfDate);
    let view = project.review_scene(target.scene, None).unwrap();
    assert!(!view.lines[0].approval_valid);
    assert_eq!(view.lines[0].reason, "Authored context changed since the last review.");
    assert!(
        project.review_snapshot(target.scene, None).unwrap().text_evidence().unwrap()
            [&target.variant.unwrap()]
            .binding
            .approved
    );
    apply(&mut project, &target, EditorialAction::Attest);
    assert!(project.review_scene(target.scene, None).unwrap().lines[0].approval_valid);
    assert_eq!(
        project.load_text_asset(file.asset.id).unwrap().asset.entries[0].lines[0].variants[0]
            .text
            .lifecycle
            .freshness,
        Freshness::Current
    );
}

#[test]
fn supporting_text_edit_and_undo_reload_the_canonical_asset() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Text undo").unwrap();
    let (mut file, target) = codex(&mut project, vec![]);
    let before = file.asset.editorial_scene();
    file.asset.entries[0].lines[0].variants[0]
        .text
        .set_body("A changed line.", wobu_narrative::Provenance::Human);
    project.save_text_asset(&mut file).unwrap();
    let expected = file.asset.editorial_scene();
    let restored = project.restore_scene(before, Some(&expected), "harbour-notes").unwrap();
    assert!(restored.rel.starts_with("narrative/texts/"));
    assert_eq!(restored.scene.id, target.scene);
    assert_eq!(
        project.load_text_asset(file.asset.id).unwrap().asset.entries[0].lines[0].variants[0]
            .text
            .body,
        "A lamp above the harbour."
    );
    assert!(project.scene_catalog().unwrap().scenes.is_empty());
    assert!(project.narrative_affected().unwrap().is_empty());
}

#[test]
fn text_freshness_publication_respects_the_shared_editorial_lock() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Text lock").unwrap();
    let mut source = project.create_scene("Harbour").unwrap();
    source.scene.summary = "A working lamp.".into();
    project.save_scene(&mut source).unwrap();
    let (file, target) = codex(&mut project, vec![SourceLink::Scene(source.scene.id)]);
    let tx = project.begin_review(&request(&project, &target, EditorialAction::Approve)).unwrap();
    source.scene.summary = "A broken lamp.".into();
    assert!(project.save_scene(&mut source).unwrap_err().to_string().contains("Another writer"));
    assert_eq!(project.load_text_asset(file.asset.id).unwrap().asset, file.asset);
    drop(tx);
    project.refresh_narrative_dependencies().unwrap();
    assert_eq!(
        project.load_text_asset(file.asset.id).unwrap().asset.entries[0].lines[0].variants[0]
            .text
            .lifecycle
            .freshness,
        Freshness::OutOfDate
    );
}
