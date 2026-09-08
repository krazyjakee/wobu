use std::collections::BTreeMap;

use wobu_narrative::*;
use wobu_narrative_compiler::*;
use wobu_narrative_runtime::{CommandResult, Error, Runtime, State, Yield, evaluate};

fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn end() -> Destination {
    Destination::End { label: "finished".into() }
}
fn fixture() -> (Scene, StateSchema, CompileOptions) {
    let mut scene = Scene::new("Council");
    let mut beat = Beat::new("Evidence");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written("Bring the logbook.")));
    beat.dialogue.push(slot);
    let mut choice = Choice::new("Present it", end());
    choice.effects = vec![
        Effect::Add(Increment { var: name("trust"), by: 2 }),
        Effect::Command(HostCommand {
            name: name("show_evidence"),
            args: vec![Operand::Var(name("trust"))],
        }),
        Effect::Add(Increment { var: name("trust"), by: 3 }),
        Effect::Command(HostCommand {
            name: name("show_evidence"),
            args: vec![Operand::Var(name("trust"))],
        }),
    ];
    beat.choices.push(choice);
    scene.beats.push(beat);
    let schema = StateSchema::new([
        VariableDecl {
            name: name("trust"),
            ty: VarType::Int { min: 0, max: 10 },
            default: Value::Int(1),
            owner: Owner::Narrative,
            description: "Writer-only secret description".into(),
        },
        VariableDecl {
            name: name("has_logbook"),
            ty: VarType::Bool,
            default: Value::Bool(false),
            owner: Owner::Host,
            description: String::new(),
        },
    ])
    .unwrap();
    let options = CompileOptions {
        commands: BTreeMap::from([(name("show_evidence"), vec![VarType::Int { min: 0, max: 10 }])]),
        ..CompileOptions::default()
    };
    (scene, schema, options)
}
fn graph(scene: &Scene, schema: &StateSchema, options: &CompileOptions) -> Graph {
    let report = compile(std::slice::from_ref(scene), schema, options);
    assert!(report.graph.is_some(), "{:?}", report.diagnostics);
    report.graph.unwrap()
}
fn start(graph: Graph, scene: &Scene) -> Runtime {
    Runtime::start(
        graph,
        &scene.id.to_string(),
        BTreeMap::from([(name("has_logbook"), Value::Bool(true))]),
        "playthrough-1".into(),
        42,
        32,
    )
    .unwrap()
}
fn roundtrip(runtime: &Runtime, graph: &Graph) -> Runtime {
    let snapshot =
        serde_json::from_slice(&serde_json::to_vec(&runtime.snapshot()).unwrap()).unwrap();
    let restored = Runtime::restore(graph.clone(), snapshot).unwrap();
    assert_eq!(runtime.current(), restored.current());
    assert_eq!(runtime.state(), restored.state());
    assert_eq!(runtime.visits(), restored.visits());
    restored
}
fn success() -> CommandResult {
    CommandResult::Success { host_inputs: State::new() }
}

#[test]
fn each_yield_roundtrips_and_commands_commit_once_with_ordered_arguments() {
    let (scene, schema, options) = fixture();
    let graph = graph(&scene, &schema, &options);
    let mut run = roundtrip(&start(graph.clone(), &scene), &graph);
    assert!(matches!(run.current().unwrap(), Yield::Line { .. }));
    let Yield::Choices { choices, .. } = run.advance().unwrap() else { panic!() };
    let mut run = roundtrip(&run, &graph);
    let Yield::GameCommand { token: first, args, .. } = run.choose(&choices[0].id).unwrap() else {
        panic!()
    };
    assert_eq!(args, vec![Value::Int(3)]);
    assert_eq!(run.state()[&name("trust")], Value::Int(6));
    let mut run = roundtrip(&run, &graph);
    let pending = run.snapshot();
    assert_eq!(run.complete_command("wrong", success()), Err(Error::InvalidCommand));
    assert_eq!(run.snapshot(), pending);
    assert_eq!(
        run.complete_command(&first, CommandResult::Cancelled),
        Err(Error::CommandCancelled)
    );
    assert_eq!(run.snapshot(), pending);
    assert!(matches!(
        run.complete_command(&first, CommandResult::Failed { message: "try later".into() }),
        Err(Error::CommandFailed(_))
    ));
    assert_eq!(run.snapshot(), pending);
    let Yield::GameCommand { token: second, args, .. } =
        run.complete_command(&first, success()).unwrap()
    else {
        panic!()
    };
    assert_eq!(args, vec![Value::Int(6)]);
    assert_ne!(first, second);
    let mut run = roundtrip(&run, &graph);
    assert_eq!(run.complete_command(&first, success()).unwrap(), run.current().unwrap());
    assert_eq!(run.state()[&name("trust")], Value::Int(6));
    assert!(matches!(run.complete_command(&second, success()).unwrap(), Yield::End { .. }));
    let mut run = roundtrip(&run, &graph);
    assert_eq!(run.complete_command(&second, success()).unwrap(), run.current().unwrap());
    assert_eq!(run.advance().unwrap(), run.current().unwrap());
}

