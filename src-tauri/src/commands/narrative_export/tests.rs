use super::*;
use wobu_narrative::{
    Beat, Choice, Destination, DialogueSlot, Speaker, StateDocument, Text, Variant,
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("wobu-export-cmd-{}", wobu_core::new_id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn project(temp: &Temp) -> Project {
    let mut project = Project::create(&temp.0, "Export").unwrap();
    let mut file = project.create_scene("Council").unwrap();
    let mut beat = Beat::new("Evidence");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written("Draft line")));
    beat.dialogue.push(slot);
    beat.choices.push(Choice::new("Continue", Destination::End { label: "done".into() }));
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();
    project
}
#[test]
fn development_exports_saved_snapshot_and_release_reports_blockers_without_writing() {
    let temp = Temp::new();
    let project = project(&temp);
    let before = project.narrative_fingerprint().unwrap();
    let (check, package) = prepare(&project, Profile::Development, BTreeMap::new(), false).unwrap();
    assert!(package.is_some());
    assert_eq!(check.strings, 2);
    let (check, package) = prepare(&project, Profile::Release, BTreeMap::new(), false).unwrap();
    assert!(package.is_none());
    assert!(
        check.diagnostics.iter().any(|d| d.severity == wobu_narrative_compiler::Severity::Error)
    );
    assert_eq!(before, project.narrative_fingerprint().unwrap());
}
#[test]
fn source_changes_during_capture_are_refused() {
    let temp = Temp::new();
    let project = project(&temp);
    let rel = project.scene_catalog().unwrap().scenes[0].rel.clone();
    assert!(
        prepare_checked(&project, Profile::Development, BTreeMap::new(), false, || {
            std::fs::write(project.root().join(rel), "changed").unwrap();
        })
        .is_err()
    );
}
#[test]
fn state_changes_during_capture_are_refused() {
    let temp = Temp::new();
    let mut project = project(&temp);
    project.save_state(&StateDocument::new(vec![]), None).unwrap();
    let path = project.root().join("narrative/state.yaml");
    assert!(
        prepare_checked(&project, Profile::Development, BTreeMap::new(), false, || {
            std::fs::write(path, "schema_version: 1\nvariables: []\n# external edit\n").unwrap();
        })
        .is_err()
    );
}

#[test]
fn supporting_only_project_releases_after_shared_approval_and_stale_context_blocks_it() {
    use wobu_narrative::{Name, SceneId, TextEntry, TextKind, review::EditorialAction};
    use wobu_store::project::narrative_review::ReviewRequest;
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Text only release").unwrap();
    let mut file = project
        .create_text_asset(TextKind::Codex, "Lantern", Name::new("codex_read").unwrap())
        .unwrap();
    let mut entry = TextEntry::new("Description");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants
        .push(Variant::new(Text::written("The repaired lantern guides the fishing boats.")));
    entry.lines.push(slot);
    file.asset.entries.push(entry);
    project.save_text_asset(&mut file).unwrap();
    assert!(prepare(&project, Profile::Development, BTreeMap::new(), false).unwrap().1.is_some());
    assert!(prepare(&project, Profile::Release, BTreeMap::new(), false).unwrap().1.is_none());
    let view = project.review_scene(SceneId::from_raw(file.asset.id.raw()), None).unwrap();
    project
        .apply_review(&ReviewRequest {
            guard: view.guard,
            target: view.lines[0].target.clone(),
            context_revision: view.lines[0].context_revision.clone(),
            state_json: view.state_json,
            action: EditorialAction::Approve,
        })
        .unwrap();
    let first = prepare(&project, Profile::Release, BTreeMap::new(), false).unwrap();
    assert!(first.1.is_some(), "{:?}", first.0.diagnostics);
    assert_eq!(first.0.strings, 1);
    let second = prepare(&project, Profile::Release, BTreeMap::new(), false).unwrap();
    assert_eq!(first.0.payload_hash, second.0.payload_hash);
    file = project.load_text_asset(file.asset.id).unwrap();
    file.asset.summary = "The lantern was extinguished.".into();
    project.save_text_asset(&mut file).unwrap();
    assert!(prepare(&project, Profile::Release, BTreeMap::new(), false).unwrap().1.is_none());
}
