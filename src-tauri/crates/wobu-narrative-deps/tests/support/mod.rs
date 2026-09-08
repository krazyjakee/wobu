//! One project, built in memory, that every test here edits and re-captures.
//!
//! Deliberately small and deliberately complete: a scene with two speakers and
//! two lines, a supporting text asset that cites a fact, and one world record
//! of each kind the resolver queries. Anything smaller stops exercising the
//! difference between "this line read it" and "this line was merely near it",
//! which is the only thing worth testing here.

// Shared by four test binaries, each of which uses a different part of it — the
// same reason `wobu-narrative`'s own fixture module carries this.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use wobu_narrative::{
    Beat, Belief, Condition, DialogueSlot, EntityId, Fact, FutureRestriction, KnowledgeClaim,
    KnowledgeProvenance, Name, Participant, Quest, Relationship, Scene, SourceLink, Speaker,
    StateSchema, Text, TextAsset, TextEntry, TextKind, Value, VarType, VariableDecl, Variant,
    WorldDocument, WorldEvent,
};
use wobu_narrative_context::Character;
use wobu_narrative_deps::{Affected, DependencyIndex, Producer, Snapshot, ToolVersions, capture};

pub struct World {
    /// Named rather than looked up by map order: two ULIDs minted in the same
    /// millisecond sort by their random half, so `characters.keys().next()` is
    /// a coin flip and a test written against it passes for the wrong reason.
    pub kael: EntityId,
    pub mira: EntityId,
    pub scenes: Vec<Scene>,
    pub texts: Vec<TextAsset>,
    pub world: WorldDocument,
    pub schema: StateSchema,
    pub characters: BTreeMap<EntityId, Character>,
    pub producers: BTreeMap<wobu_narrative::VariantId, Producer>,
    pub versions: ToolVersions,
}

impl World {
    pub fn snapshot(&self) -> Snapshot<'_> {
        Snapshot {
            scenes: &self.scenes,
            texts: &self.texts,
            world: &self.world,
            schema: &self.schema,
            characters: &self.characters,
            producers: &self.producers,
            versions: self.versions,
        }
    }

    pub fn index(&self) -> DependencyIndex {
        DependencyIndex::rebuild(capture(&self.snapshot()))
    }

    /// Which lines an edit affects, expressed the way a caller would: capture
    /// before, mutate, capture after, diff.
    pub fn affected(&mut self, edit: impl FnOnce(&mut World)) -> Vec<Affected> {
        let before = self.index();
        edit(self);
        before.diff(&self.index())
    }

    pub fn kael_line(&self) -> wobu_narrative::VariantId {
        self.scenes[0].beats[0].dialogue[0].variants[0].id
    }

    pub fn mira_line(&self) -> wobu_narrative::VariantId {
        self.scenes[0].beats[0].dialogue[1].variants[0].id
    }

    pub fn codex_line(&self) -> wobu_narrative::VariantId {
        self.texts[0].entries[0].lines[0].variants[0].id
    }
}

pub const KAEL: &str = "Kael";
pub const MIRA: &str = "Mira";

pub fn ids() -> (EntityId, EntityId) {
    (wobu_core::new_id(), wobu_core::new_id())
}

