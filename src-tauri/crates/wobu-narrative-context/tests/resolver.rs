use std::collections::BTreeMap;
use wobu_narrative::*;
use wobu_narrative_context::*;

struct Fixture {
    scene: Scene,
    world: WorldDocument,
    schema: StateSchema,
    characters: BTreeMap<EntityId, Character>,
    options: Options,
}
impl Fixture {
    fn new() -> Self {
        let mut scene = Scene::new("Council hearing");
        let mut characters = BTreeMap::new();
        let mut world = WorldDocument::default();
        let fact = Fact {
            id: wobu_core::new_id(),
            name: "Citadel attack".into(),
            assertion: "Geth attacked the Citadel".into(),
            sources: vec!["mission/citadel".into()],
            entity_ids: vec![],
        };
        world.facts.push(fact.clone());
        for (index, name) in ["Garrus", "Liara", "Farmer"].into_iter().enumerate() {
            let id = wobu_core::new_id();
            characters.insert(
                id,
                Character { id, name: name.into(), voice: Some(format!("{name} speaks plainly")) },
            );
            scene.participants.push(Participant { entity: id, role: String::new() });
            let provenance = match index {
                0 => KnowledgeProvenance::Witnessed,
                1 => KnowledgeProvenance::Told { by: scene.participants[0].entity },
                _ => KnowledgeProvenance::Rumour { source: "travellers".into() },
            };
            world.knowledge.push(KnowledgeClaim {
                id: wobu_core::new_id(),
                name: format!("{name} knows"),
                character: id,
                fact: fact.id,
                belief: Belief::True,
                provenance,
                when: Condition::Always,
            });
        }
        let mut beat = Beat::new("Present evidence");
        beat.must_convey.push("Ask for an investigation".into());
        beat.must_not_reveal.push("Do not reveal the collector attack".into());
        let slot = DialogueSlot::new(Speaker::Entity(scene.participants[0].entity));
        let selection = Selection { scene: scene.id, beat: beat.id, slot: slot.id, variant: None };
        beat.dialogue.push(slot);
        scene.beats.push(beat);
        Self {
            scene,
            world,
            schema: StateSchema::default(),
            characters,
            options: Options { selection, state: BTreeMap::new(), token_budget: 4000 },
        }
    }
    fn resolve(&self) -> FrozenContext {
        resolve(
            Input {
                scene: &self.scene,
                world: &self.world,
                schema: &self.schema,
                characters: &self.characters,
            },
            self.options.clone(),
        )
    }
    fn has(result: &FrozenContext, code: &str) -> bool {
        result.diagnostics.iter().any(|d| d.code == code)
    }
}
#[test]
fn three_perspectives_never_inherit_another_characters_knowledge() {
    let mut f = Fixture::new();
    for (index, expected) in ["witnessed", "told", "rumour"].into_iter().enumerate() {
        f.scene.beats[0].dialogue[0].speaker = Speaker::Entity(f.scene.participants[index].entity);
        let result = f.resolve();
        assert!(result.ready);
        let facts: Vec<_> = result.fragments.iter().filter(|f| f.kind == "knowledge").collect();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].data["claim"]["provenance"].to_string().contains(expected));
    }
}
#[test]
fn false_and_unknown_beliefs_are_preserved_without_claiming_truth() {
    let mut f = Fixture::new();
    for belief in [Belief::False, Belief::Unknown] {
        f.world.knowledge[0].belief = belief.clone();
        let result = f.resolve();
        assert!(result.ready);
        let fact = result.fragments.iter().find(|f| f.kind == "knowledge").unwrap();
        assert_eq!(fact.data["claim"]["belief"], serde_json::to_value(belief).unwrap());
        assert_eq!(fact.data["canonical_fact"]["assertion"], "Geth attacked the Citadel");
    }
}
#[test]
fn restricted_facts_stay_required_and_never_become_usable_knowledge() {
    let mut f = Fixture::new();
    f.world.restrictions.push(FutureRestriction {
        id: wobu_core::new_id(),
        name: "Future fact".into(),
        fact: f.world.facts[0].id,
        characters: vec![],
        until: Condition::Never,
    });
    let result = f.resolve();
    assert!(Fixture::has(&result, "forbidden_knowledge"));
    assert!(!result.fragments.iter().any(|f| f.kind == "knowledge"));
    assert!(result.fragments.iter().any(|f| f.kind == "future_restriction" && f.required));
    f.world.restrictions[0].until = Condition::Always;
    assert!(f.resolve().fragments.iter().any(|f| f.kind == "knowledge"));
}
#[test]
fn conflicts_and_deleted_fact_or_told_source_block_without_erasing_diagnostics() {
    let mut f = Fixture::new();
    let mut second = f.world.knowledge[0].clone();
    second.id = wobu_core::new_id();
    second.belief = Belief::False;
    f.world.knowledge.push(second);
    let result = f.resolve();
    assert!(!result.ready);
    assert!(Fixture::has(&result, "conflicting_beliefs"));
    assert_eq!(result.fragments.iter().filter(|f| f.kind == "knowledge").count(), 2);
    f.world.facts.clear();
    let result = f.resolve();
    assert!(Fixture::has(&result, "missing_source"));
    assert!(
        result.dependencies.contains_key(&format!("world/facts/{}", f.world.knowledge[0].fact))
    );
    let mut f = Fixture::new();
    f.world.knowledge[0].provenance = KnowledgeProvenance::Told { by: wobu_core::new_id() };
    assert!(!f.resolve().ready);
}
#[test]
fn required_overflow_retains_constraints_and_optional_truncation_is_explicit() {
    let mut f = Fixture::new();
    f.options.token_budget = 1;
    let result = f.resolve();
    assert!(!result.ready);
    assert!(Fixture::has(&result, "required_overflow"));
    assert!(result.request.contains("Do not reveal the collector attack"));
    assert!(!result.omitted.is_empty());
    f.options.token_budget = 4000;
    let normal = f.resolve();
    let without_optional: Vec<_> =
        normal.fragments.iter().filter(|f| f.required).cloned().collect();
    assert!(!without_optional.is_empty());
    f.world.facts[0].assertion = "Long knowledge ".repeat(3000);
    let result = f.resolve();
    assert!(result.ready);
    assert!(Fixture::has(&result, "truncated"));
    assert!(result.estimated_tokens <= 4000);
}
#[test]
fn empty_queries_and_inactive_members_invalidate_and_attribution_is_deterministic() {
    let mut f = Fixture::new();
    f.world.knowledge.clear();
    let empty = f.resolve();
    assert_eq!(empty, f.resolve());
    assert!(empty.queries.iter().any(|q| q.name == "speaker_knowledge" && q.members.is_empty()));
    f.world.knowledge.push(KnowledgeClaim {
        id: wobu_core::new_id(),
        name: "Not acquired".into(),
        character: f.scene.participants[0].entity,
        fact: f.world.facts[0].id,
        belief: Belief::True,
        provenance: KnowledgeProvenance::Witnessed,
        when: Condition::Never,
    });
    let inactive = f.resolve();
    assert_ne!(empty.hash, inactive.hash);
    assert!(!inactive.fragments.iter().any(|f| f.kind == "knowledge"));
    f.world.knowledge[0].when = Condition::Always;
    let active = f.resolve();
    assert_ne!(inactive.hash, active.hash);
    let old = active.request.clone();
    f.characters.get_mut(&f.scene.participants[0].entity).unwrap().voice =
        Some("Very sarcastic".into());
    assert_ne!(active.hash, f.resolve().hash);
    assert_eq!(active.request, old);
}
#[test]
fn typed_scenario_controls_acquisition_and_future_release_and_rejects_missing_state() {
    let mut f = Fixture::new();
    let name = Name::new("chapter").unwrap();
    f.schema = StateSchema::new([VariableDecl {
        name: name.clone(),
        ty: VarType::Int { min: 0, max: 10 },
        default: Value::Int(0),
        owner: Owner::Narrative,
        description: String::new(),
    }])
    .unwrap();
    f.world.knowledge[0].when = Condition::Compare(Comparison {
        var: name.clone(),
        op: CompareOp::Ge,
        value: Operand::Literal(Value::Int(3)),
    });
    assert!(!f.resolve().ready);
    f.options.state.insert(name.clone(), Value::Int(1));
    assert!(!f.resolve().fragments.iter().any(|f| f.kind == "knowledge"));
    f.options.state.insert(name.clone(), Value::Int(3));
    assert!(f.resolve().fragments.iter().any(|f| f.kind == "knowledge"));
    f.options.state.insert(name, Value::Int(11));
    assert!(!f.resolve().ready);
}
#[test]
fn event_attendance_does_not_imply_knowledge_and_missing_conditions_fail_closed() {
    let mut f = Fixture::new();
    f.world.events.push(WorldEvent {
        id: wobu_core::new_id(),
        name: "Aftermath".into(),
        summary: "The Citadel was attacked".into(),
        fact_ids: vec![f.world.facts[0].id],
        entity_ids: vec![f.scene.participants[0].entity],
        when: Condition::Always,
    });
    assert!(f.resolve().fragments.iter().any(|f| f.kind == "event"));
    f.world.knowledge[0].belief = Belief::Unknown;
    let result = f.resolve();
    assert!(!result.fragments.iter().any(|f| f.kind == "event"));
    assert!(Fixture::has(&result, "unavailable_event"));
    f.world.restrictions.push(FutureRestriction {
        id: wobu_core::new_id(),
        name: "Bad gate".into(),
        fact: f.world.facts[0].id,
        characters: vec![],
        until: Condition::Compare(Comparison {
            var: Name::new("missing").unwrap(),
            op: CompareOp::Eq,
            value: Operand::Literal(Value::Bool(true)),
        }),
    });
    let result = f.resolve();
    assert!(!result.ready);
    assert!(result.fragments.iter().any(|f| f.kind == "future_restriction"));
}
#[test]
fn deleted_selection_and_inactive_variants_are_reviewable_blockers() {
    let mut f = Fixture::new();
    let mut variant = Variant::new(Text::written("Existing wording"));
    variant.when = Some(Condition::Never);
    f.options.selection.variant = Some(variant.id);
    f.scene.beats[0].dialogue[0].variants.push(variant);
    assert!(Fixture::has(&f.resolve(), "inactive_variant"));
    f.scene.beats[0].dialogue.clear();
    assert!(Fixture::has(&f.resolve(), "missing_source"));
}