#[test]
fn invalid_host_results_and_unavailable_choices_are_transactional() {
    let (mut scene, schema, options) = fixture();
    let mut unavailable = scene.beats[0].choices[0].duplicated();
    unavailable.requires = Some(Condition::Never);
    let unavailable_id = unavailable.id.to_string();
    scene.beats[0].choices.push(unavailable);
    let graph = graph(&scene, &schema, &options);
    let mut run = start(graph, &scene);
    assert_eq!(run.choose(&unavailable_id), Err(Error::InvalidAction));
    run.advance().unwrap();
    let before = run.snapshot();
    assert_eq!(run.choose(&unavailable_id), Err(Error::UnavailableChoice(unavailable_id)));
    assert_eq!(run.snapshot(), before);
    let Yield::GameCommand { token, .. } =
        run.choose(&scene.beats[0].choices[0].id.to_string()).unwrap()
    else {
        panic!()
    };
    let before = run.snapshot();
    for inputs in [
        BTreeMap::from([(name("trust"), Value::Int(5))]),
        BTreeMap::from([(name("has_logbook"), Value::Int(1))]),
        BTreeMap::from([(name("unknown"), Value::Bool(true))]),
    ] {
        assert!(
            run.complete_command(&token, CommandResult::Success { host_inputs: inputs }).is_err()
        );
        assert_eq!(run.snapshot(), before);
    }
}

#[test]
fn overflow_after_a_command_prevents_every_effect_and_command() {
    let (mut scene, schema, options) = fixture();
    scene.beats[0].choices[0]
        .effects
        .push(Effect::Add(Increment { var: name("trust"), by: i64::MAX }));
    let graph = graph(&scene, &schema, &options);
    let mut run = start(graph, &scene);
    run.advance().unwrap();
    let before = run.snapshot();
    assert_eq!(
        run.choose(&scene.beats[0].choices[0].id.to_string()),
        Err(Error::Overflow("trust".into()))
    );
    assert_eq!(run.snapshot(), before);
}

#[test]
fn missing_host_input_and_tampered_saves_fail_closed() {
    let (scene, schema, options) = fixture();
    let graph = graph(&scene, &schema, &options);
    assert!(matches!(
        Runtime::start(graph.clone(), &scene.id.to_string(), State::new(), "run".into(), 0, 32),
        Err(Error::MissingState(_))
    ));
    let run = start(graph.clone(), &scene);
    for (field, value) in [
        ("version", serde_json::json!(99)),
        ("graph_hash", serde_json::json!("other")),
        ("beat", serde_json::json!("missing")),
        ("state", serde_json::json!({})),
        ("phase", serde_json::json!({"dialogue":{"index":999,"variant":"missing"}})),
    ] {
        let mut saved = serde_json::to_value(run.snapshot()).unwrap();
        saved[field] = value;
        let snapshot = serde_json::from_value(saved).unwrap();
        assert!(Runtime::restore(graph.clone(), snapshot).is_err(), "{field}");
    }
    let mut different = graph.clone();
    different
        .scenes
        .get_mut(&scene.id.to_string())
        .unwrap()
        .beats
        .get_mut(&scene.beats[0].id.to_string())
        .unwrap()
        .dialogue[0]
        .variants[0]
        .text = "Edited".into();
    assert!(matches!(Runtime::restore(different, run.snapshot()), Err(Error::Incompatible)));
}

