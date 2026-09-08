use super::*;
use wobu_narrative::{Beat, DialogueSlot, Speaker, Text, Variant};

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("wobu-source-cmd-{}", wobu_core::new_id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn raw_read_and_explicit_format_preserve_unicode_and_ids_but_normalise_comments() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Source").unwrap();
    let mut file = project.create_scene("Council").unwrap();
    let mut beat = Beat::new("Evidence");
    let mut slot = DialogueSlot::new(Speaker::Player);
    slot.variants.push(Variant::new(Text::written("Je l’ai vu.\n星が見える。")));
    beat.dialogue.push(slot);
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();
    let path = project.root().join(&file.rel);
    let yaml = format!("# Writer's note\n{}", std::fs::read_to_string(&path).unwrap());
    std::fs::write(&path, &yaml).unwrap();
    let source = source_get(&project, file.scene.id).unwrap();
    assert_eq!(source.yaml, yaml);
    assert_eq!(source.file.as_ref().unwrap().scene, file.scene);
    assert_eq!(
        source.file.as_ref().unwrap().stamp,
        atomic::read_stamped(&path).unwrap().map(|(_, stamp)| stamp)
    );
    let check = source_check(&project, file.scene.id, &source.yaml).unwrap();
    assert!(check.problem.is_none());
    assert!(!check.formatted.as_ref().unwrap().contains("Writer's note"));
    assert_eq!(SceneDocument::parse(&check.formatted.unwrap()).unwrap().scene, file.scene);
    assert_eq!(std::fs::read_to_string(path).unwrap(), yaml, "checking never writes");
}

#[test]
fn invalid_unknown_and_future_source_never_get_a_saveable_scene() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Source").unwrap();
    let file = project.create_scene("Council").unwrap();
    let yaml = source_get(&project, file.scene.id).unwrap().yaml;
    let cases = [
        "schema_version: 1\nscene: [".to_string(),
        format!("{yaml}\nunknown: true\n"),
        yaml.replace("schema_version: 2", "schema_version: 999"),
        yaml.replace(&file.scene.id.to_string(), &SceneId::new().to_string()),
    ];
    for invalid in cases {
        let check = source_check(&project, file.scene.id, &invalid).unwrap();
        assert!(check.scene.is_none());
        assert!(check.formatted.is_none());
        assert!(check.problem.is_some());
    }
    let broken = source_check(&project, file.scene.id, "schema_version: 1\nscene: [").unwrap();
    assert!(broken.problem.unwrap().location.is_some());
    assert_eq!(source_get(&project, file.scene.id).unwrap().yaml, yaml);
}

#[test]
fn malformed_disk_source_is_readable_repairable_and_backed_up_without_touching_layout_or_artifacts()
{
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Repair").unwrap();
    let original = project.create_scene("Council").unwrap();
    let path = project.root().join(&original.rel);
    let valid = std::fs::read_to_string(&path).unwrap();
    let broken = format!("# Preserve me\n{valid}unknown_field: [\n");
    std::fs::write(&path, &broken).unwrap();
    let layout = project.root().join("narrative/layout/proof.json");
    std::fs::create_dir_all(layout.parent().unwrap()).unwrap();
    std::fs::write(&layout, "unchanged layout").unwrap();
    let artifact = project.root().join("compiled-story.json");
    std::fs::write(&artifact, "last valid artifact").unwrap();
    let raw = source_at(&project, &original.rel, None).unwrap();
    assert!(raw.file.is_none());
    assert!(raw.problem.is_some());
    assert_eq!(raw.yaml, broken);
    assert!(
        source_repair(&mut project, &original.rel, "schema_version: 1\nscene: [", &raw.stamp, None)
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    let repaired = source_repair(&mut project, &original.rel, &valid, &raw.stamp, None).unwrap();
    assert_eq!(repaired.source.file.unwrap().scene, original.scene);
    assert_eq!(
        std::fs::read_to_string(project.root().join(repaired.recovery_rel)).unwrap(),
        broken
    );
    assert_eq!(std::fs::read_to_string(layout).unwrap(), "unchanged layout");
    assert_eq!(std::fs::read_to_string(artifact).unwrap(), "last valid artifact");
    assert_eq!(project.scene_catalog().unwrap().scenes.len(), 1);
}

#[test]
fn repair_keeps_concurrent_winner_and_rejects_identity_collisions_and_future_versions() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Repair").unwrap();
    let original = project.create_scene("Council").unwrap();
    let path = project.root().join(&original.rel);
    let valid = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, "schema_version: 1\nscene: [").unwrap();
    let raw = source_at(&project, &original.rel, None).unwrap();
    let other = project.create_scene("Other").unwrap();
    let collision = valid.replace(&original.scene.id.to_string(), &other.scene.id.to_string());
    assert!(source_repair(&mut project, &original.rel, &collision, &raw.stamp, None).is_err());
    std::fs::write(&path, "schema_version: 1\nscene: [ # colleague's draft").unwrap();
    let error = source_repair(&mut project, &original.rel, &valid, &raw.stamp, None).unwrap_err();
    assert_eq!(error.code, Code::Conflict);
    assert!(std::fs::read_to_string(&path).unwrap().contains("colleague"));
    let future = valid.replace("schema_version: 2", "schema_version: 999");
    std::fs::write(&path, &future).unwrap();
    let future_source = source_at(&project, &original.rel, None).unwrap();
    assert!(
        source_repair(&mut project, &original.rel, &valid, &future_source.stamp, None).is_err()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), future);
    assert!(source_at(&project, "narrative/scenes/../../project.json", None).is_err());
    assert!(source_at(&project, "/tmp/other.yaml", None).is_err());
}

