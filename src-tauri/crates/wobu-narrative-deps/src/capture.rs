//! Reading a dependency set off the canonical model.
//!
//! Every address here is spelled the way
//! [`wobu_narrative_context::resolve`] spells the same thing, and the hashes
//! are taken with the same [`content_hash`]. That is not tidiness: the frozen
//! context on a generation receipt and the dependency set in the index have to
//! be talking about the same world, and the day they disagree about whether a
//! character's voice is `character/<id>/narrative_voice` is the day an
//! invalidation stops reaching a receipt. `tests/resolver_vocabulary.rs` walks
//! a real resolve and asserts the two agree.
//!
//! ## What is deliberately not a dependency
//!
//! - **Another line's wording.** A scene's dependency sets never contain a
//!   sibling variant's body, revision or provenance, so rewriting one line
//!   cannot mark the rest of the scene stale. This is the whole point of the
//!   exercise: the review context that exists today hashes the entire scene
//!   document, so today a typo fix withdraws every approval in the file.
//! - **Editorial flags.** Policy, review state and freshness are outputs of
//!   this process, and a fingerprint that contained them would move every time
//!   it was acted on.
//! - **Display-only labels.** [`TextEntry::label`](wobu_narrative::TextEntry)
//!   documents itself as a name nothing is derived from, so renaming one
//!   invalidates nothing. A beat's `title` is *not* in that category: the
//!   context resolver puts it in the `objective` fragment, so it really is an
//!   input a model saw.
//! - **Canvas layout.** There is no field it could arrive through; see the
//!   crate documentation.
//! - **Act, arc and tag classification.** Nothing in the resolver reads them,
//!   so filing a scene under a different act does not change a word of it.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::json;
use wobu_narrative::{
    Condition, DialogueSlot, EntityId, Name, Scene, SourceLink, StateSchema, TextAsset, VariantId,
    WorldDocument,
};
use wobu_narrative_context::{Character, content_hash};

use crate::{DEPENDENCY_VERSION, DependencySet, Producer, QuerySet, TargetRef, ToolVersions};

/// One coherent read of everything a capture is allowed to see.
///
/// A borrowed struct rather than a set of arguments so that adding an input is
/// a change every caller has to acknowledge, and so that the list of things
/// this crate can reach is a single readable declaration. Notice what is not on
/// it: no path, no project, no index, no clock.
pub struct Snapshot<'a> {
    pub scenes: &'a [Scene],
    pub texts: &'a [TextAsset],
    pub world: &'a WorldDocument,
    pub schema: &'a StateSchema,
    /// Character narrative voices by entity id. An id missing from this map is
    /// recorded as an absent field rather than skipped, so that a participant
    /// who stops being a character is a change and not a silence.
    pub characters: &'a BTreeMap<EntityId, Character>,
    /// Provider identity for wording a model wrote, by variant.
    ///
    /// Sparse on purpose: hand-written wording has no producer, and must not
    /// acquire one when the project's default model changes.
    pub producers: &'a BTreeMap<VariantId, Producer>,
    pub versions: ToolVersions,
}

/// Every line in the snapshot, in a deterministic order.
///
/// Slots with no variants produce nothing. That is not an omission: an empty
/// slot has no wording, so there is no cached result to invalidate and no
/// freshness to derive. It becomes a target the moment it holds a variant.
pub fn capture(snapshot: &Snapshot<'_>) -> Vec<DependencySet> {
    let mut sets: Vec<DependencySet> = Vec::new();
    for scene in snapshot.scenes {
        sets.extend(capture_scene(scene, snapshot));
    }
    for asset in snapshot.texts {
        sets.extend(capture_text(asset, snapshot));
    }
    sets.sort_by(|a, b| a.target.cmp(&b.target));
    sets
}