#[test]
fn first_matching_variant_is_selected_once_and_outcomes_use_author_priority() {
    let (mut scene, schema, options) = fixture();
    let slot = &mut scene.beats[0].dialogue[0];
    slot.variants.insert(
        0,
        Variant { when: Some(Condition::Never), ..Variant::new(Text::written("Never")) },
    );
    slot.variants.push(Variant::new(Text::written("Later fallback")));
    scene.beats[0].choices.clear();
    scene.beats[0].outcomes = vec![
        Outcome {
            when: Some(Condition::Never),
            ..Outcome::new(Destination::End { label: "never".into() })
        },
        Outcome::new(end()),
        Outcome::new(Destination::End { label: "later".into() }),
    ];
    let graph = graph(&scene, &schema, &options);
    let mut run = start(graph.clone(), &scene);
    assert!(
        matches!(run.current().unwrap(), Yield::Line { text, .. } if text == "Bring the logbook.")
    );
    run.update_host_inputs(BTreeMap::from([(name("has_logbook"), Value::Bool(false))])).unwrap();
    let mut run = roundtrip(&run, &graph);
    assert_eq!(run.advance().unwrap(), Yield::End { label: "finished".into() });
}

#[test]
fn no_match_and_automatic_loops_are_bounded_and_transactional() {
    let (mut scene, schema, options) = fixture();
    scene.beats[0].choices.clear();
    scene.beats[0].outcomes.push(Outcome { when: Some(Condition::Never), ..Outcome::new(end()) });
    let mut run = start(graph(&scene, &schema, &options), &scene);
    let before = run.snapshot();
    assert!(matches!(run.advance(), Err(Error::NoMatch(_))));
    assert_eq!(run.snapshot(), before);
    scene.beats[0].dialogue.clear();
    scene.beats[0].outcomes = vec![Outcome::new(Destination::Beat(scene.beats[0].id))];
    assert!(matches!(
        Runtime::start(
            graph(&scene, &schema, &options),
            &scene.id.to_string(),
            BTreeMap::from([(name("has_logbook"), Value::Bool(true))]),
            "run".into(),
            0,
            16
        ),
        Err(Error::StepLimit(16))
    ));
}

#[test]
fn comparisons_boolean_composition_and_missing_values_have_defined_semantics() {
    let state = BTreeMap::from([
        (name("a"), Value::Int(4)),
        (name("b"), Value::Int(5)),
        (name("flag"), Value::Bool(true)),
        (name("quest"), Value::Enum(name("active"))),
    ]);
    for (op, expected) in [
        (CompareOp::Eq, false),
        (CompareOp::Ne, true),
        (CompareOp::Lt, true),
        (CompareOp::Le, true),
        (CompareOp::Gt, false),
        (CompareOp::Ge, false),
    ] {
        assert_eq!(
            evaluate(
                &Condition::Compare(Comparison {
                    var: name("a"),
                    op,
                    value: Operand::Var(name("b"))
                }),
                &state
            ),
            Ok(expected)
        );
    }
    for (var, value) in [("flag", Value::Bool(true)), ("quest", Value::Enum(name("active")))] {
        assert_eq!(
            evaluate(
                &Condition::Compare(Comparison {
                    var: name(var),
                    op: CompareOp::Eq,
                    value: Operand::Literal(value)
                }),
                &state
            ),
            Ok(true)
        );
    }
    assert_eq!(evaluate(&Condition::All(vec![]), &state), Ok(true));
    assert_eq!(evaluate(&Condition::Any(vec![]), &state), Ok(false));
    assert_eq!(evaluate(&Condition::Not(Box::new(Condition::Never)), &state), Ok(true));
    let missing = Condition::Compare(Comparison {
        var: name("missing"),
        op: CompareOp::Eq,
        value: Operand::Literal(Value::Bool(true)),
    });
    assert!(matches!(evaluate(&missing, &state), Err(Error::MissingState(_))));
    assert_eq!(evaluate(&Condition::Any(vec![Condition::Always, missing]), &state), Ok(true));
}

#[test]
fn compile_is_deterministic_and_strips_author_context_and_lifecycle() {
    let (mut scene, schema, options) = fixture();
    scene.summary = "Secret writer context".into();
    scene.beats[0].must_not_reveal.push("Secret spoiler".into());
    let first = graph(&scene, &schema, &options);
    let mut second = scene.clone();
    second.name = "Rename".into();
    second.summary.clear();
    second.beats[0].must_not_reveal.clear();
    assert_eq!(first.canonical_bytes(), graph(&second, &schema, &options).canonical_bytes());
    let bytes = String::from_utf8(first.canonical_bytes()).unwrap();
    for excluded in ["Secret", "provenance", "lifecycle", "description", "must_not_reveal"] {
        assert!(!bytes.contains(excluded));
    }
    let decoded: Graph = serde_json::from_str(&bytes).unwrap();
    assert_eq!(first.hash(), decoded.hash());
    assert!(first.source_map.contains_key(&scene.beats[0].dialogue[0].variants[0].id.to_string()));
    let other = scene.duplicated();
    assert_eq!(
        compile(&[scene.clone(), other.clone()], &schema, &options).graph,
        compile(&[other, scene], &schema, &options).graph
    );
}

