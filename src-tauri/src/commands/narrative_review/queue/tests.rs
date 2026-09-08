use super::*;
use wobu_narrative::{Beat, DialogueSlot, Speaker, Text, Variant, review::EditorialAction};
struct TestProject(Project);
impl TestProject {
    fn new() -> Self {
        Self(
            Project::create(
                &std::env::temp_dir().join(format!("wobu-review-{}", wobu_core::new_id())),
                "Review",
            )
            .unwrap(),
        )
    }
}
impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.root());
    }
}
fn fixture(project: &mut Project, name: &str) -> Vec<ReviewRequest> {
    let mut file = project.create_scene(name).unwrap();
    let mut beat = Beat::new("Arrival");
    for body in ["The beacon failed.", "Then we start again."] {
        let mut slot = DialogueSlot::new(Speaker::Narrator);
        slot.variants.push(Variant::new(Text::written(body)));
        beat.dialogue.push(slot);
    }
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();
    let view = project.review_scene(file.scene.id, None).unwrap();
    view.lines
        .into_iter()
        .map(|line| ReviewRequest {
            guard: view.guard.clone(),
            target: line.target,
            context_revision: line.context_revision,
            state_json: view.state_json.clone(),
            action: EditorialAction::Approve,
        })
        .collect()
}
#[test]
fn dry_run_preserves_canonical_data_then_grouped_original_guards_commit_and_cannot_repeat() {
    let mut holder = TestProject::new();
    let project = &mut holder.0;
    let requests = fixture(project, "First");
    let before = project.narrative_fingerprint().unwrap();
    let initial_history =
        project.review_scene(requests[0].target.scene, None).unwrap().history.len();
    let preview = batch(project, &requests, false).unwrap();
    assert!(preview.items.iter().all(|item| item.status == "eligible"));
    assert_eq!(before, project.narrative_fingerprint().unwrap());
    assert_eq!(
        project.review_scene(requests[0].target.scene, None).unwrap().history.len(),
        initial_history
    );
    let result = batch(project, &requests, true).unwrap();
    assert!(result.items.iter().all(|item| item.status == "applied"));
    let view = project.review_scene(requests[0].target.scene, None).unwrap();
    assert!(view.lines.iter().all(|line| line.approval_valid));
    assert_eq!(view.history.len(), initial_history + 2);
    let head = view.guard.head;
    assert!(
        batch(project, &requests, true)
            .unwrap()
            .items
            .iter()
            .all(|item| item.status == "conflicting")
    );
    assert_eq!(head, project.review_scene(requests[0].target.scene, None).unwrap().guard.head);
}
#[test]
fn one_conflicting_scene_does_not_hide_other_scene_results_and_duplicates_are_skipped() {
    let mut holder = TestProject::new();
    let project = &mut holder.0;
    let first = fixture(project, "First");
    let second = fixture(project, "Second");
    let mut changed = project.load_scene(first[0].target.scene).unwrap();
    changed.scene.summary = "A peer changed the scene.".into();
    project.save_scene(&mut changed).unwrap();
    let requests = vec![first[0].clone(), second[0].clone(), second[0].clone()];
    let result = batch(project, &requests, true).unwrap();
    assert_eq!(
        result.items.iter().map(|item| item.status).collect::<Vec<_>>(),
        vec!["conflicting", "applied", "skipped"]
    );
    assert!(!project.review_scene(first[0].target.scene, None).unwrap().lines[0].approval_valid);
}
#[test]
fn bounded_listing_discloses_unreadable_sources_and_pages_without_omitting_valid_scenes() {
    let mut holder = TestProject::new();
    let project = &mut holder.0;
    for index in 0..33 {
        project.create_scene(&format!("Scene {index}")).unwrap();
    }
    std::fs::write(project.root().join("narrative/scenes/broken.scene.yaml"), "not: [valid")
        .unwrap();
    let page = list(project, None, 0, None).unwrap();
    assert_eq!(page.scenes.len(), 32);
    assert_eq!(page.total_scenes, 33);
    assert_eq!(page.next_offset, Some(32));
    assert!(!page.errors.is_empty());
    let last = list(project, None, 32, Some(&page.catalog_revision)).unwrap();
    assert_eq!(last.scenes.len(), 1);
    assert_eq!(last.next_offset, None);
    project.create_scene("A new peer scene").unwrap();
    assert!(list(project, None, 32, Some(&page.catalog_revision)).is_err());
    assert!(!page.scenes.iter().any(|scene| scene.scene_id == last.scenes[0].scene_id));
}

#[test]
fn supporting_text_is_listed_and_approved_in_the_project_wide_queue() {
    let mut holder = TestProject::new();
    let project = &mut holder.0;
    let mut file = project
        .create_text_asset(
            wobu_narrative::TextKind::Journal,
            "Tide diary",
            wobu_narrative::Name::new("evening").unwrap(),
        )
        .unwrap();
    let mut entry = wobu_narrative::TextEntry::new("Day one");
    let mut slot = DialogueSlot::new(Speaker::Player);
    slot.variants.push(Variant::new(Text::written("I fixed the lamp before sunset.")));
    entry.lines.push(slot);
    file.asset.entries.push(entry);
    project.save_text_asset(&mut file).unwrap();
    let listed = list(project, None, 0, None).unwrap();
    assert_eq!(listed.total_scenes, 1);
    assert!(listed.errors.is_empty());
    let view = &listed.scenes[0];
    assert_eq!(view.scene_id.raw(), file.asset.id.raw());
    let request = ReviewRequest {
        guard: view.guard.clone(),
        target: view.lines[0].target.clone(),
        context_revision: view.lines[0].context_revision.clone(),
        state_json: view.state_json.clone(),
        action: EditorialAction::Approve,
    };
    assert_eq!(batch(project, &[request], true).unwrap().items[0].status, "applied");
    assert!(list(project, None, 0, None).unwrap().scenes[0].lines[0].approval_valid);
}
