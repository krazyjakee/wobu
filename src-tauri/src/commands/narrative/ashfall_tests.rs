//! Shared UI example against guarded command helpers and public Preview commands.
//! This does not substitute for a native WebKit walkthrough.
use super::*;
use crate::commands::narrative_preview::{
    PreviewAction, compile_project, narrative_preview_start, narrative_preview_step,
};
use std::collections::BTreeMap;
use wobu_narrative::{Name, Value};
use wobu_narrative_runtime::{CommandResult as HostResult, Snapshot, Yield};

#[derive(serde::Deserialize)]
struct WireFrame {
    current: Yield,
    snapshot: Snapshot,
    state: BTreeMap<Name, Value>,
}

fn wire(value: impl serde::Serialize) -> WireFrame {
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

#[test]
fn ashfall_example_reopens_identically_and_previews_three_reconverging_routes() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../examples/narrative/ashfall-council/source.json"
    ))
    .unwrap();
    let root = std::env::temp_dir().join(format!("wobu-ashfall-example-{}", wobu_core::new_id()));
    let mut project = Project::create(&root, "Ashfall walkthrough").unwrap();
    let state: StateDocument = serde_json::from_value(fixture["state"].clone()).unwrap();
    save_state(&mut project, state.clone(), &Precondition::New).unwrap();
    let created = project.create_scene("Ashfall council hearing").unwrap();
    let mut scene: Scene = serde_json::from_value(fixture["scene"].clone()).unwrap();
    scene.id = created.scene.id;
    for beat in &mut scene.beats {
        for slot in &mut beat.dialogue {
            for variant in &mut slot.variants {
                variant.text = narrative_text_written(variant.text.body.clone(), false);
            }
        }
    }
    let expected = Precondition::Stamp { stamp: created.stamp.clone().unwrap() };
    let saved = save_scene(&mut project, scene.clone(), None, &expected).unwrap();
    assert!(saved.scene.editorial_head.is_some());
    scene.editorial_head = saved.scene.editorial_head;
    assert_eq!(saved.scene, scene);
    let source_path = project.root().join(&saved.rel);
    let source_bytes = std::fs::read(&source_path).unwrap();
    let project_path = project.root().to_path_buf();
    drop(project);
    let reopened = Project::open(&project_path).unwrap();
    assert_eq!(reopened.load_scene(scene.id).unwrap().scene, scene);
    assert_eq!(reopened.state_document().unwrap().unwrap().0, state);
    assert_eq!(std::fs::read(&source_path).unwrap(), source_bytes);
    let commands = BTreeMap::from([(Name::new("ashfall_file_record").unwrap(), vec![])]);
    let report = compile_project(&reopened, commands).unwrap();
    assert!(report.graph.is_some(), "{:?}", report.diagnostics);
    let graph = report.graph.unwrap();
    let var = |name| Name::new(name).unwrap();
    for (trust, knows, route, expected_trust, expected_variant) in
        [(40, true, 0, 50, 0), (20, true, 1, 25, 1), (20, false, 2, 10, 2)]
    {
        let start = narrative_preview_start(
            graph.clone(),
            scene.id,
            BTreeMap::from([
                (var("ashfall_trust"), Value::Int(trust)),
                (var("ashfall_knows_logbook"), Value::Bool(knows)),
                (var("ashfall_record_filed"), Value::Bool(false)),
            ]),
            Some(0),
        )
        .map(wire)
        .unwrap();
        assert!(matches!(start.current, Yield::Line { .. }));
        let choices = narrative_preview_step(graph.clone(), start.snapshot, PreviewAction::Advance)
            .map(wire)
            .unwrap();
        let Yield::Choices { choices: available, .. } = &choices.current else {
            panic!("expected the council decision")
        };
        assert_eq!(available.len(), if trust >= 40 && knows { 3 } else { 2 });
        let choice = &scene.beats[0].choices[route];
        let mut response = narrative_preview_step(
            graph.clone(),
            choices.snapshot,
            PreviewAction::Choose { choice_id: choice.id.to_string() },
        )
        .map(wire)
        .unwrap();
        if route == 0 {
            let Yield::GameCommand { token, name, args } = response.current else {
                panic!("evidence must yield the clerk command")
            };
            assert_eq!(name, var("ashfall_file_record"));
            assert!(args.is_empty());
            response = narrative_preview_step(
                graph.clone(),
                response.snapshot,
                PreviewAction::CompleteCommand {
                    token,
                    result: HostResult::Success {
                        host_inputs: BTreeMap::from([(
                            var("ashfall_record_filed"),
                            Value::Bool(true),
                        )]),
                    },
                },
            )
            .map(wire)
            .unwrap();
        }
        assert_eq!(response.state[&var("ashfall_trust")], Value::Int(expected_trust));
        let Yield::Line { beat, slot, variant, .. } = &response.current else {
            panic!("all routes must reconverge on the response line")
        };
        assert_eq!(*beat, scene.beats[1].id.to_string());
        assert_eq!(*slot, scene.beats[1].dialogue[0].id.to_string());
        assert_eq!(*variant, scene.beats[1].dialogue[0].variants[expected_variant].id.to_string());
        let end = narrative_preview_step(graph.clone(), response.snapshot, PreviewAction::Advance)
            .map(wire)
            .unwrap();
        assert!(matches!(end.current, Yield::End { label } if label == "Hearing complete"));
    }
    assert_eq!(std::fs::read(&source_path).unwrap(), source_bytes);
    drop(reopened);
    std::fs::remove_dir_all(root).unwrap();
}
