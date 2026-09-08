//! What a world record is allowed to say, and what it is only allowed to say
//! wrongly once somebody has been told.
//!
//! Two rules are being pinned here, and they pull in opposite directions on
//! purpose.
//!
//! A **belief is not a claim about the world.** A character who is certain of
//! something false, and a second record saying the same character is right about
//! it under different circumstances, are both ordinary authoring — the whole
//! point of separating [`Fact`] from [`KnowledgeClaim`] is that canon does not
//! move when somebody is mistaken. Deciding which of two conditioned accounts
//! applies needs a playthrough's state, so this crate must not answer it, and
//! must not report the pair as a contradiction to be repaired.
//!
//! A **reference or a comparison that could never be right is reported.** A
//! rumour attributed to a character who does not exist, or a condition comparing
//! a bounded integer with a boolean, is wrong in every playthrough, and the only
//! moment it is cheap to fix is while the author is still looking at it. Each is
//! addressed to the field responsible so #155's form can put the message beside
//! the input rather than at the top of the page.

use std::collections::BTreeSet;

use wobu_narrative::{
    Belief, CompareOp, Comparison, Condition, EntityId, Fact, KnowledgeClaim, KnowledgeProvenance,
    Name, Operand, Owner, StateSchema, Value, VarType, VariableDecl, WorldDiagnostic,
    WorldDocument,
};

fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}

/// One integer and one boolean, so a condition can be wrong about a type rather
/// than only wrong about a spelling.
fn schema() -> StateSchema {
    StateSchema::new([
        VariableDecl {
            name: name("trust"),
            ty: VarType::Int { min: -100, max: 100 },
            default: Value::Int(0),
            owner: Owner::Narrative,
            description: String::new(),
        },
        VariableDecl {
            name: name("saw_the_attack"),
            ty: VarType::Bool,
            default: Value::Bool(false),
            owner: Owner::Narrative,
            description: String::new(),
        },
    ])
    .unwrap()
}

fn compare(var: &str, op: CompareOp, value: Value) -> Condition {
    Condition::Compare(Comparison { var: name(var), op, value: Operand::Literal(value) })
}

/// One canonical fact and nothing else, ready for claims to be hung off.
fn world() -> WorldDocument {
    WorldDocument {
        facts: vec![Fact {
            id: wobu_core::new_id(),
            name: "Attack".into(),
            assertion: "The machines attacked the city.".into(),
            sources: vec![],
            entity_ids: vec![],
        }],
        ..WorldDocument::default()
    }
}

fn claim(
    character: EntityId,
    fact: EntityId,
    belief: Belief,
    provenance: KnowledgeProvenance,
    when: Condition,
) -> KnowledgeClaim {
    KnowledgeClaim {
        id: wobu_core::new_id(),
        name: "Account".into(),
        character,
        fact,
        belief,
        provenance,
        when,
    }
}

fn diagnose(world: &WorldDocument, characters: &BTreeSet<EntityId>) -> Vec<WorldDiagnostic> {
    world.diagnose(&schema(), characters, characters, &BTreeSet::new())
}

#[test]
fn one_character_may_hold_conditioned_true_and_false_accounts_of_the_same_fact() {
    let mut world = world();
    let fact = world.facts[0].id;
    let orren = wobu_core::new_id();

    // Orren believes the rumour until he sees the wreckage himself. Both records
    // are true statements about Orren; neither is a statement about the attack.
    world.knowledge.push(claim(
        orren,
        fact,
        Belief::False,
        KnowledgeProvenance::Rumour { source: "Dockside gossip".into() },
        Condition::Not(Box::new(compare("saw_the_attack", CompareOp::Eq, Value::Bool(true)))),
    ));
    world.knowledge.push(claim(
        orren,
        fact,
        Belief::True,
        KnowledgeProvenance::Witnessed,
        compare("saw_the_attack", CompareOp::Eq, Value::Bool(true)),
    ));

    // Not a contradiction to be repaired: which account applies is a question
    // about a playthrough's state, and answering it here would mean this crate
    // deciding what a character knows during a run it cannot see.
    assert!(diagnose(&world, &BTreeSet::from([orren])).is_empty());
    // And canon did not move because somebody was wrong about it.
    assert_eq!(world.facts[0].assertion, "The machines attacked the city.");
    assert_eq!(WorldDocument::parse(&world.to_yaml().unwrap()).unwrap(), world);
}

#[test]
fn a_belief_told_by_someone_who_does_not_exist_is_addressed_to_the_teller() {
    let mut world = world();
    let fact = world.facts[0].id;
    let mira = wobu_core::new_id();

    world.knowledge.push(claim(
        mira,
        fact,
        Belief::True,
        // Kael is who Mira heard it from, and Kael is not in this project — a
        // deleted character, or an id pasted from somewhere else.
        KnowledgeProvenance::Told { by: wobu_core::new_id() },
        Condition::Always,
    ));

    let issues = diagnose(&world, &BTreeSet::from([mira]));
    // The claim itself is fine. Saying so at the record level would send the
    // author looking at Mira; the message belongs on the field holding the id
    // that does not resolve.
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].field, "provenance.by");
    assert_eq!(issues[0].record_id, Some(world.knowledge[0].id));
}

#[test]
fn a_world_condition_is_type_checked_and_not_merely_name_checked() {
    // An undeclared variable is the obvious failure and is already refused. The
    // ones worth pinning are the conditions whose every name resolves: they read
    // as correct, and they can never be satisfied.
    let character = wobu_core::new_id();
    let known = BTreeSet::from([character]);

    for (label, when) in [
        // `trust` is an integer; comparing it with a boolean has no answer.
        ("a mismatched type", compare("trust", CompareOp::Eq, Value::Bool(true))),
        // 9,000 is outside the declared range, so nothing could ever equal it.
        ("a literal out of range", compare("trust", CompareOp::Le, Value::Int(9_000))),
        // Booleans are a set of names, not an ordering.
        (
            "an ordering over a boolean",
            compare("saw_the_attack", CompareOp::Gt, Value::Bool(false)),
        ),
    ] {
        let mut world = world();
        world.knowledge.push(claim(
            character,
            world.facts[0].id,
            Belief::True,
            KnowledgeProvenance::Witnessed,
            when,
        ));
        let issues = diagnose(&world, &known);
        assert_eq!(issues.len(), 1, "{label}: {issues:?}");
        assert_eq!(issues[0].field, "when", "{label}");
        // The message is the type error itself, so the author reads what is
        // wrong with the comparison rather than that something is.
        assert!(!issues[0].message.is_empty(), "{label}");
    }
}