/// The dependency sets of one scene's dialogue.
pub fn capture_scene(scene: &Scene, snapshot: &Snapshot<'_>) -> Vec<DependencySet> {
    let mut sets = Vec::new();
    for (beat_id, slot) in scene.dialogue_slots() {
        let Some(beat) = scene.beat(beat_id) else { continue };
        for (position, variant) in slot.variants.iter().enumerate() {
            let mut fields = Fields::default();
            let mut names = Names::default();

            fields.present(format!("scene/{}/name", scene.id), &scene.name);
            fields.present(format!("scene/{}/summary", scene.id), &scene.summary);
            fields.present(format!("scene/{}/entry", scene.id), &scene.entry);
            fields.present(format!("scene/{}/participants", scene.id), &scene.participants);
            names.of_optional(scene.entry.as_ref());

            let site = format!("scene/{}/beat/{}", scene.id, beat.id);
            fields.present(format!("{site}/title"), &beat.title);
            fields.present(format!("{site}/intents"), &beat.intents);
            fields.present(format!("{site}/must_convey"), &beat.must_convey);
            fields.present(format!("{site}/must_not_reveal"), &beat.must_not_reveal);

            let site = format!("{site}/slot/{}", slot.id);
            fields.present(format!("{site}/speaker"), &slot.speaker);
            wording(&mut fields, &mut names, &site, slot, position);

            let mut participants: BTreeSet<_> =
                scene.participants.iter().map(|p| p.entity).collect();
            let speaker = slot.speaker.entity();
            participants.extend(speaker);

            let queries = world(&mut fields, &mut names, snapshot, speaker, &participants);
            characters(&mut fields, snapshot, &participants);
            state(&mut fields, snapshot, &names);

            sets.push(finish(
                TargetRef::SceneLine {
                    scene: scene.id,
                    beat: beat.id,
                    slot: slot.id,
                    variant: variant.id,
                },
                snapshot,
                fields,
                queries,
            ));
        }
    }
    sets
}

/// The dependency sets of one supporting text asset's lines (#167).
///
/// The asset's [`sources`](wobu_narrative::TextAsset) are followed as typed
/// links, which is the reason they were made typed links rather than prose
/// mentioning a quest by name: a codex page that says it is about a fact goes
/// stale when that fact is rewritten or deleted, and nothing has to parse a
/// sentence to know it.
pub fn capture_text(asset: &TextAsset, snapshot: &Snapshot<'_>) -> Vec<DependencySet> {
    let mut sets = Vec::new();
    for entry in &asset.entries {
        for slot in &entry.lines {
            for (position, variant) in slot.variants.iter().enumerate() {
                let mut fields = Fields::default();
                let mut names = Names::default();

                let asset_site = format!("text/{}", asset.id);
                fields.present(format!("{asset_site}/name"), &asset.name);
                fields.present(format!("{asset_site}/summary"), &asset.summary);
                fields.present(format!("{asset_site}/kind"), &asset.kind);
                fields.present(format!("{asset_site}/trigger"), &asset.trigger);
                fields.present(format!("{asset_site}/must_convey"), &asset.must_convey);
                fields.present(format!("{asset_site}/must_not_reveal"), &asset.must_not_reveal);
                fields.present(format!("{asset_site}/participants"), &asset.participants);
                fields.present(format!("{asset_site}/sources"), &asset.sources);
                names.of_optional(asset.trigger.when.as_ref());

                let site = format!("{asset_site}/entry/{}", entry.id);
                fields.present(format!("{site}/when"), &entry.when);
                names.of_optional(entry.when.as_ref());

                let site = format!("{site}/slot/{}", slot.id);
                fields.present(format!("{site}/speaker"), &slot.speaker);
                wording(&mut fields, &mut names, &site, slot, position);

                let mut participants: BTreeSet<_> =
                    asset.participants.iter().map(|p| p.entity).collect();
                let speaker = slot.speaker.entity();
                participants.extend(speaker);

                let queries = world(&mut fields, &mut names, snapshot, speaker, &participants);
                for link in &asset.sources {
                    source_link(&mut fields, snapshot, *link, &mut participants);
                }
                characters(&mut fields, snapshot, &participants);
                state(&mut fields, snapshot, &names);

                sets.push(finish(
                    TargetRef::TextLine {
                        asset: asset.id,
                        entry: entry.id,
                        slot: slot.id,
                        variant: variant.id,
                    },
                    snapshot,
                    fields,
                    queries,
                ));
            }
        }
    }
    sets
}

/* ── the pieces ──────────────────────────────────────────────────────────── */

/// Source address → content hash, with `None` for a lookup that found nothing.
#[derive(Default)]
struct Fields(BTreeMap<String, Option<String>>);

