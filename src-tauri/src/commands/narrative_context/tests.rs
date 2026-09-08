use super::*;
use wobu_narrative::{Beat, DialogueSlot, Speaker, StateDocument};
struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("wobu-context-{}", wobu_core::new_id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(temp: &Temp) -> (Project, Options) {
    let mut project = Project::create(&temp.0, "Context").unwrap();
    let mut file = project.create_scene("Council").unwrap();
    let mut beat = Beat::new("Evidence");
    let slot = DialogueSlot::new(Speaker::Narrator);
    let selection = wobu_narrative_context::Selection {
        scene: file.scene.id,
        beat: beat.id,
        slot: slot.id,
        variant: None,
    };
    beat.dialogue.push(slot);
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();
    (project, Options { selection, state: BTreeMap::new(), token_budget: 4000 })
}
#[test]
fn frozen_capture_reads_saved_sources_and_retains_old_request_after_edits() {
    let temp = Temp::new();
    let (mut project, options) = fixture(&temp);
    let first = capture(&project, options.clone(), || {}).unwrap();
    assert!(first.ready);
    let mut file = project.load_scene(options.selection.scene).unwrap();
    file.scene.beats[0].must_convey.push("Ask for proof".into());
    project.save_scene(&mut file).unwrap();
    let current = capture(&project, options, || {}).unwrap();
    assert_ne!(first.hash, current.hash);
    assert!(!first.request.contains("Ask for proof"));
    assert!(current.request.contains("Ask for proof"));
}
#[test]
fn mutation_during_capture_is_rejected_including_world_creation_and_state_rewrite() {
    let temp = Temp::new();
    let (mut project, options) = fixture(&temp);
    assert!(
        capture(&project, options.clone(), || {
            std::fs::write(
                project.root().join("narrative/world.yaml"),
                WorldDocument::default().to_yaml().unwrap(),
            )
            .unwrap();
        })
        .is_err()
    );
    project.save_state(&StateDocument::new(vec![]), None).unwrap();
    assert!(
        capture(&project, options, || {
            std::fs::write(
                project.root().join("narrative/state.yaml"),
                "schema_version: 1\nvariables: []\n# rewritten",
            )
            .unwrap();
        })
        .is_err()
    );
}

#[test]
fn character_capture_uses_same_read_stamp_and_only_explicit_narrative_voice() {
    let temp = Temp::new();
    let (mut project, options) = fixture(&temp);
    let mut character = project.create_node(NodeKind::Character, "Witness", None).unwrap();
    character.notes_raw = "Private visual notes must not enter a dialogue request".into();
    character
        .attributes
        .insert("narrative_voice".into(), serde_json::json!("Dry, concise sentences"));
    project.save_node(character.clone()).unwrap();
    let mut file = project.load_scene(options.selection.scene).unwrap();
    file.scene.beats[0].dialogue[0].speaker = Speaker::Entity(character.id);
    project.save_scene(&mut file).unwrap();
    let result = capture(&project, options.clone(), || {}).unwrap();
    assert!(result.request.contains("Dry, concise sentences"));
    assert!(!result.request.contains("Private visual notes"));
    let path = project.root().join("nodes/character").join(format!("{}.md", character.slug));
    let original = std::fs::read_to_string(&path).unwrap();
    assert!(
        capture(&project, options, || {
            std::fs::write(path, format!("{original}\nExternal edit")).unwrap();
        })
        .is_err()
    );
}
#[test]
fn unsafe_context_numbers_are_rejected_at_the_webview_boundary() {
    assert!(unsafe_integer(&serde_json::json!({"relationship":i64::MAX})));
    assert!(!unsafe_integer(&serde_json::json!({"relationship":9007199254740991_i64})));
}

#[test]
fn authored_state_is_typed_before_webview_number_rounding() {
    let temp = Temp::new();
    let (_, options) = fixture(&temp);
    for raw in [r#"{"chapter":9007199254740991.4}"#, r#"{"chapter":1.00000000000000001}"#] {
        assert!(parse_capture_options(options.selection.clone(), raw, 4000).is_err());
    }
    let valid =
        parse_capture_options(options.selection, r#"{"chapter":9007199254740991}"#, 4000).unwrap();
    assert_eq!(
        valid.state[&"chapter".parse().unwrap()],
        wobu_narrative::Value::Int(9007199254740991)
    );
}
