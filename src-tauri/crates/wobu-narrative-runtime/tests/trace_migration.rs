use std::collections::BTreeMap;
use wobu_narrative::*;
use wobu_narrative_compiler::{CompileOptions, Graph, StateVariable, compile};
use wobu_narrative_runtime::{CommandResult, Error, Runtime, TraceEvent, Yield};

fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}
fn fixture() -> (Graph, Scene) {
    let mut scene = Scene::new("Ashfall hearing");
    let mut beat = Beat::new("Evidence");
    let mut slot = DialogueSlot::new(Speaker::Player);
    slot.variants.push(Variant::new(Text::written("I brought proof.")));
    beat.dialogue.push(slot);
    let mut choice = Choice::new("Present proof", Destination::End { label: "Supported".into() });
    choice.requires = Some(Condition::All(vec![
        Condition::Compare(Comparison {
            var: name("logbook"),
            op: CompareOp::Eq,
            value: Operand::Literal(Value::Bool(true)),
        }),
        Condition::Always,
    ]));
    choice.effects = vec![
        Effect::Add(Increment { var: name("trust"), by: 2 }),
        Effect::Command(HostCommand {
            name: name("award"),
            args: vec![Operand::Var(name("trust"))],
        }),
    ];
    beat.choices.push(choice);
    let mut hidden = Choice::new("Unavailable", Destination::End { label: "Unavailable".into() });
    hidden.requires = Some(Condition::All(vec![Condition::Never, Condition::Always]));
    beat.choices.push(hidden);
    scene.beats.push(beat);
    let schema = StateSchema::new([
        VariableDecl {
            name: name("trust"),
            ty: VarType::Int { min: 0, max: 10 },
            default: Value::Int(1),
            owner: Owner::Narrative,
            description: String::new(),
        },
        VariableDecl {
            name: name("logbook"),
            ty: VarType::Bool,
            default: Value::Bool(false),
            owner: Owner::Host,
            description: String::new(),
        },
    ])
    .unwrap();
    let options = CompileOptions {
        commands: BTreeMap::from([(name("award"), vec![VarType::Int { min: 0, max: 10 }])]),
        ..CompileOptions::default()
    };
    (compile(&[scene.clone()], &[], &schema, &options).graph.unwrap(), scene)
}
fn start(graph: Graph, scene: &Scene) -> Runtime {
    Runtime::start(
        graph,
        &scene.id.to_string(),
        BTreeMap::from([(name("logbook"), Value::Bool(true))]),
        "migration-fixture".into(),
        0,
        100,
    )
    .unwrap()
}
fn updated(graph: &Graph) -> Graph {
    let mut graph = graph.clone();
    graph.state.insert(
        name("new_input"),
        StateVariable { ty: VarType::Bool, default: Value::Bool(false), owner: Owner::Host },
    );
    graph
}

#[test]
fn migration_is_opt_in_and_validates_both_graph_identities_and_new_schema() {
    let (old, scene) = fixture();
    let run = start(old.clone(), &scene);
    let new = updated(&old);
    assert!(matches!(Runtime::restore(new.clone(), run.snapshot()), Err(Error::Incompatible)));
    assert!(matches!(
        Runtime::restore_with_migration(old.clone(), new.clone(), run.snapshot(), Ok),
        Err(Error::MissingState(_))
    ));
    let migrated =
        Runtime::restore_with_migration(old.clone(), new.clone(), run.snapshot(), |mut plan| {
            assert_eq!(plan.from_graph_hash, old.hash());
            assert_eq!(plan.to_graph_hash, new.hash());
            plan.state.insert(name("new_input"), Value::Bool(true));
            Ok(plan)
        })
        .unwrap();
    assert_eq!(migrated.current(), run.current());
    assert_eq!(migrated.state()[&name("new_input")], Value::Bool(true));
    let snapshot =
        serde_json::from_value(serde_json::to_value(migrated.snapshot()).unwrap()).unwrap();
    assert!(Runtime::restore(new.clone(), snapshot).is_ok());
    for bad in ["from", "to", "type", "cursor", "unknown"] {
        assert!(
            Runtime::restore_with_migration(
                old.clone(),
                new.clone(),
                run.snapshot(),
                |mut plan| {
                    plan.state.insert(name("new_input"), Value::Bool(true));
                    match bad {
                        "from" => plan.from_graph_hash = "different".into(),
                        "to" => plan.to_graph_hash = "different".into(),
                        "type" => {
                            plan.state.insert(name("trust"), Value::Int(99));
                        }
                        "cursor" => plan.beat = "missing".into(),
                        _ => {
                            plan.state.insert(name("unknown"), Value::Bool(true));
                        }
                    }
                    Ok(plan)
                }
            )
            .is_err(),
            "{bad}"
        );
    }
    let invoked = std::cell::Cell::new(false);
    assert!(
        Runtime::restore_with_migration(new.clone(), new, run.snapshot(), |plan| {
            invoked.set(true);
            Ok(plan)
        })
        .is_err()
    );
    assert!(!invoked.get(), "invalid original identity must fail before the hook runs");
}