#[test]
fn compiler_enforces_release_world_commands_references_and_duplicate_ids() {
    let (mut scene, schema, mut options) = fixture();
    options.profile = Profile::Release;
    assert!(compile(&[scene.clone()], &schema, &options).graph.is_none());
    scene.beats[0].dialogue[0].variants[0].text.lifecycle.review = ReviewState::Approved;
    assert!(compile(&[scene.clone()], &schema, &options).graph.is_some());
    scene.beats[0].dialogue[0].variants[0].text.lifecycle.freshness = Freshness::OutOfDate;
    assert!(compile(&[scene.clone()], &schema, &options).graph.is_none());
    options.profile = Profile::Development;
    options.commands.clear();
    assert!(
        compile(&[scene.clone()], &schema, &options)
            .diagnostics
            .iter()
            .any(|d| d.code == "invalid_command")
    );
    scene.beats[0].choices[0].effects.clear();
    scene.beats[0].choices[0].to = Destination::Beat(BeatId::new());
    assert!(compile(&[scene.clone()], &schema, &options).graph.is_none());
    scene.beats[0].choices[0].to = end();
    let entity = scene.id.raw();
    scene.participants.push(Participant { entity, role: String::new() });
    assert!(
        compile(&[scene.clone()], &schema, &options)
            .diagnostics
            .iter()
            .any(|d| d.code == "unknown_entity")
    );
    options.known_entities.insert(entity);
    assert!(compile(&[scene.clone()], &schema, &options).graph.is_some());
    assert!(
        compile(&[scene.clone(), scene], &schema, &options)
            .diagnostics
            .iter()
            .any(|d| d.code == "duplicate_id")
    );
}

#[test]
fn scenario_overrides_are_validated_without_changing_graph_identity() {
    let (scene, schema, options) = fixture();
    let graph = graph(&scene, &schema, &options);
    let state =
        BTreeMap::from([(name("trust"), Value::Int(4)), (name("has_logbook"), Value::Bool(true))]);
    assert!(
        Runtime::start(graph.clone(), &scene.id.to_string(), state.clone(), "run".into(), 0, 32)
            .is_err()
    );
    let run =
        Runtime::start_with_state(graph.clone(), &scene.id.to_string(), state, "run".into(), 0, 32)
            .unwrap();
    assert_eq!(run.state()[&name("trust")], Value::Int(4));
    roundtrip(&run, &graph);
    for value in [Value::Int(11), Value::Bool(true)] {
        let state =
            BTreeMap::from([(name("trust"), value), (name("has_logbook"), Value::Bool(true))]);
        assert!(
            Runtime::start_with_state(
                graph.clone(),
                &scene.id.to_string(),
                state,
                "run".into(),
                0,
                32
            )
            .is_err()
        );
    }
}

#[test]
fn set_effects_and_cross_scene_entry_use_current_state() {
    let (mut scene, schema, options) = fixture();
    let mut next_scene = Scene::new("Aftermath");
    next_scene.entry = Some(Condition::Compare(Comparison {
        var: name("trust"),
        op: CompareOp::Eq,
        value: Operand::Literal(Value::Int(8)),
    }));
    let mut next_beat = Beat::new("Ending");
    next_beat.outcomes.push(Outcome::new(end()));
    next_scene.beats.push(next_beat);
    scene.beats[0].choices[0].effects = vec![Effect::Set(Assignment {
        var: name("trust"),
        value: Operand::Literal(Value::Int(8)),
    })];
    scene.beats[0].choices[0].to = Destination::Scene(next_scene.id);
    let graph = compile(&[scene.clone(), next_scene.clone()], &schema, &options).graph.unwrap();
    let mut run = start(graph.clone(), &scene);
    run.advance().unwrap();
    assert_eq!(
        run.choose(&scene.beats[0].choices[0].id.to_string()).unwrap(),
        Yield::End { label: "finished".into() }
    );
    assert_eq!(run.state()[&name("trust")], Value::Int(8));
    assert_eq!(run.visits()[&next_scene.beats[0].id.to_string()], 1);
    roundtrip(&run, &graph);
    assert!(matches!(
        Runtime::start(
            graph,
            &next_scene.id.to_string(),
            BTreeMap::from([(name("has_logbook"), Value::Bool(true))]),
            "run".into(),
            0,
            32
        ),
        Err(Error::EntryDenied(_))
    ));
}

