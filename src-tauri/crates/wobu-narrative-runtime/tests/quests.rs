//! #207. A quest's current stage, and the objective the author wrote for it.
//!
//! The point of the whole feature, stated as a test: a host asking "what should
//! the player be doing" gets a sentence somebody wrote on purpose, rather than
//! the title of a beat the player has not reached.

use std::collections::BTreeMap;

use wobu_narrative::*;
use wobu_narrative_compiler::*;
use wobu_narrative_runtime::{Error, Runtime};

fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}

/// One scene with a single choice that sets `hired`, and a quest whose stages
/// follow it.
fn fixture(objectives: &[(&str, &str)]) -> (Scene, StateSchema, Quest) {
    let mut beat = Beat::new("Rosa sizes her up");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written("The griddle is already hot.")));
    beat.dialogue.push(slot);
    let mut take = Choice::new("Take the apron", Destination::End { label: "Hired".into() });
    take.effects = vec![Effect::Set(Assignment {
        var: name("hired"),
        value: Operand::Literal(Value::Bool(true)),
    })];
    beat.choices.push(take);
    let mut scene = Scene::new("A shift at the diner");
    scene.beats.push(beat);

    let schema = StateSchema::new([VariableDecl {
        name: name("hired"),
        ty: VarType::Bool,
        default: Value::Bool(false),
        owner: Owner::Narrative,
        description: String::new(),
    }])
    .unwrap();

    let stages = ["available", "working", "completed"]
        .into_iter()
        .map(|stage| {
            let mut one = QuestStage::new(name(stage));
            if let Some((_, body)) = objectives.iter().find(|(which, _)| *which == stage) {
                one.objective = Some(QuestObjective::written(*body));
            }
            one
        })
        .collect();
    let quest = Quest {
        id: EntityId::generate(),
        name: "A shift at the diner".into(),
        summary: String::new(),
        stages,
        initial: name("available"),
        transitions: vec![QuestTransition {
            from: name("available"),
            to: name("working"),
            when: Condition::Compare(Comparison {
                var: name("hired"),
                op: CompareOp::Eq,
                value: Operand::Literal(Value::Bool(true)),
            }),
        }],
        scene_ids: vec![scene.id],
    };
    (scene, schema, quest)
}

fn graph(
    scene: &Scene,
    schema: &StateSchema,
    quests: Vec<Quest>,
    profile: Profile,
) -> CompileReport {
    compile(
        std::slice::from_ref(scene),
        &[],
        schema,
        &CompileOptions { profile, quests, ..CompileOptions::default() },
    )
}

fn start(graph: Graph, scene: &Scene) -> Runtime {
    Runtime::start(graph, &scene.id.to_string(), BTreeMap::new(), "playthrough-1".into(), 0, 64)
        .unwrap()
}

#[test]
fn playing_a_transition_changes_the_objective_the_runtime_reports() {
    let (scene, schema, quest) = fixture(&[
        ("available", "Find Rosa at the diner and ask about work."),
        ("working", "Get through the lunch rush."),
        ("completed", "Collect your pay."),
    ]);
    let quest_id = quest.id.to_string();
    let report = graph(&scene, &schema, vec![quest], Profile::Development);
    let mut runtime = start(report.graph.expect("a fully authored quest compiles"), &scene);

    // Before anything is played, the initial stage's objective.
    assert_eq!(runtime.quests()[&quest_id], name("available"));
    let first = runtime.objectives();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].text, "Find Rosa at the diner and ask about work.");
    // Reading does not advance: the same answer twice.
    assert_eq!(runtime.objectives(), first);

    // Walk to the choice and take it. The effect sets `hired`, the transition
    // fires, and the objective becomes the one for the stage the player is in.
    runtime.advance().unwrap();
    let choice = match runtime.current().unwrap() {
        wobu_narrative_runtime::Yield::Choices { choices, .. } => choices[0].id.clone(),
        other => panic!("expected a choice, got {other:?}"),
    };
    runtime.choose(&choice).unwrap();

    assert_eq!(runtime.quests()[&quest_id], name("working"));
    assert_eq!(runtime.objectives()[0].text, "Get through the lunch rush.");
    // And the id is the string table key, so a translated build looks it up the
    // same way it looks up a line of dialogue.
    assert!(runtime.objectives()[0].id.is_some());
}

#[test]
fn a_reachable_stage_with_no_objective_is_a_task_in_development_and_a_blocker_in_release() {
    // The same rule a dialogue slot with no wording obeys, for the same reason.
    let (scene, schema, quest) = fixture(&[("available", "Find Rosa at the diner.")]);

    let development = graph(&scene, &schema, vec![quest.clone()], Profile::Development);
    let warnings: Vec<_> =
        development.diagnostics.iter().filter(|d| d.code == "missing_objective").collect();
    assert_eq!(warnings.len(), 1, "{:?}", development.diagnostics);
    assert_eq!(warnings[0].severity, Severity::Warning);
    assert_eq!(warnings[0].quest.as_deref(), Some(quest.id.to_string().as_str()));
    assert_eq!(warnings[0].stage.as_ref(), Some(&name("working")));
    assert!(development.graph.is_some(), "Development must still compile");

    let release = graph(&scene, &schema, vec![quest], Profile::Release);
    assert!(
        release
            .diagnostics
            .iter()
            .any(|d| d.code == "missing_objective" && d.severity == Severity::Error)
    );
    // An error of any kind means no graph, so a Release build cannot ship a quest
    // with a stage it has nothing to show for. This fixture's dialogue is also
    // unapproved, which Release gates too — the claim under test is the severity
    // above, not which of the two refused first.
    assert!(release.graph.is_none());
}