pub fn harbour() -> World {
    let (kael, mira) = ids();
    let fact = Fact {
        id: wobu_core::new_id(),
        name: "The lamp was lit".into(),
        assertion: "The harbour lamp was lit on the night of the wreck.".into(),
        sources: vec![],
        entity_ids: vec![],
    };
    let unseen = Fact {
        id: wobu_core::new_id(),
        name: "The tide was wrong".into(),
        assertion: "The tide ran the wrong way that night.".into(),
        sources: vec![],
        entity_ids: vec![],
    };
    let claim = KnowledgeClaim {
        id: wobu_core::new_id(),
        name: "Kael saw the lamp".into(),
        character: kael,
        fact: fact.id,
        belief: Belief::True,
        provenance: KnowledgeProvenance::Witnessed,
        when: Condition::Always,
    };
    let relation = Relationship {
        id: wobu_core::new_id(),
        name: "Kael trusts Mira".into(),
        from: kael,
        to: mira,
        kind: Name::new("trust").unwrap(),
        value: Value::Int(3),
        when: Condition::Always,
    };
    let event = WorldEvent {
        id: wobu_core::new_id(),
        name: "The wreck".into(),
        summary: "A hull broke on the bar.".into(),
        fact_ids: vec![fact.id],
        entity_ids: vec![kael],
        when: Condition::Always,
    };
    let restriction = FutureRestriction {
        id: wobu_core::new_id(),
        name: "Hold the tide back".into(),
        fact: unseen.id,
        characters: vec![],
        until: Condition::Never,
    };
    let quest = Quest {
        id: wobu_core::new_id(),
        name: "Harbour watch".into(),
        summary: "Find out who lit the lamp.".into(),
        stages: vec![Name::new("open").unwrap()],
        initial: Name::new("open").unwrap(),
        transitions: vec![],
        scene_ids: vec![],
    };
    let world = WorldDocument {
        facts: vec![fact.clone(), unseen],
        knowledge: vec![claim],
        relationships: vec![relation],
        events: vec![event],
        quests: vec![quest],
        restrictions: vec![restriction],
        ..WorldDocument::default()
    };

    let schema = StateSchema::new([
        VariableDecl {
            name: Name::new("trusted").unwrap(),
            description: String::new(),
            ty: VarType::Bool,
            default: Value::Bool(false),
            owner: wobu_narrative::Owner::Narrative,
        },
        VariableDecl {
            name: Name::new("unused").unwrap(),
            description: String::new(),
            ty: VarType::Bool,
            default: Value::Bool(false),
            owner: wobu_narrative::Owner::Narrative,
        },
    ])
    .unwrap();

    let mut scene = Scene::new("Harbour watch");
    scene.summary = "Kael and Mira argue on the quay.".into();
    scene.participants = vec![
        Participant { entity: kael, role: String::new() },
        Participant { entity: mira, role: String::new() },
    ];
    let mut beat = Beat::new("On the quay");
    let mut kael_slot = DialogueSlot::new(Speaker::Entity(kael));
    let mut kael_variant = Variant::new(Text::written("The lamp was lit. I saw it."));
    kael_variant.when = Some(Condition::Always);
    kael_slot.variants.push(kael_variant);
    let mut mira_slot = DialogueSlot::new(Speaker::Entity(mira));
    mira_slot.variants.push(Variant::new(Text::written("You saw nothing.")));
    beat.dialogue.push(kael_slot);
    beat.dialogue.push(mira_slot);
    beat.must_convey = vec!["Kael is certain.".into()];
    scene.beats.push(beat);

    let mut asset =
        TextAsset::new(TextKind::Codex, "The harbour lamp", Name::new("read_codex").unwrap());
    asset.sources = vec![SourceLink::Fact(fact.id), SourceLink::Scene(scene.id)];
    let mut entry = TextEntry::new("Opening");
    let mut line = DialogueSlot::new(Speaker::Narrator);
    line.variants.push(Variant::new(Text::written("The lamp burns above the bar.")));
    entry.lines.push(line);
    asset.entries.push(entry);

    let characters = BTreeMap::from([
        (kael, Character { id: kael, name: KAEL.into(), voice: Some("Clipped, certain.".into()) }),
        (mira, Character { id: mira, name: MIRA.into(), voice: Some("Dry, patient.".into()) }),
    ]);

    World {
        kael,
        mira,
        scenes: vec![scene],
        texts: vec![asset],
        world,
        schema,
        characters,
        producers: BTreeMap::new(),
        versions: ToolVersions::current(),
    }
}

/// The variant ids an affected report names, so an assertion can be an exact
/// set rather than a count.
pub fn variants(affected: &[Affected]) -> BTreeSet<wobu_narrative::VariantId> {
    affected.iter().map(|item| item.target.variant()).collect()
}
