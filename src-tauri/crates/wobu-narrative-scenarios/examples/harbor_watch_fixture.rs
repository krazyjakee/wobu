//! Reproduce the original handwritten fixture without running the narrative runtime.
use std::{collections::BTreeMap, path::PathBuf};
use wobu_narrative::*;
use wobu_narrative_runtime::CommandResult;
use wobu_narrative_scenarios::{Action, Assertion, Boundary, ExpectedError, Scenario, Step};
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id(n: u32) -> String {
    format!("{n:026}")
}
fn eq(var: &str, value: Value) -> Condition {
    Condition::Compare(Comparison {
        var: name(var),
        op: CompareOp::Eq,
        value: Operand::Literal(value),
    })
}
fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("provide new fixture directory"));
    assert!(!root.exists(), "refusing to overwrite fixture");
    std::fs::create_dir_all(root.join("scenarios")).unwrap();
    let mut scene = Scene::new("Harbor Watch: the missing light");
    scene.id = id(1).parse().unwrap();
    let mut beat = Beat::new("Report at the seawall");
    beat.id = id(2).parse().unwrap();
    let mut knowledge = DialogueSlot::new(Speaker::Narrator);
    knowledge.id = id(3).parse().unwrap();
    for (n, source, text) in [
        (4, "witnessed", "I watched the north lantern go dark before the bell."),
        (5, "told", "Mara told me the north lantern went dark before the bell."),
        (6, "rumour", "The quay is whispering about a dark lantern. I cannot swear it happened."),
    ] {
        let mut v = Variant::new(Text::written(text));
        v.id = id(n).parse().unwrap();
        v.when = Some(eq("knowledge", Value::Enum(name(source))));
        knowledge.variants.push(v);
    }
    let mut trust = DialogueSlot::new(Speaker::Narrator);
    trust.id = id(7).parse().unwrap();
    for (n, high, text) in [
        (8, true, "You have kept your word before. Take the watch key."),
        (9, false, "Stay on the seawall until we verify the report."),
    ] {
        let mut v = Variant::new(Text::written(text));
        v.id = id(n).parse().unwrap();
        if high {
            v.when = Some(Condition::Compare(Comparison {
                var: name("trust"),
                op: CompareOp::Ge,
                value: Operand::Literal(Value::Int(50)),
            }));
        }
        trust.variants.push(v);
    }
    beat.dialogue = vec![knowledge, trust];
    for (n, label, high) in
        [(10, "Record the report", false), (11, "Lead the lantern search", true)]
    {
        let mut choice = Choice::new(label, Destination::End { label: "Watch assigned".into() });
        choice.id = id(n).parse().unwrap();
        if high {
            choice.requires = Some(Condition::Compare(Comparison {
                var: name("trust"),
                op: CompareOp::Ge,
                value: Operand::Literal(Value::Int(50)),
            }));
        }
        choice.effects = vec![
            Effect::Add(Increment { var: name("trust"), by: 5 }),
            Effect::Command(HostCommand { name: name("record_watch"), args: vec![] }),
        ];
        beat.choices.push(choice);
    }
    scene.beats.push(beat);
    let state = StateDocument::new(vec![
        VariableDecl {
            name: name("trust"),
            ty: VarType::Int { min: 0, max: 100 },
            default: Value::Int(20),
            owner: Owner::Narrative,
            description: String::new(),
        },
        VariableDecl {
            name: name("knowledge"),
            ty: VarType::Enum { members: vec![name("witnessed"), name("told"), name("rumour")] },
            default: Value::Enum(name("rumour")),
            owner: Owner::Host,
            description: String::new(),
        },
        VariableDecl {
            name: name("recorded"),
            ty: VarType::Bool,
            default: Value::Bool(false),
            owner: Owner::Host,
            description: String::new(),
        },
    ]);
    std::fs::write(root.join("scene.yaml"), SceneDocument::new(scene).to_yaml().unwrap()).unwrap();
    std::fs::write(root.join("state.yaml"), state.to_yaml().unwrap()).unwrap();
    for (k, knowledge) in ["witnessed", "told", "rumour"].iter().enumerate() {
        for high in [false, true] {
            let start = if high { 70 } else { 20 };
            let line = |slot, variant| {
                Some(Boundary::Line {
                    scene: Some(id(1)),
                    beat: Some(id(2)),
                    slot: Some(id(slot)),
                    variant: Some(id(variant)),
                })
            };
            let command =
                Some(Boundary::Command { name: Some(name("record_watch")), args: Some(vec![]) });
            let after = BTreeMap::from([
                (name("trust"), Value::Int(start + 5)),
                (name("recorded"), Value::Bool(false)),
            ]);
            let step = |action, boundary, state, error| Step {
                action,
                expect: Assertion { boundary, state, error },
            };
            let scenario = Scenario {
                version: 1,
                scene: id(1).parse().unwrap(),
                initial_state: BTreeMap::from([
                    (name("trust"), Value::Int(start)),
                    (name("knowledge"), Value::Enum(name(knowledge))),
                    (name("recorded"), Value::Bool(false)),
                ]),
                seed: 17,
                commands: BTreeMap::from([(name("record_watch"), vec![])]),
                steps: vec![
                    step(None, line(3, 4 + k as u32), BTreeMap::new(), None),
                    step(
                        Some(Action::Advance),
                        line(7, if high { 8 } else { 9 }),
                        BTreeMap::new(),
                        None,
                    ),
                    step(
                        Some(Action::Advance),
                        Some(Boundary::Choices {
                            scene: None,
                            beat: Some(id(2)),
                            ids: Some(if high { vec![id(10), id(11)] } else { vec![id(10)] }),
                        }),
                        BTreeMap::new(),
                        None,
                    ),
                    step(
                        Some(Action::Choose { choice: id(if high { 11 } else { 10 }) }),
                        command.clone(),
                        after.clone(),
                        None,
                    ),
                    step(Some(Action::SaveCheckpoint), command.clone(), after.clone(), None),
                    step(
                        Some(Action::CompleteCommand {
                            result: CommandResult::Failed { message: "The clerk is away".into() },
                        }),
                        command.clone(),
                        after.clone(),
                        Some(ExpectedError::CommandFailed),
                    ),
                    step(Some(Action::RestoreCheckpoint), command, after, None),
                    step(
                        Some(Action::CompleteCommand {
                            result: CommandResult::Success {
                                host_inputs: BTreeMap::from([(
                                    name("recorded"),
                                    Value::Bool(true),
                                )]),
                            },
                        }),
                        Some(Boundary::End { scene: Some(id(1)), beat: Some(id(2)) }),
                        BTreeMap::from([
                            (name("trust"), Value::Int(start + 5)),
                            (name("recorded"), Value::Bool(true)),
                        ]),
                        None,
                    ),
                ],
            };
            let number = 20 + (k as u32) * 2 + u32::from(high);
            let doc = serde_json::json!({"schema_version":1,"id":id(number),"kind":"scenario","name":format!("{} trust / {knowledge}",if high{"High"}else{"Low"}),"payload":scenario});
            std::fs::write(
                root.join(format!("scenarios/{}.json", id(number))),
                serde_json::to_vec_pretty(&doc).unwrap(),
            )
            .unwrap();
        }
    }
}
