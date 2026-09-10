//! Explicit twelve-configuration/seven-witness example, not a claim about arbitrary games.
use crate::*;
use wobu_narrative::*;
pub fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
pub fn eq(n: &str, value: Value) -> Condition {
    Condition::Compare(Comparison {
        var: name(n),
        op: CompareOp::Eq,
        value: Operand::Literal(value),
    })
}
pub fn config(permit: bool, stage: &str, evidence: i64) -> State {
    State::from([
        (name("permit"), Value::Bool(permit)),
        (name("stage"), Value::Enum(name(stage))),
        (name("evidence"), Value::Int(evidence)),
    ])
}
fn when(permit: bool, stage: &str, evidence: i64) -> Condition {
    Condition::All(
        config(permit, stage, evidence).into_iter().map(|(n, v)| eq(n.as_str(), v)).collect(),
    )
}
fn set(n: &str, v: Value) -> Effect {
    Effect::Set(Assignment { var: name(n), value: Operand::Literal(v) })
}
pub struct Fixture {
    pub scenes: Vec<Scene>,
    pub schema: StateSchema,
    pub world: WorldDocument,
    pub policy: Policy,
}
impl Fixture {
    pub fn input(&self) -> Input<'_> {
        Input { scenes: &self.scenes, schema: &self.schema, world: &self.world }
    }
}
pub fn fixture() -> Fixture {
    let schema = StateSchema::new([
        VariableDecl {
            name: name("permit"),
            ty: VarType::Bool,
            default: Value::Bool(false),
            owner: Owner::Narrative,
            description: String::new(),
        },
        VariableDecl {
            name: name("stage"),
            ty: VarType::Enum { members: vec![name("arrival"), name("hearing"), name("departed")] },
            default: Value::Enum(name("arrival")),
            owner: Owner::Narrative,
            description: String::new(),
        },
        VariableDecl {
            name: name("evidence"),
            ty: VarType::Int { min: 0, max: 1 },
            default: Value::Int(0),
            owner: Owner::Narrative,
            description: String::new(),
        },
    ])
    .unwrap();
    let mut scene = Scene::new("Declared hearing model");
    let mut beat = Beat::new("Hearing");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    // These authored conditions make all three distinctions relevant. Order is intentional.
    for condition in [
        eq("permit", Value::Bool(true)),
        eq("stage", Value::Enum(name("hearing"))),
        eq("evidence", Value::Int(1)),
        Condition::Always,
    ] {
        let mut variant = Variant::new(Text::written("A prepared line"));
        variant.when = Some(condition);
        slot.variants.push(variant);
    }
    beat.dialogue.push(slot);
    let target = Target { scene: scene.id, beat: beat.id };
    scene.beats.push(beat);
    let mut world = WorldDocument::default();
    let quest = Quest {
        id: EntityId::generate(),
        name: "Hearing stages".into(),
        summary: String::new(),
        stages: vec![name("arrival").into(), name("hearing").into(), name("departed").into()],
        initial: name("arrival"),
        scene_ids: vec![scene.id],
        transitions: vec![
            QuestTransition {
                from: name("arrival"),
                to: name("hearing"),
                when: when(true, "arrival", 1),
            },
            QuestTransition {
                from: name("hearing"),
                to: name("departed"),
                when: when(false, "hearing", 1),
            },
        ],
    };
    let quest_id = quest.id;
    world.quests.push(quest);
    let mut events = Vec::new();
    for (label, gate, effects) in [
        ("Grant", when(false, "arrival", 0), vec![set("permit", Value::Bool(true))]),
        (
            "Find evidence",
            when(true, "arrival", 0),
            vec![Effect::Add(Increment { var: name("evidence"), by: 1 })],
        ),
        ("Revoke", when(true, "hearing", 1), vec![set("permit", Value::Bool(false))]),
        ("Restore", when(false, "departed", 1), vec![set("permit", Value::Bool(true))]),
    ] {
        let event = WorldEvent {
            id: EntityId::generate(),
            name: label.into(),
            summary: String::new(),
            fact_ids: vec![],
            entity_ids: vec![],
            when: gate,
        };
        events.push(EventBinding { event: event.id, effects });
        world.events.push(event);
    }
    let policy = Policy {
        version: VERSION,
        target,
        initial: vec![config(false, "arrival", 0)],
        invariants: vec![
            Condition::Any(vec![
                eq("stage", Value::Enum(name("arrival"))),
                eq("evidence", Value::Int(1)),
            ]),
            Condition::Any(vec![
                Condition::Not(Box::new(eq("stage", Value::Enum(name("arrival"))))),
                eq("permit", Value::Bool(true)),
                eq("evidence", Value::Int(0)),
            ]),
        ],
        quests: vec![QuestBinding { quest: quest_id, variable: name("stage") }],
        events,
        external: false,
        limits: Limits::default(),
    };
    Fixture { scenes: vec![scene], schema, world, policy }
}
