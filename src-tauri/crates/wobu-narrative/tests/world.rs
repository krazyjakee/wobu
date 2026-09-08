use std::collections::BTreeSet;
use wobu_narrative::*;

fn fixture() -> WorldDocument {
    let fact = Fact {
        id: wobu_core::new_id(),
        name: "Attack".into(),
        assertion: "The machines attacked the city.".into(),
        sources: vec!["Chapter 1".into()],
        entity_ids: vec![],
    };
    let character = wobu_core::new_id();
    let claim = KnowledgeClaim {
        id: wobu_core::new_id(),
        name: "Mistaken witness".into(),
        character,
        fact: fact.id,
        belief: Belief::False,
        provenance: KnowledgeProvenance::Rumour { source: "A traveller".into() },
        when: Condition::Never,
    };
    WorldDocument { facts: vec![fact], knowledge: vec![claim], ..WorldDocument::default() }
}

#[test]
fn false_unknown_and_provenance_round_trip_without_changing_canonical_truth() {
    let mut world = fixture();
    let mut unknown = world.knowledge[0].clone();
    unknown.id = wobu_core::new_id();
    unknown.belief = Belief::Unknown;
    unknown.provenance = KnowledgeProvenance::Told { by: wobu_core::new_id() };
    world.knowledge.push(unknown);
    let yaml = world.to_yaml().unwrap();
    assert_eq!(WorldDocument::parse(&yaml).unwrap(), world);
    assert!(yaml.contains("false"));
    assert_eq!(world.facts[0].assertion, "The machines attacked the city.");
}

#[test]
fn shape_and_version_errors_are_rejected_at_read_and_write_boundaries() {
    for yaml in
        ["facts: []", "schema_version: 9\nunknown: true", "schema_version: 1\nunknown: true"]
    {
        assert!(WorldDocument::parse(yaml).is_err(), "{yaml}");
    }
    let mut world = fixture();
    world.schema_version = 9;
    assert!(world.to_yaml().is_err());
    world.schema_version = 1;
    let yaml = world.to_yaml().unwrap().replace("assertion:", "assertionn:");
    assert!(WorldDocument::parse(&yaml).is_err());
}

#[test]
fn draft_diagnostics_preserve_dangling_links_and_directed_relationships() {
    let mut world = fixture();
    let a = world.knowledge[0].character;
    let b = wobu_core::new_id();
    world.relationships.push(Relationship {
        id: wobu_core::new_id(),
        name: "Distrust".into(),
        from: a,
        to: b,
        kind: Name::new("trust").unwrap(),
        value: Value::Int(-10),
        when: Condition::Always,
    });
    let known = BTreeSet::from([a, b]);
    assert!(world.diagnose(&StateSchema::default(), &known, &known, &BTreeSet::new()).is_empty());
    world.facts.clear();
    let issues = world.diagnose(&StateSchema::default(), &known, &known, &BTreeSet::new());
    assert!(issues.iter().any(|issue| issue.field == "fact"));
    assert_eq!(world.relationships.len(), 1, "No reverse relationship is manufactured");
    assert_eq!(WorldDocument::parse(&world.to_yaml().unwrap()).unwrap(), world);
}

#[test]
fn duplicate_ids_invalid_quest_stages_and_undeclared_conditions_are_diagnostics() {
    let mut world = fixture();
    world.quests.push(Quest {
        id: world.facts[0].id,
        name: "Quest".into(),
        summary: String::new(),
        stages: vec![Name::new("started").unwrap()],
        initial: Name::new("missing").unwrap(),
        transitions: vec![QuestTransition {
            from: Name::new("started").unwrap(),
            to: Name::new("missing").unwrap(),
            when: Condition::Compare(Comparison {
                var: Name::new("unknown_variable").unwrap(),
                op: CompareOp::Eq,
                value: Operand::Literal(Value::Bool(true)),
            }),
        }],
        scene_ids: vec![SceneId::new()],
    });
    let issues = world.diagnose(
        &StateSchema::default(),
        &BTreeSet::new(),
        &BTreeSet::new(),
        &BTreeSet::new(),
    );
    for field in ["id", "initial", "transitions.to", "transitions.when", "scene_ids"] {
        assert!(issues.iter().any(|issue| issue.field == field), "{field}: {issues:?}");
    }
}

#[test]
fn future_restriction_requires_an_explicit_release_condition() {
    let mut world = fixture();
    world.restrictions.push(FutureRestriction {
        id: wobu_core::new_id(),
        name: "Keep secret".into(),
        fact: world.facts[0].id,
        characters: vec![],
        until: Condition::Never,
    });
    let yaml = world.to_yaml().unwrap();
    assert_eq!(WorldDocument::parse(&yaml).unwrap(), world);
    assert!(WorldDocument::parse(&yaml.replace("  until: never\n", "")).is_err());
}

#[test]
fn three_characters_hold_independent_accounts_of_one_event_with_place_backlinks() {
    let mut world = fixture();
    world.knowledge.clear();
    let [kael, mira, orren, citadel] = std::array::from_fn(|_| wobu_core::new_id());
    world.facts[0].entity_ids.push(citadel);
    for (character, belief, provenance) in [
        (kael, Belief::True, KnowledgeProvenance::Witnessed),
        (mira, Belief::True, KnowledgeProvenance::Told { by: kael }),
        (orren, Belief::False, KnowledgeProvenance::Rumour { source: "Dockside gossip".into() }),
    ] {
        world.knowledge.push(KnowledgeClaim {
            id: wobu_core::new_id(),
            name: "Account".into(),
            character,
            fact: world.facts[0].id,
            belief,
            provenance,
            when: Condition::Always,
        });
    }
    let characters = BTreeSet::from([kael, mira, orren]);
    let entities = BTreeSet::from([kael, mira, orren, citadel]);
    assert!(
        world
            .diagnose(&StateSchema::default(), &characters, &entities, &BTreeSet::new())
            .is_empty()
    );
    let issues =
        world.diagnose(&StateSchema::default(), &characters, &characters, &BTreeSet::new());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].field, "entity_ids");
    assert_eq!(WorldDocument::parse(&world.to_yaml().unwrap()).unwrap(), world);
}