#[test]
fn large_loop_budget_uses_constant_native_stack() {
    let (mut scene, schema, options) = fixture();
    scene.beats[0].dialogue.clear();
    scene.beats[0].choices.clear();
    scene.beats[0].outcomes = vec![Outcome::new(Destination::Beat(scene.beats[0].id))];
    assert!(matches!(
        Runtime::start(
            graph(&scene, &schema, &options),
            &scene.id.to_string(),
            BTreeMap::from([(name("has_logbook"), Value::Bool(true))]),
            "run".into(),
            0,
            10000
        ),
        Err(Error::StepLimit(10000))
    ));
}

#[test]
fn draft_slots_and_malformed_logic_produce_source_addressed_errors() {
    let (mut scene, schema, mut options) = fixture();
    let beat_id = scene.beats[0].id;
    let slot_id = scene.beats[0].dialogue[0].id;
    scene.beats[0].dialogue[0].variants.clear();
    let draft = compile(&[scene.clone()], &schema, &options);
    assert!(draft.graph.is_some());
    let missing = draft.diagnostics.iter().find(|d| d.code == "missing_text").unwrap();
    assert_eq!(missing.site, Site::DialogueSlot { beat: beat_id, slot: slot_id });
    assert_eq!(missing.severity, Severity::Warning);
    options.profile = Profile::Release;
    assert!(compile(&[scene.clone()], &schema, &options).graph.is_none());
    options.profile = Profile::Development;
    scene.beats[0].choices[0].effects = vec![Effect::Set(Assignment {
        var: name("has_logbook"),
        value: Operand::Literal(Value::Bool(false)),
    })];
    let invalid = compile(&[scene], &schema, &options);
    assert!(invalid.graph.is_none());
    let effect =
        invalid.diagnostics.iter().find(|d| d.message.contains("owned by the host")).unwrap();
    assert!(
        matches!(effect.site, Site::Destination(DestinationSite::Choice { beat, .. }) if beat == beat_id)
    );
}

#[test]
fn invalid_command_arguments_and_acknowledgement_changes_are_rejected() {
    let (mut scene, schema, options) = fixture();
    let Effect::Command(command) = &mut scene.beats[0].choices[0].effects[1] else { panic!() };
    command.args = vec![Operand::Literal(Value::Bool(true))];
    assert!(
        compile(&[scene], &schema, &options)
            .diagnostics
            .iter()
            .any(|d| d.code == "invalid_command")
    );
    let (scene, schema, options) = fixture();
    let mut run = start(graph(&scene, &schema, &options), &scene);
    run.advance().unwrap();
    let Yield::GameCommand { token, .. } =
        run.choose(&scene.beats[0].choices[0].id.to_string()).unwrap()
    else {
        panic!()
    };
    run.complete_command(&token, success()).unwrap();
    let before = run.snapshot();
    let changed = CommandResult::Success {
        host_inputs: BTreeMap::from([(name("has_logbook"), Value::Bool(false))]),
    };
    assert_eq!(run.complete_command(&token, changed), Err(Error::InvalidCommand));
    assert_eq!(run.snapshot(), before);
}

#[test]
fn blank_choice_labels_warn_during_authoring_and_block_release_at_the_choice() {
    let (mut scene, schema, mut options) = fixture();
    scene.beats[0].dialogue[0].variants[0].text.lifecycle.review = ReviewState::Approved;
    scene.beats[0].choices[0].label = " \t\n ".into();
    let site = Site::Destination(DestinationSite::Choice {
        beat: scene.beats[0].id,
        choice: scene.beats[0].choices[0].id,
    });
    for (profile, expected_severity) in
        [(Profile::Development, Severity::Warning), (Profile::Release, Severity::Error)]
    {
        options.profile = profile;
        let report = compile(&[scene.clone()], &schema, &options);
        assert_eq!(report.graph.is_some(), profile == Profile::Development);
        assert_eq!(report.diagnostics.len(), 1);
        let diagnostic = &report.diagnostics[0];
        assert_eq!(diagnostic.code, "missing_choice_text");
        assert_eq!(diagnostic.scene, scene.id.to_string());
        assert_eq!(diagnostic.site, site);
        assert_eq!(diagnostic.severity, expected_severity);
    }
    scene.beats[0].choices[0].label = "Present evidence".into();
    assert!(compile(&[scene], &schema, &options).graph.is_some());
}