impl Fields {
    fn present(&mut self, address: String, value: &impl Serialize) {
        self.0.insert(address, Some(content_hash(value)));
    }

    /// Record what a lookup found, *including* that it found nothing.
    ///
    /// The `None` branch is the half that matters. Skipping a missing record
    /// would make a dependency set that never mentions the fact it needed, and
    /// the fact arriving later would then invalidate nothing.
    fn lookup<T: Serialize>(&mut self, address: String, value: Option<&T>) {
        self.0.insert(address, value.map(content_hash));
    }
}

/// Declared variables a line's conditions read, gathered rather than assumed.
///
/// [`Condition::variables`] is the edge; this is the accumulator that keeps
/// every place a condition can appear — a scene's entry, a variant's `when`, a
/// world record's `when`, a restriction's `until` — feeding the same set, so
/// that changing a variable's declaration reaches every branch that reads it.
#[derive(Default)]
struct Names(BTreeSet<Name>);

impl Names {
    fn of(&mut self, condition: &Condition) {
        self.0.extend(condition.variables().into_iter().cloned());
    }

    fn of_optional(&mut self, condition: Option<&Condition>) {
        if let Some(condition) = condition {
            self.of(condition);
        }
    }
}

/// The parts of a dialogue slot that decide which wording applies.
///
/// `precedence` is the subtle one. Variants are first-match in author order, so
/// a line's applicability depends on every condition *above* it as much as on
/// its own: inserting an earlier variant that matches shadows this one without
/// touching it. Recording the earlier ids and conditions — and only those, so
/// that a variant below is irrelevant — makes that a tracked change.
fn wording(
    fields: &mut Fields,
    names: &mut Names,
    site: &str,
    slot: &DialogueSlot,
    position: usize,
) {
    let variant = &slot.variants[position];
    let site = format!("{site}/variant/{}", variant.id);
    fields.present(format!("{site}/when"), &variant.when);
    names.of_optional(variant.when.as_ref());
    let earlier: Vec<_> =
        slot.variants[..position].iter().map(|item| (item.id, &item.when)).collect();
    for (_, condition) in &earlier {
        names.of_optional(condition.as_ref());
    }
    fields.present(format!("{site}/precedence"), &earlier);
}

/// The world records this speaker could draw on, as candidate sets.
///
/// Each query name, its parameters and its filter mirror
/// [`wobu_narrative_context::resolve`]'s, with one deliberate difference:
/// nothing here evaluates a condition, so `relevant_events` is taken over every
/// fact any of the speaker's claims name rather than over the facts a chosen
/// scenario makes available. That is a superset of what any single scenario
/// would produce, which is the safe direction — the index may consider a line
/// affected that one preview would not have cared about, and can never miss one
/// that it would.
fn world(
    fields: &mut Fields,
    names: &mut Names,
    snapshot: &Snapshot<'_>,
    speaker: Option<EntityId>,
    participants: &BTreeSet<EntityId>,
) -> BTreeMap<String, QuerySet> {
    let world = snapshot.world;
    let mut queries = BTreeMap::new();

    let restrictions: Vec<_> = world
        .restrictions
        .iter()
        .filter(|r| r.characters.is_empty() || speaker.is_some_and(|id| r.characters.contains(&id)))
        .collect();
    queries.insert(
        "restrictions".to_string(),
        set(
            json!({"speaker": speaker, "all_characters": true}),
            restrictions.iter().map(|r| (r.id, *r)),
        ),
    );
    for restriction in &restrictions {
        names.of(&restriction.until);
        fact(fields, world, restriction.fact);
    }

    let claims: Vec<_> = world.knowledge.iter().filter(|k| Some(k.character) == speaker).collect();
    queries.insert(
        "speaker_knowledge".to_string(),
        set(json!({ "character": speaker }), claims.iter().map(|k| (k.id, *k))),
    );
    let mut reachable = BTreeSet::new();
    for claim in &claims {
        names.of(&claim.when);
        fact(fields, world, claim.fact);
        reachable.insert(claim.fact);
        if let wobu_narrative::KnowledgeProvenance::Told { by } = claim.provenance {
            fields.lookup(format!("character/{by}/identity"), snapshot.characters.get(&by));
        }
    }

    let relations: Vec<_> = world
        .relationships
        .iter()
        .filter(|r| Some(r.from) == speaker && participants.contains(&r.to))
        .collect();
    queries.insert(
        "directed_relationships".to_string(),
        set(json!({"from": speaker, "to": participants}), relations.iter().map(|r| (r.id, *r))),
    );
    for relation in &relations {
        names.of(&relation.when);
    }

    let events: Vec<_> = world
        .events
        .iter()
        .filter(|e| {
            speaker.is_some_and(|id| e.entity_ids.contains(&id))
                || e.fact_ids.iter().any(|id| reachable.contains(id))
        })
        .collect();
    queries.insert(
        "relevant_events".to_string(),
        set(
            json!({"speaker": speaker, "known_facts": reachable}),
            events.iter().map(|e| (e.id, *e)),
        ),
    );
    for event in &events {
        names.of(&event.when);
        for id in &event.fact_ids {
            fact(fields, world, *id);
        }
    }

    queries
}