#[test]
fn a_stage_nothing_leads_to_is_not_required_to_have_one() {
    // Requiring wording for a stage that cannot be entered would be busywork with
    // no symptom. `completed` has no transition into it in this fixture.
    let (scene, schema, mut quest) =
        fixture(&[("available", "Find Rosa."), ("working", "Get through the rush.")]);
    quest.transitions.retain(|t| t.to == name("working"));
    let report = graph(&scene, &schema, vec![quest], Profile::Release);
    assert!(
        !report.diagnostics.iter().any(|d| d.code == "missing_objective"),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn the_stage_a_quest_reached_survives_a_save_and_a_restore() {
    let (scene, schema, quest) = fixture(&[
        ("available", "Find Rosa."),
        ("working", "Get through the rush."),
        ("completed", "Collect your pay."),
    ]);
    let quest_id = quest.id.to_string();
    let compiled = graph(&scene, &schema, vec![quest], Profile::Development).graph.unwrap();
    let mut runtime = start(compiled.clone(), &scene);
    runtime.advance().unwrap();
    let choice = match runtime.current().unwrap() {
        wobu_narrative_runtime::Yield::Choices { choices, .. } => choices[0].id.clone(),
        other => panic!("expected a choice, got {other:?}"),
    };
    runtime.choose(&choice).unwrap();

    let saved = runtime.snapshot();
    let round_tripped: wobu_narrative_runtime::Snapshot =
        serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
    let restored = Runtime::restore(compiled, round_tripped).unwrap();
    assert_eq!(restored.quests()[&quest_id], name("working"));
    assert_eq!(restored.objectives()[0].text, "Get through the rush.");
}

#[test]
fn a_project_with_no_quests_compiles_and_saves_exactly_as_it_did_before() {
    // The guarantee existing saves, packages and scenario tapes depend on.
    let (scene, schema, _) = fixture(&[]);
    let without = graph(&scene, &schema, Vec::new(), Profile::Development).graph.unwrap();
    assert!(without.quests.is_empty());
    assert!(!String::from_utf8_lossy(&without.canonical_bytes()).contains("quests"));

    let runtime = start(without, &scene);
    assert!(runtime.quests().is_empty());
    assert!(runtime.objectives().is_empty());
    let json = serde_json::to_string(&runtime.snapshot()).unwrap();
    assert!(!json.contains("quests"), "{json}");
}

#[test]
fn a_save_whose_quest_cursor_disagrees_with_its_graph_is_refused() {
    let (scene, schema, quest) = fixture(&[
        ("available", "Find Rosa."),
        ("working", "Get through the rush."),
        ("completed", "Collect your pay."),
    ]);
    let compiled = graph(&scene, &schema, vec![quest], Profile::Development).graph.unwrap();
    let runtime = start(compiled.clone(), &scene);
    let mut tampered: serde_json::Value = serde_json::to_value(runtime.snapshot()).unwrap();
    let quests = tampered["quests"].as_object_mut().unwrap();
    let key = quests.keys().next().unwrap().clone();
    quests[&key] = serde_json::json!("abandoned");

    let snapshot: wobu_narrative_runtime::Snapshot = serde_json::from_value(tampered).unwrap();
    assert!(matches!(Runtime::restore(compiled, snapshot), Err(Error::InvalidState(_))));
}

#[test]
fn a_chain_of_transitions_settles_in_one_pass_and_a_cycle_does_not_spin() {
    let (scene, schema, mut quest) = fixture(&[
        ("available", "Find Rosa."),
        ("working", "Get through the rush."),
        ("completed", "Collect your pay."),
    ]);
    // available -> working -> completed, both unconditional, plus a cycle back.
    quest.transitions = vec![
        QuestTransition { from: name("available"), to: name("working"), when: Condition::Always },
        QuestTransition { from: name("working"), to: name("completed"), when: Condition::Always },
        QuestTransition { from: name("completed"), to: name("available"), when: Condition::Always },
    ];
    let quest_id = quest.id.to_string();
    let compiled = graph(&scene, &schema, vec![quest], Profile::Development).graph.unwrap();
    let runtime = start(compiled, &scene);
    // Bounded by the number of stages, so it settles rather than looping forever.
    // Three stages, three advances from `available`, which is where it lands.
    assert_eq!(runtime.quests()[&quest_id], name("available"));
}
