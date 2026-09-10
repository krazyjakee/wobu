use std::collections::BTreeSet;
use wobu_narrative::*;

fn name(raw: &str) -> Name {
    Name::new(raw).unwrap()
}

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
        stages: vec![Name::new("started").unwrap().into()],
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

#[test]
fn nested_world_conditions_and_legacy_provenance_share_canonical_map_source() {
    let mut world = fixture();
    world.knowledge[0].when = Condition::Not(Box::new(Condition::Compare(Comparison {
        var: Name::new("secret_known").unwrap(),
        op: CompareOp::Eq,
        value: Operand::Literal(Value::Bool(true)),
    })));
    world.restrictions.push(FutureRestriction {
        id: wobu_core::new_id(),
        name: "Secret".into(),
        fact: world.facts[0].id,
        characters: vec![],
        until: Condition::Not(Box::new(Condition::Never)),
    });
    let yaml = world.to_yaml().unwrap();
    assert!(yaml.contains("not:"));
    assert!(yaml.contains("rumour:"));
    assert_eq!(WorldDocument::parse(&yaml).unwrap(), world);
    let mixed = yaml.replace("provenance:\n    rumour:", "provenance: !rumour");
    assert_ne!(mixed, yaml);
    assert_eq!(WorldDocument::parse(&mixed).unwrap(), world);
    assert_eq!(WorldDocument::parse(&mixed).unwrap().to_yaml().unwrap(), yaml);
    let old = fixture();
    let legacy = serde_norway::to_string(&old).unwrap();
    assert!(legacy.contains("!rumour"));
    assert_eq!(WorldDocument::parse(&legacy).unwrap(), old);
}

/// #207. A stage is a bare name or a name with player-facing wording, and the
/// bare one is written back bare.
#[test]
fn quest_stages_accept_both_shapes_and_round_trip_in_the_one_they_were_written_in() {
    let bare = "\
schema_version: 2
quests:
  - id: 01J00000000000000000000001
    name: A shift at the diner
    summary: Rosa puts her to work.
    stages:
      - available
      - completed
    initial: available
";
    let document = WorldDocument::parse(bare).unwrap();
    assert_eq!(document.quests[0].stage_names(), [name("available"), name("completed")]);
    assert!(document.quests[0].stages.iter().all(|stage| stage.objective.is_none()));
    // Byte-identical: an existing World file nobody has added an objective to is
    // a file a collaborator merges, and opening it must not rewrite it.
    assert!(document.to_yaml().unwrap().contains("- available\n"));
    assert_eq!(WorldDocument::parse(&document.to_yaml().unwrap()).unwrap(), document);

    let mut authored = document.clone();
    authored.quests[0].stages[0].objective =
        Some(QuestObjective::written("Find Rosa at the diner and ask about work."));
    let yaml = authored.to_yaml().unwrap();
    assert!(yaml.contains("Find Rosa at the diner"), "{yaml}");
    assert_eq!(WorldDocument::parse(&yaml).unwrap(), authored);
    // The stage nobody wrote for is still a bare name in the same file.
    assert!(yaml.contains("- completed\n"), "{yaml}");
}

#[test]
fn a_version_one_world_cannot_carry_objectives_in_either_direction() {
    let v1 = "\
schema_version: 1
quests:
  - id: 01J00000000000000000000001
    name: Quest
    summary: ''
    stages:
      - name: available
        objective:
          id: 01J00000000000000000000009
          text:
            revision: '00000000000000000000000000000000'
            body: Find Rosa.
    initial: available
";
    assert!(WorldDocument::parse(v1).unwrap_err().to_string().contains("version 2"));

    let mut document = WorldDocument {
        schema_version: 1,
        quests: vec![Quest {
            id: wobu_core::Id::generate(),
            name: "Quest".into(),
            summary: String::new(),
            stages: vec![name("available").into()],
            initial: name("available"),
            transitions: vec![],
            scene_ids: vec![],
        }],
        ..Default::default()
    };
    // Bare stages are version-1 shape, so this is still writable.
    assert!(document.to_yaml().is_ok());
    document.quests[0].stages[0].objective = Some(QuestObjective::written("Find Rosa."));
    assert!(document.to_yaml().unwrap_err().to_string().contains("version 2"));
}

/// Reachability is over the authored graph and ignores conditions: answering
/// "unreachable" in the reassuring direction would excuse a missing objective
/// that a player will see.
#[test]
fn reachable_stages_follow_transitions_from_the_initial_stage_and_nothing_else() {
    let quest = Quest {
        id: wobu_core::Id::generate(),
        name: "A shift at the diner".into(),
        summary: String::new(),
        stages: vec![
            name("available").into(),
            name("working").into(),
            name("completed").into(),
            name("orphaned").into(),
        ],
        initial: name("available"),
        transitions: vec![
            QuestTransition {
                from: name("available"),
                to: name("working"),
                when: Condition::Always,
            },
            QuestTransition {
                from: name("working"),
                to: name("completed"),
                when: Condition::Always,
            },
            // Nothing leads into `orphaned`, so requiring wording for it would be
            // busywork with no symptom.
            QuestTransition {
                from: name("orphaned"),
                to: name("completed"),
                when: Condition::Always,
            },
        ],
        scene_ids: vec![],
    };
    assert_eq!(
        quest.reachable_stages().iter().map(|stage| stage.name.clone()).collect::<Vec<_>>(),
        [name("available"), name("working"), name("completed")]
    );
}

#[test]
fn an_objective_that_is_blank_or_whose_revision_drifted_is_reported_against_its_quest() {
    let mut document = WorldDocument {
        schema_version: 2,
        quests: vec![Quest {
            id: wobu_core::Id::generate(),
            name: "Quest".into(),
            summary: String::new(),
            stages: vec![name("available").into(), name("done").into()],
            initial: name("available"),
            transitions: vec![],
            scene_ids: vec![],
        }],
        ..Default::default()
    };
    document.quests[0].stages[0].objective = Some(QuestObjective::written("   "));
    let mut drifted = QuestObjective::written("Find Rosa.");
    drifted.text.body = "Find Rosa at the diner.".into();
    document.quests[0].stages[1].objective = Some(drifted);

    let issues = document.diagnose(
        &StateSchema::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    );
    let objectives: Vec<_> =
        issues.iter().filter(|issue| issue.field == "stages.objective").collect();
    assert_eq!(objectives.len(), 2, "{issues:?}");
    assert!(objectives[0].message.contains("empty objective"), "{:?}", objectives[0]);
    assert!(objectives[1].message.contains("stopped matching"), "{:?}", objectives[1]);
}