fn set<'a, T: Serialize + 'a>(
    parameters: serde_json::Value,
    records: impl Iterator<Item = (EntityId, &'a T)>,
) -> QuerySet {
    QuerySet {
        parameters: parameters.to_string(),
        members: records.map(|(id, record)| (id.to_string(), content_hash(record))).collect(),
    }
}

/// A canonical fact, or the hole where one used to be.
fn fact(fields: &mut Fields, world: &WorldDocument, id: EntityId) {
    fields.lookup(format!("world/facts/{id}"), world.facts.iter().find(|f| f.id == id));
}

/// One typed link from a supporting text asset to the record it is about.
///
/// A linked character joins the participant set rather than being hashed on the
/// spot, so that the one address a character's voice lives at is the same
/// whether the character is in the cast or merely cited.
fn source_link(
    fields: &mut Fields,
    snapshot: &Snapshot<'_>,
    link: SourceLink,
    participants: &mut BTreeSet<EntityId>,
) {
    let world = snapshot.world;
    match link {
        SourceLink::Scene(id) => {
            let scene = snapshot.scenes.iter().find(|scene| scene.id == id);
            // Name and summary rather than the whole document: an asset that is
            // *about* a scene is not rewritten because a line inside that scene
            // was, and hashing the document would make it look as though it
            // were.
            fields.lookup(format!("scene/{id}/name"), scene.map(|scene| &scene.name));
            fields.lookup(format!("scene/{id}/summary"), scene.map(|scene| &scene.summary));
        }
        SourceLink::Quest(id) => {
            fields.lookup(format!("world/quests/{id}"), world.quests.iter().find(|q| q.id == id))
        }
        SourceLink::Fact(id) => fact(fields, world, id),
        SourceLink::Event(id) => {
            fields.lookup(format!("world/events/{id}"), world.events.iter().find(|e| e.id == id))
        }
        SourceLink::Character(id) => {
            participants.insert(id);
        }
    }
}

fn characters(fields: &mut Fields, snapshot: &Snapshot<'_>, ids: &BTreeSet<EntityId>) {
    for id in ids {
        fields.lookup(format!("character/{id}/narrative_voice"), snapshot.characters.get(id));
    }
}

/// The *declarations* of the variables a line's conditions read.
///
/// The declaration and not the value: a variable's current value is a scenario,
/// and a line is not rewritten because a preview moved a slider. Narrowing an
/// integer's range or removing an enum member, on the other hand, changes which
/// branches can ever be taken, and that is a change to the line.
fn state(fields: &mut Fields, snapshot: &Snapshot<'_>, names: &Names) {
    for name in &names.0 {
        fields.lookup(format!("state/{name}"), snapshot.schema.get(name));
    }
}

fn finish(
    target: TargetRef,
    snapshot: &Snapshot<'_>,
    fields: Fields,
    queries: BTreeMap<String, QuerySet>,
) -> DependencySet {
    DependencySet {
        version: DEPENDENCY_VERSION,
        producer: snapshot.producers.get(&target.variant()).cloned(),
        target,
        versions: snapshot.versions,
        fields: fields.0,
        queries,
    }
}
