//! The wire shape a Why affected inspector binds to.
//!
//! Driven over a real project rather than a hand-written row, so that the thing
//! being asserted is what the backend will actually send: a webview reading a
//! shape nothing produces is a pane that renders in a test and is blank in the
//! app.

use super::*;
use wobu_core::NodeKind;
use wobu_narrative::{Beat, DialogueSlot, Participant, Speaker, Text, Variant};
use wobu_store::Project;

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("wobu-affected-{}", wobu_core::new_id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn rows(project: &Project) -> Vec<serde_json::Value> {
    project
        .narrative_affected()
        .unwrap()
        .iter()
        .map(|item| serde_json::to_value(row(item)).unwrap())
        .collect()
}

#[test]
fn an_affected_line_arrives_named_explained_and_ready_to_render() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Affected").unwrap();
    let mut kael = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    kael.attributes.insert("narrative_voice".into(), serde_json::json!("Clipped."));
    project.save_node(kael.clone()).unwrap();

    let mut file = project.create_scene("Council").unwrap();
    file.scene.participants = vec![Participant { entity: kael.id, role: String::new() }];
    let mut beat = Beat::new("Evidence");
    let mut slot = DialogueSlot::new(Speaker::Entity(kael.id));
    slot.variants.push(Variant::new(Text::written("Ask for proof.")));
    let (slot_id, variant_id) = (slot.id, slot.variants[0].id);
    beat.dialogue.push(slot);
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();
    project.rebuild_narrative_dependencies().unwrap();
    assert!(rows(&project).is_empty());

    let mut edited = project.get_node_stamped(kael.id).unwrap().0;
    edited.attributes.insert("narrative_voice".into(), serde_json::json!("Hoarse."));
    project.save_node(edited).unwrap();

    let rows = rows(&project);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row["scene"], serde_json::json!(file.scene.id.to_string()));
    assert_eq!(row["asset"], serde_json::Value::Null);
    assert_eq!(row["slot"], serde_json::json!(slot_id.to_string()));
    assert_eq!(row["variant"], serde_json::json!(variant_id.to_string()));
    assert_eq!(row["kind"], serde_json::json!("changed"));
    assert!(row["before"].is_string() && row["after"].is_string());

    let explanations = row["explanations"].as_array().unwrap();
    assert_eq!(explanations.len(), 1);
    let explanation = &explanations[0];
    assert_eq!(
        explanation["source"],
        serde_json::json!(format!("character/{}/narrative_voice", kael.id))
    );
    assert_eq!(explanation["context"], serde_json::json!("context"));
    assert!(explanation["line"].as_str().unwrap().contains(&variant_id.to_string()));
    assert!(explanation["message"].as_str().unwrap().ends_with("was edited."));
}

#[test]
fn a_supporting_text_line_names_its_asset_rather_than_a_scene() {
    // `TargetRef` is two variants and the wire shape has to say which, or a pane
    // showing "scene: null" would have to guess what it is looking at.
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Affected").unwrap();
    let mut asset = project
        .create_text_asset(
            wobu_narrative::TextKind::Bark,
            "Gate guard",
            wobu_narrative::Name::new("approach_gate").unwrap(),
        )
        .unwrap();
    let mut entry = wobu_narrative::TextEntry::new("Warning");
    let mut line = DialogueSlot::new(Speaker::Narrator);
    line.variants.push(Variant::new(Text::written("Move along.")));
    entry.lines.push(line);
    asset.asset.entries.push(entry);
    project.save_text_asset(&mut asset).unwrap();

    // Nothing recorded yet, so the one line reports as untracked — which is the
    // honest answer for a project the tracker has never seen.
    let rows = rows(&project);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["scene"], serde_json::Value::Null);
    assert_eq!(rows[0]["asset"], serde_json::json!(asset.asset.id.to_string()));
    assert_eq!(rows[0]["kind"], serde_json::json!("untracked"));
    assert_eq!(rows[0]["before"], serde_json::Value::Null);
    assert!(rows[0]["explanations"].as_array().unwrap().is_empty());
}
