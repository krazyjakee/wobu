//! Write the handwritten Preview walkthrough into a COPY of Ashfall.wobu.
//! Usage: cargo run -p wobu-narrative-runtime --example ashfall_preview_fixture -- /tmp/Ashfall.wobu
use std::path::PathBuf;
use wobu_narrative::*;

fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}
fn end(label: &str) -> Destination {
    Destination::End { label: label.into() }
}
fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("provide a copied project path"));
    assert!(root.join("project.json").is_file(), "use an existing project copy");
    assert!(!root.join("narrative").exists(), "will not overwrite existing narrative data");
    let mut hearing = Scene::new("Ashfall council hearing");
    hearing.id = "00000000000000000000000001".parse().unwrap();
    let mut evidence = Beat::new("Present evidence");
    evidence.id = "00000000000000000000000002".parse().unwrap();
    let mut line = DialogueSlot::new(Speaker::Player);
    line.id = "00000000000000000000000003".parse().unwrap();
    let mut variant =
        Variant::new(Text::written("I brought the logbook. Let the council judge the evidence."));
    variant.id = "00000000000000000000000004".parse().unwrap();
    line.variants.push(variant);
    evidence.dialogue.push(line);
    let mut verdict = Beat::new("Council response");
    verdict.id = "00000000000000000000000010".parse().unwrap();
    let mut response = DialogueSlot::new(Speaker::Narrator);
    response.id = "00000000000000000000000011".parse().unwrap();
    let mut accepted =
        Variant::new(Text::written("The council accepts the record. An investigation will begin."));
    accepted.id = "00000000000000000000000012".parse().unwrap();
    accepted.when = Some(Condition::Compare(Comparison {
        var: name("record_confirmed"),
        op: CompareOp::Eq,
        value: Operand::Literal(Value::Bool(true)),
    }));
    response.variants.push(accepted);
    let mut waiting =
        Variant::new(Text::written("The council is waiting for the clerk to confirm the record."));
    waiting.id = "00000000000000000000000013".parse().unwrap();
    response.variants.push(waiting);
    verdict.dialogue.push(response);
    let mut finish = Outcome::new(end("Hearing complete"));
    finish.id = "00000000000000000000000014".parse().unwrap();
    verdict.outcomes.push(finish);
    let mut present = Choice::new("Present the logbook", Destination::Beat(verdict.id));
    present.id = "00000000000000000000000005".parse().unwrap();
    present.effects = vec![
        Effect::Add(Increment { var: name("trust"), by: 10 }),
        Effect::Command(HostCommand { name: name("file_record"), args: vec![] }),
    ];
    evidence.choices.push(present);
    hearing.beats = vec![evidence, verdict];

    let mut bounded = Scene::new("Ashfall bounded loop check");
    bounded.id = "00000000000000000000000007".parse().unwrap();
    let mut again = Beat::new("Wait for clearance");
    again.id = "00000000000000000000000008".parse().unwrap();
    let mut retry = Outcome::new(Destination::Beat(again.id));
    retry.id = "00000000000000000000000009".parse().unwrap();
    retry.when = Some(Condition::Compare(Comparison {
        var: name("repeat_wait"),
        op: CompareOp::Eq,
        value: Operand::Literal(Value::Bool(true)),
    }));
    let mut clear = Outcome::new(end("Loop guard cleared"));
    clear.id = "00000000000000000000000015".parse().unwrap();
    again.outcomes = vec![retry, clear];
    bounded.beats.push(again);
    let state = StateDocument::new(vec![
        VariableDecl {
            name: name("trust"),
            ty: VarType::Int { min: 0, max: 100 },
            default: Value::Int(40),
            owner: Owner::Narrative,
            description: "Council support".into(),
        },
        VariableDecl {
            name: name("record_confirmed"),
            ty: VarType::Bool,
            default: Value::Bool(false),
            owner: Owner::Host,
            description: "Clerk's command output".into(),
        },
        VariableDecl {
            name: name("repeat_wait"),
            ty: VarType::Bool,
            default: Value::Bool(false),
            owner: Owner::Host,
            description: "Deliberately enable the loop limit demonstration".into(),
        },
    ]);
    std::fs::create_dir_all(root.join("narrative/scenes")).unwrap();
    std::fs::write(root.join("narrative/state.yaml"), state.to_yaml().unwrap()).unwrap();
    for scene in [hearing, bounded] {
        std::fs::write(
            root.join(format!("narrative/scenes/{}.yaml", scene.id)),
            SceneDocument::new(scene).to_yaml().unwrap(),
        )
        .unwrap();
    }
}
