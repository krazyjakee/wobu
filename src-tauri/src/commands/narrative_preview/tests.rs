use super::*;
use wobu_narrative::{
    Beat, Choice, Destination, DialogueSlot, Effect, Increment, Speaker, StateDocument, Text,
    Variant,
};

#[test]
fn handwritten_project_compiles_and_previews_without_mutating_its_source() {
    let root = std::env::temp_dir().join(format!("wobu-preview-{}", wobu_core::new_id()));
    let mut project = Project::create(&root, "Ashfall").unwrap();
    let state = StateDocument::new(vec![wobu_narrative::VariableDecl {
        name: Name::new("trust").unwrap(),
        ty: VarType::Int { min: 0, max: 100 },
        default: wobu_narrative::Value::Int(10),
        owner: wobu_narrative::Owner::Narrative,
        description: String::new(),
    }]);
    project.save_state(&state, None).unwrap();
    let mut file = project.create_scene("Council hearing").unwrap();
    let mut beat = Beat::new("Present evidence");
    let mut slot = DialogueSlot::new(Speaker::Player);
    slot.variants.push(Variant::new(Text::written("I brought the logbook.")));
    beat.dialogue.push(slot);
    let mut choice =
        Choice::new("Show evidence", Destination::End { label: "Council support".into() });
    choice.effects.push(Effect::Add(Increment { var: Name::new("trust").unwrap(), by: 10 }));
    let choice_id = choice.id.to_string();
    beat.choices.push(choice);
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();
    let fingerprint = project.narrative_fingerprint().unwrap();
    let report = compile_project(&project, BTreeMap::new()).unwrap();
    assert!(
        report.diagnostics.iter().all(|d| d.severity != wobu_narrative_compiler::Severity::Error),
        "{:?}",
        report.diagnostics
    );
    let graph = report.graph.unwrap();
    let initial = BTreeMap::from([(Name::new("trust").unwrap(), wobu_narrative::Value::Int(40))]);
    let start = narrative_preview_start(graph.clone(), file.scene.id, initial, None).unwrap();
    assert!(matches!(start.current, Yield::Line { .. }));
    let snapshot = start.snapshot.clone();
    let choices = narrative_preview_step(graph.clone(), snapshot, PreviewAction::Advance).unwrap();
    let invalid = narrative_preview_step(
        graph.clone(),
        choices.snapshot.clone(),
        PreviewAction::Choose { choice_id: "missing".into() },
    );
    assert!(invalid.is_err());
    let end = narrative_preview_step(
        graph.clone(),
        choices.snapshot,
        PreviewAction::Choose { choice_id },
    )
    .unwrap();
    assert_eq!(end.state[&Name::new("trust").unwrap()], wobu_narrative::Value::Int(50));
    assert!(matches!(end.current, Yield::End { .. }));
    let restored = narrative_preview_step(graph, start.snapshot, PreviewAction::Restore).unwrap();
    assert_eq!(restored.state[&Name::new("trust").unwrap()], wobu_narrative::Value::Int(40));
    assert_eq!(project.narrative_fingerprint().unwrap(), fingerprint);
    drop(project);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn preview_compilation_reports_broken_source_and_requires_command_registration() {
    let root = std::env::temp_dir().join(format!("wobu-preview-errors-{}", wobu_core::new_id()));
    let mut project = Project::create(&root, "Ashfall").unwrap();
    let mut file = project.create_scene("Hearing").unwrap();
    let mut beat = Beat::new("Challenge");
    let mut choice = Choice::new("Leave", Destination::End { label: "Left".into() });
    choice.effects.push(Effect::Command(wobu_narrative::HostCommand {
        name: Name::new("ring_bell").unwrap(),
        args: vec![],
    }));
    beat.choices.push(choice);
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();
    let refused = compile_project(&project, BTreeMap::new()).unwrap();
    assert!(refused.graph.is_none());
    assert!(
        compile_project(&project, BTreeMap::from([(Name::new("ring_bell").unwrap(), vec![])]))
            .unwrap()
            .graph
            .is_some()
    );
    std::fs::write(project.root().join("narrative/scenes/broken.yaml"), "scene: [").unwrap();
    assert!(compile_project(&project, BTreeMap::new()).is_err());
    drop(project);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn preview_refuses_numbers_that_the_webview_would_silently_round() {
    assert!(bridge_integers(&serde_json::json!({"range":{"max":i64::MAX}})).is_err());
    assert!(bridge_integers(&serde_json::json!({"state":[-9_007_199_254_740_992_i64]})).is_err());
    assert!(
        bridge_integers(
            &serde_json::json!({"range":[-9_007_199_254_740_991_i64,9_007_199_254_740_991_i64]})
        )
        .is_ok()
    );
}

#[test]
fn preview_participants_must_be_characters_not_arbitrary_world_nodes() {
    let root = std::env::temp_dir().join(format!("wobu-preview-cast-{}", wobu_core::new_id()));
    let mut project = Project::create(&root, "Ashfall").unwrap();
    let prop = project.create_node(wobu_core::NodeKind::Prop, "Logbook", None).unwrap();
    let character = project.create_node(wobu_core::NodeKind::Character, "Mira", None).unwrap();
    let mut file = project.create_scene("Hearing").unwrap();
    let mut beat = Beat::new("Challenge");
    beat.outcomes.push(wobu_narrative::Outcome::new(Destination::End { label: "Left".into() }));
    file.scene.beats.push(beat);
    file.scene
        .participants
        .push(wobu_narrative::Participant { entity: prop.id, role: String::new() });
    project.save_scene(&mut file).unwrap();
    let refused = compile_project(&project, BTreeMap::new()).unwrap();
    assert!(refused.graph.is_none());
    assert!(refused.diagnostics.iter().any(|d| d.code == "unknown_entity"
        && d.site == wobu_narrative::Site::Participant { entity: prop.id }));
    file.scene.participants[0].entity = character.id;
    project.save_scene(&mut file).unwrap();
    assert!(compile_project(&project, BTreeMap::new()).unwrap().graph.is_some());
    drop(project);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn preview_rejects_output_counters_that_advance_beyond_the_exact_webview_range() {
    let mut scene = wobu_narrative::Scene::new("Loop");
    let mut beat = Beat::new("Pause each lap");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written("One more lap.")));
    beat.dialogue.push(slot);
    beat.outcomes.push(wobu_narrative::Outcome::new(Destination::Beat(beat.id)));
    let beat_id = beat.id.to_string();
    scene.beats.push(beat);
    let graph = compile(
        &[scene.clone()],
        &[],
        &wobu_narrative::StateSchema::empty(),
        &CompileOptions::default(),
    )
    .graph
    .unwrap();
    let start = narrative_preview_start(graph.clone(), scene.id, Values::new(), None).unwrap();
    let mut saved = serde_json::to_value(start.snapshot).unwrap();
    saved["visits"][&beat_id] = serde_json::json!(9_007_199_254_740_991_i64);
    let snapshot: Snapshot = serde_json::from_value(saved).unwrap();
    // This snapshot is exact on input. Visiting the beat once more is not.
    assert!(
        narrative_preview_step(graph.clone(), snapshot.clone(), PreviewAction::Restore).is_ok()
    );
    let error = narrative_preview_step(graph, snapshot, PreviewAction::Advance).unwrap_err();
    assert!(error.message.contains("9007199254740991"));
}

#[test]
fn pending_preview_commands_restore_fail_cancel_and_accept_validated_host_outputs() {
    let mut scene = wobu_narrative::Scene::new("Ashfall command");
    let mut beat = Beat::new("Command");
    let mut outcome = wobu_narrative::Outcome::new(Destination::End { label: "Done".into() });
    outcome.effects.push(Effect::Command(wobu_narrative::HostCommand {
        name: Name::new("award").unwrap(),
        args: vec![],
    }));
    beat.outcomes.push(outcome);
    scene.beats.push(beat);
    let schema = wobu_narrative::StateSchema::new([wobu_narrative::VariableDecl {
        name: Name::new("host_ready").unwrap(),
        ty: VarType::Bool,
        default: wobu_narrative::Value::Bool(false),
        owner: wobu_narrative::Owner::Host,
        description: String::new(),
    }])
    .unwrap();
    let graph = compile(
        &[scene.clone()],
        &[],
        &schema,
        &CompileOptions {
            commands: BTreeMap::from([(Name::new("award").unwrap(), vec![])]),
            ..CompileOptions::default()
        },
    )
    .graph
    .unwrap();
    let start = narrative_preview_start(
        graph.clone(),
        scene.id,
        BTreeMap::from([(Name::new("host_ready").unwrap(), wobu_narrative::Value::Bool(false))]),
        Some(17),
    )
    .unwrap();
    let Yield::GameCommand { token, .. } = &start.current else { panic!() };
    let saved = start.snapshot.clone();
    for result in [HostResult::Failed { message: "retry later".into() }, HostResult::Cancelled] {
        let restored =
            narrative_preview_step(graph.clone(), saved.clone(), PreviewAction::Restore).unwrap();
        let failed = narrative_preview_step(
            graph.clone(),
            restored.snapshot,
            PreviewAction::CompleteCommand { token: token.clone(), result },
        )
        .unwrap();
        assert_eq!(failed.snapshot, saved);
        assert!(!failed.trace.committed);
        assert!(failed.trace.error.is_some());
        assert_eq!(failed.current, start.current);
    }
    let bad = HostResult::Success {
        host_inputs: BTreeMap::from([(
            Name::new("host_ready").unwrap(),
            wobu_narrative::Value::Int(1),
        )]),
    };
    assert!(
        narrative_preview_step(
            graph.clone(),
            saved.clone(),
            PreviewAction::CompleteCommand { token: token.clone(), result: bad }
        )
        .is_err()
    );
    let result = HostResult::Success {
        host_inputs: BTreeMap::from([(
            Name::new("host_ready").unwrap(),
            wobu_narrative::Value::Bool(true),
        )]),
    };
    let done = narrative_preview_step(
        graph.clone(),
        saved,
        PreviewAction::CompleteCommand { token: token.clone(), result: result.clone() },
    )
    .unwrap();
    assert_eq!(done.state[&Name::new("host_ready").unwrap()], wobu_narrative::Value::Bool(true));
    let repeated = narrative_preview_step(
        graph,
        done.snapshot.clone(),
        PreviewAction::CompleteCommand { token: token.clone(), result },
    )
    .unwrap();
    assert_eq!(repeated.snapshot, done.snapshot);
}
