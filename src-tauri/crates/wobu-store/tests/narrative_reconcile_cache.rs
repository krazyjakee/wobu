use std::{fs, path::Path};
use wobu_store::{NarrativeRecordDocument, NarrativeRecordKind, Project};

fn fixture() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::create(dir.path(), "Reconciliation").unwrap();
    (dir, project)
}
fn replace_same_size(path: &Path, before: &str, after: &str) {
    assert_eq!(before.len(), after.len());
    let old = fs::metadata(path).unwrap();
    let raw = fs::read_to_string(path).unwrap();
    assert!(raw.contains(before));
    fs::write(path, raw.replace(before, after)).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old.modified().unwrap()))
        .unwrap();
    assert_eq!(fs::metadata(path).unwrap().len(), old.len());
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), old.modified().unwrap());
}
#[test]
fn cached_sources_still_reject_same_size_same_mtime_edits_before_apply() {
    let (_dir, mut project) = fixture();
    let file = project.create_scene("First").unwrap();
    project.reconcile().unwrap();
    let observation = project.reconcile_plan().unwrap().observe().unwrap();
    replace_same_size(&project.root().join(&file.rel), "First", "Other");
    assert!(!observation.revalidate().unwrap());
    assert!(project.reconcile().unwrap());
    assert_eq!(project.load_scene(file.scene.id).unwrap().scene.name, "Other");
}
#[test]
fn every_cached_pass_rechecks_deleted_and_duplicate_scene_membership() {
    let (_dir, mut project) = fixture();
    let file = project.create_scene("First").unwrap();
    project.reconcile().unwrap();
    let path = project.root().join(&file.rel);
    let duplicate = project.root().join("narrative/scenes/duplicate.yaml");
    let observation = project.reconcile_plan().unwrap().observe().unwrap();
    fs::copy(&path, &duplicate).unwrap();
    assert!(!observation.revalidate().unwrap());
    project.reconcile().unwrap();
    assert!(project.load_scene(file.scene.id).is_err());
    let observation = project.reconcile_plan().unwrap().observe().unwrap();
    fs::remove_file(duplicate).unwrap();
    assert!(!observation.revalidate().unwrap());
    project.reconcile().unwrap();
    assert_eq!(project.load_scene(file.scene.id).unwrap().scene.name, "First");
    let observation = project.reconcile_plan().unwrap().observe().unwrap();
    fs::remove_file(path).unwrap();
    assert!(!observation.revalidate().unwrap());
    project.reconcile().unwrap();
    assert!(project.scene_catalog().unwrap().scenes.is_empty());
}
#[test]
fn unchanged_publications_recheck_same_size_edits_to_referenced_receipts() {
    let (_dir, mut project) = fixture();
    let id = wobu_core::new_id();
    let receipt = NarrativeRecordDocument::new(
        NarrativeRecordKind::Receipt,
        wobu_core::new_id(),
        "Receipt",
        serde_json::json!({"word":"First"}),
    );
    project.publish_narrative_records(id, "Publication", &[receipt], None).unwrap();
    project.reconcile().unwrap();
    let manifest = project.narrative_publication(id).unwrap().unwrap().manifest;
    let receipt_path =
        project.root().join(format!("narrative/objects/{}.json", manifest.records[0].hash));
    let observation = project.reconcile_plan().unwrap().observe().unwrap();
    replace_same_size(&receipt_path, "First", "Other");
    assert!(!observation.revalidate().unwrap());
    project.reconcile().unwrap();
    assert!(project.narrative_publication(id).is_err());
    let index = project.narrative_index().unwrap();
    assert!(index.iter().any(|entry| entry.rel == manifest.rel() && entry.error.is_some()));
}
#[test]
fn an_index_refresh_between_observation_and_apply_rejects_the_old_plan() {
    let (_dir, mut project) = fixture();
    let file = project.create_scene("First").unwrap();
    project.reconcile().unwrap();
    let observation = project.reconcile_plan().unwrap().observe().unwrap();
    assert!(observation.revalidate().unwrap());
    replace_same_size(&project.root().join(&file.rel), "First", "Other");
    // Reading the selected scene refreshes its disposable locator/index first.
    assert_eq!(project.load_scene(file.scene.id).unwrap().scene.name, "Other");
    assert_eq!(project.apply_reconcile(observation).unwrap(), None);
    assert_eq!(project.load_scene(file.scene.id).unwrap().scene.name, "Other");
}
