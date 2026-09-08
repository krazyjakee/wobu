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
    assert_eq!(source.file.scene, file.scene);
    assert_eq!(source.file.stamp, atomic::read_stamped(&path).unwrap().map(|(_, stamp)| stamp));
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
        yaml.replace("schema_version: 1", "schema_version: 999"),
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