#[cfg(unix)]
#[test]
fn repair_refuses_leaf_and_ancestor_symlinks_even_inside_project() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Repair").unwrap();
    let original = project.create_scene("Council").unwrap();
    let path = project.root().join(&original.rel);
    let moved = project.root().join("saved.yaml");
    std::fs::rename(&path, &moved).unwrap();
    symlink(&moved, &path).unwrap();
    assert!(source_at(&project, &original.rel, None).is_err());
    std::fs::remove_file(&path).unwrap();
    let scenes = project.root().join("narrative/scenes");
    let other = project.root().join("other-scenes");
    std::fs::rename(&scenes, &other).unwrap();
    std::fs::rename(&moved, other.join("council.yaml")).unwrap();
    symlink(&other, &scenes).unwrap();
    assert!(source_at(&project, &original.rel, None).is_err());
}

#[test]
fn source_round_trip_keeps_form_edits_and_guarded_concurrent_writes() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Source").unwrap();
    let original = project.create_scene("Council").unwrap();
    let raw = source_get(&project, original.scene.id).unwrap();
    let checked =
        source_check(&project, original.scene.id, &raw.yaml.replace("Council", "Source rename"))
            .unwrap();
    let mut form = project.load_scene(original.scene.id).unwrap();
    form.scene.name = "Form rename".into();
    project.save_scene(&mut form).unwrap();
    let mut source = wobu_store::SceneFile {
        scene: checked.scene.unwrap(),
        rel: original.rel.clone(),
        stamp: Some(raw.stamp),
    };
    assert!(matches!(
        project.save_scene(&mut source).unwrap(),
        wobu_store::SourceSave::Conflict { .. }
    ));
    assert_eq!(project.load_scene(original.scene.id).unwrap().scene.name, "Form rename");
}

#[test]
fn recovery_backup_is_idempotent_and_never_overwrites_different_original_bytes() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Repair").unwrap();
    let original = project.create_scene("Council").unwrap();
    let path = project.root().join(&original.rel);
    let valid = std::fs::read_to_string(&path).unwrap();
    let broken = "schema_version: 1\nscene: [";
    std::fs::write(&path, broken).unwrap();
    let raw = source_at(&project, &original.rel, None).unwrap();
    let backup = project
        .root()
        .join(format!("narrative/recovery/{}.{}.yaml", original.scene.id, raw.stamp.hash));
    std::fs::create_dir_all(backup.parent().unwrap()).unwrap();
    #[cfg(unix)]
    {
        // Matching bytes behind a mutable symlink are not an immutable backup.
        let external = temp.0.join("mutable-original.yaml");
        std::fs::write(&external, broken).unwrap();
        std::os::unix::fs::symlink(&external, &backup).unwrap();
        assert!(source_repair(&mut project, &original.rel, &valid, &raw.stamp, None).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        std::fs::remove_file(&backup).unwrap();
    }
    std::fs::write(&backup, "other recovery bytes").unwrap();
    assert!(source_repair(&mut project, &original.rel, &valid, &raw.stamp, None).is_err());
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), "other recovery bytes");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    std::fs::write(&backup, broken).unwrap();
    source_repair(&mut project, &original.rel, &valid, &raw.stamp, None).unwrap();
    assert_eq!(std::fs::read_to_string(backup).unwrap(), broken);
}

#[test]
fn repairing_malformed_source_cannot_retain_approval_for_different_wording() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Repair").unwrap();
    let original = project.create_scene("Council").unwrap();
    let path = project.root().join(&original.rel);
    std::fs::write(&path, "schema_version: 1\nscene: [").unwrap();
    let raw = source_at(&project, &original.rel, None).unwrap();
    let mut scene = original.scene;
    let mut beat = Beat::new("Evidence");
    let mut slot = DialogueSlot::new(Speaker::Player);
    let mut text = Text::written("Original wording");
    text.lifecycle.review = wobu_narrative::ReviewState::Approved;
    text.body = "Different wording".into();
    slot.variants.push(Variant::new(text));
    beat.dialogue.push(slot);
    scene.beats.push(beat);
    let yaml = SceneDocument::new(scene).to_yaml().unwrap();
    let error = source_repair(&mut project, &original.rel, &yaml, &raw.stamp, None).unwrap_err();
    assert_eq!(error.code, Code::Malformed);
    assert!(error.message.contains("explicit review"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), raw.yaml);
}