#[test]
fn pending_command_migration_preserves_tokens_args_committed_effects_and_retries() {
    let (old, scene) = fixture();
    let mut run = start(old.clone(), &scene);
    run.advance().unwrap();
    let Yield::GameCommand { token, args, .. } =
        run.choose(&scene.beats[0].choices[0].id.to_string()).unwrap()
    else {
        panic!()
    };
    let new = updated(&old);
    let mut migrated =
        Runtime::restore_with_migration(old, new.clone(), run.snapshot(), |mut plan| {
            plan.state.insert(name("new_input"), Value::Bool(true));
            Ok(plan)
        })
        .unwrap();
    assert!(
        matches!(migrated.current().unwrap(), Yield::GameCommand { token: current, args: current_args, .. } if current == token && current_args == args)
    );
    assert_eq!(migrated.state()[&name("trust")], Value::Int(3));
    let before = migrated.snapshot();
    assert_eq!(
        migrated.complete_command(&token, CommandResult::Cancelled),
        Err(Error::CommandCancelled)
    );
    assert_eq!(migrated.snapshot(), before);
    assert!(!migrated.trace().committed);
    let mut restored = Runtime::restore(new.clone(), before).unwrap();
    let result = CommandResult::Success {
        host_inputs: BTreeMap::from([(name("new_input"), Value::Bool(false))]),
    };
    restored.complete_command(&token, result.clone()).unwrap();
    let mut restored = Runtime::restore(new, restored.snapshot()).unwrap();
    let before = restored.snapshot();
    restored.complete_command(&token, result).unwrap();
    assert_eq!(restored.snapshot(), before);
    assert!(matches!(
        restored.trace().records[0].event,
        TraceEvent::CommandResult { repeated: true, .. }
    ));
}

#[test]
fn migration_rejects_pending_signatures_and_destinations_that_no_longer_resolve() {
    let (old, scene) = fixture();
    let mut run = start(old.clone(), &scene);
    run.advance().unwrap();
    run.choose(&scene.beats[0].choices[0].id.to_string()).unwrap();
    let mut new = old.clone();
    new.commands.clear();
    assert!(matches!(
        Runtime::restore_with_migration(old.clone(), new, run.snapshot(), Ok),
        Err(Error::InvalidCommand)
    ));
    let mut removed_source = old.clone();
    removed_source
        .scenes
        .get_mut(&scene.id.to_string())
        .unwrap()
        .beats
        .get_mut(&scene.beats[0].id.to_string())
        .unwrap()
        .choices
        .clear();
    assert!(matches!(
        Runtime::restore_with_migration(old.clone(), removed_source, run.snapshot(), Ok),
        Err(Error::InvalidState(_))
    ));
    let mut saved = serde_json::to_value(run.snapshot()).unwrap();
    saved["phase"]["commands"]["to"] = serde_json::json!({"beat":"missing"});
    assert!(
        Runtime::restore_with_migration(
            old.clone(),
            old,
            serde_json::from_value(saved).unwrap(),
            Ok
        )
        .is_err()
    );
}

#[test]
fn traces_report_evaluated_predicates_short_circuit_paths_and_taken_effects_at_stable_ids() {
    let (graph, scene) = fixture();
    let mut run = start(graph, &scene);
    run.advance().unwrap();
    let unavailable = scene.beats[0].choices[1].id.to_string();
    let records: Vec<_> = run
        .trace()
        .records
        .iter()
        .filter(|r| r.site.choice.as_deref() == Some(&unavailable))
        .collect();
    assert_eq!(records.len(), 2); // Never child and All root, skipped Always absent.
    assert!(records.iter().all(|r| matches!(r.event, TraceEvent::Condition { passed: false, .. })));
    assert!(
        !records
            .iter()
            .any(|r| matches!(&r.event, TraceEvent::Condition { path, .. } if path == &[1]))
    );
    run.choose(&scene.beats[0].choices[0].id.to_string()).unwrap();
    let trace = run.trace();
    assert!(trace.committed);
    assert!(trace.records.iter().all(|r| r.site.scene == scene.id.to_string()
        && r.site.beat.as_deref() == Some(&scene.beats[0].id.to_string())
        && r.site.choice.as_deref() == Some(&scene.beats[0].choices[0].id.to_string())));
    assert!(trace.records.iter().any(|r| matches!(r.event, TraceEvent::Transition { .. })));
    let changes: Vec<_> = trace
        .records
        .iter()
        .filter_map(|r| match &r.event {
            TraceEvent::Effect { before, after, .. } => Some((before, after)),
            _ => None,
        })
        .collect();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].0[&name("trust")], Value::Int(1));
    assert_eq!(changes[0].1[&name("trust")], Value::Int(3));
    assert!(!changes[0].0.contains_key(&name("logbook")), "effects copy only relevant variables");
}

#[test]
fn trace_truncation_is_explicit_and_never_changes_playback() {
    let (mut graph, scene) = fixture();
    graph.scenes.get_mut(&scene.id.to_string()).unwrap().entry =
        Some(Condition::All(vec![Condition::Always; 2200]));
    let run = start(graph, &scene);
    assert_eq!(run.trace().records.len(), 2048);
    assert!(run.trace().omitted > 0);
    assert!(matches!(run.current().unwrap(), Yield::Line { .. }));
}
