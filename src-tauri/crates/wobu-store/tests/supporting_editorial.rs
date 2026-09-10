use wobu_narrative::{
    DialogueSlot, GenerationPolicy, Name, SceneId, Speaker, Text, TextEntry, TextKind, Variant,
    review::{EditorialAction, PolicyScope, ReviewTarget, TextTarget},
};
use wobu_store::{Project, project::narrative_review::ReviewRequest};

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

#[test]
fn all_six_types_use_shared_review_and_release_evidence_after_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Supporting review").unwrap();
    for kind in TextKind::ALL {
        let mut file =
            project.create_text_asset(kind, kind.noun(), Name::new("host_event").unwrap()).unwrap();
        let mut entry = TextEntry::new("Arrival");
        let mut slot = DialogueSlot::new(Speaker::Narrator);
        slot.variants.push(Variant::new(Text::written("The lantern remains lit.")));
        entry.lines.push(slot);
        file.asset.entries.push(entry);
        project.save_text_asset(&mut file).unwrap();
        let target = ReviewTarget {
            scene: SceneId::from_raw(file.asset.id.raw()),
            beat: wobu_narrative::BeatId::from_raw(file.asset.entries[0].id.raw()),
            slot: file.asset.entries[0].lines[0].id,
            variant: Some(file.asset.entries[0].lines[0].variants[0].id),
        };
        let approve = request(&project, &target, EditorialAction::Approve);
        project.apply_review(&approve).unwrap();
        assert!(project.apply_review(&approve).is_err(), "a stale decision must conflict");
        project.rebuild_index().unwrap();
        let reopened = Project::open(project.root()).unwrap();
        let snapshot = reopened.review_snapshot(target.scene, None).unwrap();
        let file = reopened.load_text_asset(file.asset.id).unwrap();
        let proof = &snapshot.text_evidence().unwrap()[&target.variant.unwrap()];
        assert!(proof.verifies(
            &TextTarget {
                asset: file.asset.id,
                entry: file.asset.entries[0].id,
                slot: target.slot,
                variant: target.variant
            },
            &Speaker::Narrator,
            &file.asset.entries[0].lines[0].variants[0].text
        ));
        assert!(
            reopened.scene_catalog().unwrap().scenes.is_empty(),
            "editorial adapters never create fake scenes"
        );
        assert!(
            !std::fs::read_to_string(project.root().join(file.rel))
                .unwrap()
                .contains("supporting_text:")
        );
    }
}

#[test]
fn text_locks_context_changes_and_forged_flags_obey_shared_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Text locks").unwrap();
    let mut file = project
        .create_text_asset(TextKind::Codex, "Lantern", Name::new("read_codex").unwrap())
        .unwrap();
    let mut entry = TextEntry::new("Description");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written("A glass lantern.")));
    entry.lines.push(slot);
    file.asset.entries.push(entry);
    project.save_text_asset(&mut file).unwrap();
    let target = ReviewTarget {
        scene: SceneId::from_raw(file.asset.id.raw()),
        beat: wobu_narrative::BeatId::from_raw(file.asset.entries[0].id.raw()),
        slot: file.asset.entries[0].lines[0].id,
        variant: Some(file.asset.entries[0].lines[0].variants[0].id),
    };
    let mut forged = file.clone();
    forged.asset.entries[0].lines[0].variants[0].text.lifecycle.review =
        wobu_narrative::ReviewState::Approved;
    assert!(project.save_text_asset(&mut forged).is_err());
    project.apply_review(&request(&project, &target, EditorialAction::Approve)).unwrap();
    project
        .apply_review(&request(
            &project,
            &target,
            EditorialAction::Policy {
                scope: PolicyScope::Variant,
                policy: GenerationPolicy::Locked,
            },
        ))
        .unwrap();
    assert!(
        project
            .apply_review(&request(
                &project,
                &target,
                EditorialAction::Edit { body: "Overwrite".into() }
            ))
            .is_err()
    );
    file = project.load_text_asset(file.asset.id).unwrap();
    file.asset.summary = "The lantern is broken now.".into();
    project.save_text_asset(&mut file).unwrap();
    let view = project.review_scene(target.scene, None).unwrap();
    assert!(!view.lines[0].approval_valid);
    assert_eq!(view.lines[0].freshness, wobu_narrative::Freshness::OutOfDate);
    project.apply_review(&request(&project, &target, EditorialAction::Attest)).unwrap();
    assert!(project.review_scene(target.scene, None).unwrap().lines[0].approval_valid);
    file = project.load_text_asset(file.asset.id).unwrap();
    file.asset.entries.clear();
    assert!(
        project.save_text_asset(&mut file).is_err(),
        "protected lines cannot disappear via whole-document save"
    );
}

#[test]
fn stale_text_save_parks_canonical_asset_and_cross_domain_collision_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Concurrent text").unwrap();
    let mut file = project
        .create_text_asset(TextKind::Codex, "Lamp", Name::new("read_lamp").unwrap())
        .unwrap();
    let mut stale = file.clone();
    file.asset.summary = "Fresh writer".into();
    project.save_text_asset(&mut file).unwrap();
    stale.asset.summary = "Old writer".into();
    let wobu_store::SourceSave::Conflict { conflict_path } =
        project.save_text_asset(&mut stale).unwrap()
    else {
        panic!("stale save must conflict")
    };
    assert_eq!(project.load_text_asset(file.asset.id).unwrap().asset.summary, "Fresh writer");
    let text = std::fs::read_to_string(project.root().join(conflict_path)).unwrap();
    assert_eq!(
        wobu_narrative::TextAssetDocument::parse(&text).unwrap().asset.summary,
        "Old writer"
    );
    let mut scene = wobu_narrative::Scene::new("Collision");
    scene.id = SceneId::from_raw(file.asset.id.raw());
    let path = project.root().join("narrative/scenes/collision.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, wobu_narrative::SceneDocument::new(scene).to_yaml().unwrap()).unwrap();
    assert!(project.load_editorial_source(SceneId::from_raw(file.asset.id.raw())).is_err());
}
