use std::collections::BTreeMap;
use wobu_narrative::*;
use wobu_narrative_context::{Character, Input, Options, Selection, resolve};

fn resolve_asset(
    asset: &TextAsset,
    world: &WorldDocument,
    characters: &BTreeMap<EntityId, Character>,
) -> wobu_narrative_context::FrozenContext {
    let scene = asset.editorial_scene();
    resolve(
        Input { scene: &scene, world, schema: &StateSchema::default(), characters },
        Options {
            selection: Selection {
                scene: scene.id,
                beat: scene.beats[0].id,
                slot: scene.beats[0].dialogue[0].id,
                variant: None,
            },
            state: BTreeMap::new(),
            token_budget: 10000,
        },
    )
}

fn asset(kind: TextKind) -> TextAsset {
    let mut asset = TextAsset::new(kind, "Lantern at the quay", Name::new("quay_arrival").unwrap());
    asset.summary = "Describe the repaired lantern.".into();
    asset.must_convey.push("The quay is open again.".into());
    let mut entry = TextEntry::new("After repair");
    entry.lines.push(DialogueSlot::new(Speaker::Narrator));
    asset.entries.push(entry);
    asset
}

#[test]
fn every_type_has_specific_intent_and_entry_conditions_gate_generation() {
    let mut hashes = std::collections::BTreeSet::new();
    for kind in TextKind::ALL {
        let mut asset = asset(kind);
        let context = resolve_asset(&asset, &WorldDocument::default(), &BTreeMap::new());
        assert!(context.ready, "{:?}", context.diagnostics);
        let fragment =
            context.fragments.iter().find(|fragment| fragment.kind == "supporting_text").unwrap();
        assert_eq!(fragment.data["kind"], serde_json::to_value(kind).unwrap());
        assert_eq!(fragment.data["trigger"]["event"], "quay_arrival");
        hashes.insert(context.hash.clone());
        asset.trigger.event = Name::new("different_host_trigger").unwrap();
        assert_ne!(
            resolve_asset(&asset, &WorldDocument::default(), &BTreeMap::new()).hash,
            context.hash
        );
        asset.entries[0].when = Some(Condition::Never);
        let inactive = resolve_asset(&asset, &WorldDocument::default(), &BTreeMap::new());
        assert!(!inactive.ready);
        assert!(
            inactive.diagnostics.iter().any(|diagnostic| diagnostic.code == "inactive_text_entry")
        );
    }
    assert_eq!(hashes.len(), 6);
}

#[test]
fn explicit_source_links_never_grant_speakers_knowledge_or_reveal_restricted_facts() {
    let fact = Fact {
        id: wobu_core::new_id(),
        name: "Lantern".into(),
        assertion: "The lamp contains a fallen star.".into(),
        sources: vec![],
        entity_ids: vec![],
    };
    let mut world = WorldDocument::default();
    world.facts.push(fact.clone());
    let mut prose = asset(TextKind::Codex);
    prose.sources.push(SourceLink::Fact(fact.id));
    assert!(
        resolve_asset(&prose, &world, &BTreeMap::new())
            .fragments
            .iter()
            .any(|fragment| fragment.kind == "linked_fact")
    );
    let id = wobu_core::new_id();
    let characters = BTreeMap::from([(
        id,
        Character { id, name: "Dockhand".into(), voice: Some("Terse and plain".into()) },
    )]);
    let mut bark = asset(TextKind::Bark);
    bark.participants.push(Participant { entity: id, role: "Dockhand".into() });
    bark.entries[0].lines[0].speaker = Speaker::Entity(id);
    bark.sources = prose.sources.clone();
    let context = resolve_asset(&bark, &world, &characters);
    assert!(context.ready);
    assert!(!context.request.contains(&fact.assertion));
    world.restrictions.push(FutureRestriction {
        id: wobu_core::new_id(),
        name: "Later revelation".into(),
        fact: fact.id,
        characters: vec![],
        until: Condition::Never,
    });
    let context = resolve_asset(&prose, &world, &BTreeMap::new());
    assert!(!context.fragments.iter().any(|fragment| fragment.kind == "linked_fact"));
    assert!(context.fragments.iter().any(|fragment| fragment.kind == "future_restriction"));
    world.facts.clear();
    assert!(!resolve_asset(&prose, &world, &BTreeMap::new()).ready);
}
