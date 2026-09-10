//! The Ashfall council hearing: one original scene, built through the public
//! API, shared by every integration test here.
//!
//! Written out in full rather than loaded from a checked-in YAML file, because
//! the ids in it have to be freshly minted for the identity tests to mean
//! anything — a fixture with hard-coded ids would pass an "ids are preserved"
//! test by accident.

#![allow(dead_code)]

use wobu_narrative::{
    Beat, Choice, CompareOp, Comparison, Condition, Destination, DialogueSlot, Effect, EntityId,
    HostCommand, Increment, Intent, Name, Operand, Outcome, Owner, Participant, Scene, Speaker,
    StateSchema, Text, Value, VarType, VariableDecl, Variant,
};

pub fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}

pub fn var(n: &str, ty: VarType, default: Value, owner: Owner) -> VariableDecl {
    VariableDecl { name: name(n), ty, default, owner, description: String::new() }
}

/// The declared state the hearing branches on.
pub fn ashfall_state() -> StateSchema {
    StateSchema::new([
        var(
            "beacon_quest",
            VarType::Enum { members: vec![name("investigating"), name("resolved")] },
            Value::Enum(name("investigating")),
            Owner::Narrative,
        ),
        var("trust", VarType::Int { min: -100, max: 100 }, Value::Int(0), Owner::Narrative),
        var("support", VarType::Int { min: -100, max: 100 }, Value::Int(0), Owner::Narrative),
        var("has_logbook", VarType::Bool, Value::Bool(false), Owner::Narrative),
        // Supplied by the game, never written by the narrative. Present so the
        // host-owned rule has something real to be checked against.
        var(
            "difficulty",
            VarType::Enum { members: vec![name("story"), name("harsh")] },
            Value::Enum(name("story")),
            Owner::Host,
        ),
    ])
    .unwrap()
}

pub struct Council {
    pub scene: Scene,
    pub kael: EntityId,
    pub mira: EntityId,
    pub orren: EntityId,
}

fn compare(v: &str, op: CompareOp, value: Operand) -> Condition {
    Condition::Compare(Comparison { var: name(v), op, value })
}

/// A complete, playable-looking, entirely handwritten scene: three beats, two
/// routes that reconverge on the verdict, a state-changing outcome and a host
/// command.
pub fn council_hearing() -> Council {
    let kael = wobu_core::new_id();
    let mira = wobu_core::new_id();
    let orren = wobu_core::new_id();

    let mut verdict = Beat::new("Verdict");
    verdict.dialogue.push({
        let mut slot = DialogueSlot::new(Speaker::Entity(orren));
        slot.variants.push(Variant::new(Text::written_locked(
            "The council finds for the witness.\nDismissed.",
        )));
        slot
    });
    verdict
        .outcomes
        .push(Outcome::new(Destination::End { label: "Council rules for the player".to_string() }));

    let mut challenge = Beat::new("Challenge the captain");
    challenge.intents.push(Intent {
        subject: Speaker::Entity(mira),
        intent: "Back the player without naming her own source".to_string(),
    });
    challenge.dialogue.push({
        let mut slot = DialogueSlot::new(Speaker::Entity(mira));
        // Two wordings of one line: same slot, different states, separate
        // identities.
        slot.variants.push(Variant {
            when: Some(compare("trust", CompareOp::Ge, Operand::Literal(Value::Int(40)))),
            ..Variant::new(Text::written("I was there. Kael tells it straight — “ash and all”."))
        });
        slot.variants.push(Variant::new(Text::written("I heard something. That is all I heard.")));
        slot
    });
    challenge.outcomes.push(Outcome {
        effects: vec![Effect::Add(Increment { var: name("support"), by: 5 })],
        ..Outcome::new(Destination::Beat(verdict.id))
    });

    let mut present = Beat::new("Present evidence");
    present.intents.push(Intent {
        subject: Speaker::Player,
        intent: "Accuse the captain of ordering the beacon burned".to_string(),
    });
    present.intents.push(Intent {
        subject: Speaker::Entity(orren),
        intent: "Stay sceptical; ask for something he can hold".to_string(),
    });
    present.must_convey.push("The beacon burned on the captain's watch".to_string());
    present.must_not_reveal.push("That Mira was inside the beacon house".to_string());
    present.dialogue.push({
        let mut slot = DialogueSlot::new(Speaker::Entity(kael));
        slot.variants.push(Variant::new(Text::written(
            "The beacon burned on her watch.\n\nI have the logbook — “Ashfall, third bell” — \
             and the hand that wrote it.\n\tTake it.\n",
        )));
        slot
    });
    // Deliberately left without text: US-02's "missing prose is a task, not
    // something generated on save".
    present.dialogue.push(DialogueSlot::new(Speaker::Entity(orren)));
    present.choices.push(Choice {
        requires: Some(compare("has_logbook", CompareOp::Eq, Operand::Literal(Value::Bool(true)))),
        effects: vec![Effect::Add(Increment { var: name("support"), by: 10 })],
        ..Choice::new("Show the logbook", Destination::Beat(challenge.id))
    });
    present.choices.push(Choice {
        effects: vec![
            Effect::Add(Increment { var: name("trust"), by: -10 }),
            Effect::Command(HostCommand {
                name: name("start_combat"),
                args: vec![Operand::Var(name("difficulty"))],
            }),
        ],
        ..Choice::new("Threaten the captain", Destination::Beat(verdict.id))
    });

    let mut scene = Scene::new("Council hearing");
    scene.summary = "The council hears the case of the burned beacon.".to_string();
    scene.participants = vec![
        Participant { entity: kael, role: "accuser".to_string() },
        Participant { entity: mira, role: "witness".to_string() },
        Participant { entity: orren, role: "sceptic".to_string() },
    ];
    scene.entry = Some(compare(
        "beacon_quest",
        CompareOp::Eq,
        Operand::Literal(Value::Enum(name("investigating"))),
    ));
    scene.beats = vec![present, challenge, verdict];

    Council { scene, kael, mira, orren }
}
