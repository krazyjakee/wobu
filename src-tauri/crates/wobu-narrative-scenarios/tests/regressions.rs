use std::{collections::BTreeMap, path::PathBuf};
use wobu_narrative::{Effect, Name, SceneDocument, StateDocument, Value};
use wobu_narrative_compiler::{CompileOptions, Graph, compile};
use wobu_narrative_scenarios::{Action, Assertion, Boundary, MAX_STEPS, Scenario, Step, run};
fn fixture() -> (Graph, Vec<Scenario>) {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../examples/narrative/harbor-watch");
    let scene = SceneDocument::parse(&std::fs::read_to_string(root.join("scene.yaml")).unwrap())
        .unwrap()
        .scene;
    let state =
        StateDocument::parse(&std::fs::read_to_string(root.join("state.yaml")).unwrap()).unwrap();
    let mut paths = std::fs::read_dir(root.join("scenarios"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    let scenarios = paths
        .iter()
        .map(|p| {
            let doc: serde_json::Value =
                serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
            serde_json::from_value::<Scenario>(doc["payload"].clone()).unwrap()
        })
        .collect::<Vec<_>>();
    let graph = compile(
        &[scene],
        &state.schema().unwrap(),
        &CompileOptions { commands: scenarios[0].commands.clone(), ..CompileOptions::default() },
    )
    .graph
    .unwrap();
    (graph, scenarios)
}
#[test]
fn original_trust_knowledge_fixture_runs_offline_with_pending_checkpoint_restore() {
    let (graph, scenarios) = fixture();
    assert_eq!(scenarios.len(), 6);
    let before = serde_json::to_vec(&graph).unwrap();
    for scenario in scenarios {
        assert!(scenario.steps.iter().any(|s| matches!(s.action, Some(Action::RestoreCheckpoint))));
        let result = run(&graph, &scenario).unwrap();
        assert!(result.passed, "{result:?}");
        assert_eq!(result.checked_steps, 8);
        assert_eq!(
            serde_json::to_vec(&graph).unwrap(),
            before,
            "runner never changes prepared content"
        );
    }
}
#[test]
fn changed_consequence_reports_first_divergence_with_choice_source_then_passes_after_repair() {
    let (mut graph, scenarios) = fixture();
    let original = graph.clone();
    let scene = graph.scenes.values_mut().next().unwrap();
    let beat = scene.beats.values_mut().next().unwrap();
    let choice = beat.choices[0].id.clone();
    let Effect::Add(increment) = &mut beat.choices[0].effects[0] else { panic!() };
    increment.by = 6;
    let result = run(&graph, &scenarios[0]).unwrap();
    let failure = result.divergence.unwrap();
    assert_eq!(failure.step, 3);
    assert_eq!(failure.field, "state.trust");
    assert_eq!(failure.expected, serde_json::json!(25));
    assert_eq!(failure.actual, serde_json::json!(26));
    assert_eq!(failure.site.choice, Some(choice));
    assert_eq!(failure.site.scene, scenarios[0].scene.to_string());
    assert!(!failure.trace.records.is_empty());
    assert!(run(&original, &scenarios[0]).unwrap().passed);
}
#[test]
fn partial_assertions_allow_unconstrained_state_and_wording_but_check_supplied_ids() {
    let (mut graph, mut scenarios) = fixture();
    let scenario = &mut scenarios[0];
    scenario.steps = vec![Step {
        action: None,
        expect: Assertion {
            boundary: Some(Boundary::Line {
                scene: None,
                beat: None,
                slot: Some(format!("{:026}", 3)),
                variant: None,
            }),
            state: BTreeMap::from([(Name::new("trust").unwrap(), Value::Int(20))]),
            error: None,
        },
    }];
    for scene in graph.scenes.values_mut() {
        for beat in scene.beats.values_mut() {
            for slot in &mut beat.dialogue {
                for variant in &mut slot.variants {
                    variant.text = "A newly edited sentence.".into();
                }
            }
        }
    }
    assert!(run(&graph, scenario).unwrap().passed);
    scenario.steps[0].expect.state.insert(Name::new("trust").unwrap(), Value::Int(21));
    assert_eq!(run(&graph, scenario).unwrap().divergence.unwrap().field, "state.trust");
}
#[test]
fn rejects_unsupported_payloads_unknown_fields_and_invalid_tapes() {
    let (graph, scenarios) = fixture();
    let scenario = &scenarios[0];
    let mut future = scenario.clone();
    future.version = 999;
    assert!(run(&graph, &future).is_err());
    let mut json = serde_json::to_value(scenario).unwrap();
    json["unknown"] = true.into();
    assert!(serde_json::from_value::<Scenario>(json).is_err());
    let mut invalid = scenario.clone();
    invalid.steps[1].action = None;
    assert!(invalid.validate().is_err());
    invalid.steps = vec![scenario.steps[0].clone(); MAX_STEPS + 1];
    assert!(invalid.validate().is_err());
    invalid.steps.clear();
    assert!(invalid.validate().is_err());
    let mut missing = scenario.clone();
    missing.initial_state.remove(&Name::new("knowledge").unwrap());
    assert_eq!(run(&graph, &missing).unwrap().divergence.unwrap().field, "start");
    let mut command = scenario.clone();
    command.commands.clear();
    assert!(run(&graph, &command).is_err());
}
#[test]
fn unavailable_choice_wrong_line_and_missing_checkpoint_fail_at_the_exact_action() {
    let (graph, scenarios) = fixture();
    let mut scenario = scenarios[0].clone();
    scenario.steps[3].action = Some(Action::Choose { choice: format!("{:026}", 11) });
    let failed = run(&graph, &scenario).unwrap().divergence.unwrap();
    assert_eq!(failed.step, 3);
    assert_eq!(failed.field, "error");
    scenario = scenarios[0].clone();
    scenario.steps[0].expect.boundary = Some(Boundary::Line {
        scene: None,
        beat: None,
        slot: None,
        variant: Some(format!("{:026}", 6)),
    });
    assert_eq!(run(&graph, &scenario).unwrap().divergence.unwrap().field, "line.variant");
    scenario = scenarios[0].clone();
    scenario.steps[1].action = Some(Action::RestoreCheckpoint);
    assert_eq!(run(&graph, &scenario).unwrap().divergence.unwrap().step, 1);
}

#[test]
fn checkpoint_comparison_does_not_reuse_prior_actions_trace() {
    let (graph, scenarios) = fixture();
    let mut scenario = scenarios[0].clone();
    scenario.steps[4].expect.state.insert(Name::new("trust").unwrap(), Value::Int(999));
    let failure = run(&graph, &scenario).unwrap().divergence.unwrap();
    assert_eq!(failure.step, 4);
    assert!(failure.trace.records.is_empty(), "saving a checkpoint did not reapply effects");
    scenario = scenarios[0].clone();
    scenario.steps[1].action = Some(Action::RestoreCheckpoint);
    let failure = run(&graph, &scenario).unwrap().divergence.unwrap();
    assert!(!failure.trace.committed);
    assert!(failure.trace.error.is_some());
    assert!(failure.trace.records.is_empty());
}