#[test]
fn irrelevant_scenario_values_never_enter_provider_input_or_consume_its_budget() {
    let mut f = Fixture::new();
    let initial = f.resolve();
    let declarations: Vec<_> = (0..2000)
        .map(|i| VariableDecl {
            name: Name::new(format!("future_secret_{i}")).unwrap(),
            ty: VarType::Bool,
            default: Value::Bool(true),
            owner: Owner::Narrative,
            description: String::new(),
        })
        .collect();
    f.options.state = declarations.iter().map(|d| (d.name.clone(), d.default.clone())).collect();
    f.schema = StateSchema::new(declarations).unwrap();
    let with_state = f.resolve();
    assert!(with_state.ready);
    assert_eq!(initial.request, with_state.request);
    assert_eq!(initial.estimated_tokens, with_state.estimated_tokens);
    assert!(!with_state.request.contains("future_secret"));
    assert_ne!(initial.hash, with_state.hash);
}
#[test]
fn ambiguous_targets_and_shadowed_variants_are_blockers() {
    let mut f = Fixture::new();
    let first = Variant::new(Text::written("First"));
    let later = Variant::new(Text::written("Unreachable"));
    f.options.selection.variant = Some(later.id);
    f.scene.beats[0].dialogue[0].variants = vec![first, later];
    assert!(Fixture::has(&f.resolve(), "shadowed_variant"));
    let duplicate = f.scene.beats[0].dialogue[0].clone();
    f.scene.beats[0].dialogue.push(duplicate);
    assert!(Fixture::has(&f.resolve(), "duplicate_identity"));
}

#[test]
fn directed_relationships_never_reverse_and_conflicting_active_values_block() {
    let mut f = Fixture::new();
    let speaker = f.scene.participants[0].entity;
    let other = f.scene.participants[1].entity;
    let outgoing = Relationship {
        id: wobu_core::new_id(),
        name: "Trust".into(),
        from: speaker,
        to: other,
        kind: Name::new("trust").unwrap(),
        value: Value::Int(71),
        when: Condition::Always,
    };
    let incoming =
        Relationship { id: wobu_core::new_id(), from: other, to: speaker, ..outgoing.clone() };
    f.world.relationships = vec![outgoing.clone(), incoming];
    let result = f.resolve();
    assert_eq!(result.fragments.iter().filter(|f| f.kind == "relationship").count(), 1);
    f.world.relationships.push(Relationship {
        id: wobu_core::new_id(),
        value: Value::Int(10),
        ..outgoing
    });
    assert!(Fixture::has(&f.resolve(), "conflicting_relationships"));
}
