//! The dependency index and the frozen context have to name the same world.
//!
//! Two crates answer overlapping questions about what a line reads, and they
//! answer them for different callers: the resolver freezes one scenario onto a
//! receipt, the index tracks every scenario at once. They are allowed to differ
//! in *scope*. They are not allowed to differ in *vocabulary* — the day one
//! calls a character's voice `character/<id>/narrative_voice` and the other
//! calls it `characters/<id>/voice`, an invalidation stops reaching the receipt
//! it was supposed to withdraw, and nothing fails until a project ships words
//! nobody approved.
//!
//! So this walks a real `resolve` over the same fixture and checks the two
//! agree, address for address and member for member.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use support::harbour;
use wobu_narrative_context::{Input, Options, Selection, resolve};
use wobu_narrative_deps::capture;

fn frozen(world: &support::World) -> wobu_narrative_context::FrozenContext {
    let scene = &world.scenes[0];
    let beat = &scene.beats[0];
    let slot = &beat.dialogue[0];
    resolve(
        Input { scene, world: &world.world, schema: &world.schema, characters: &world.characters },
        Options {
            selection: Selection {
                scene: scene.id,
                beat: beat.id,
                slot: slot.id,
                variant: Some(slot.variants[0].id),
            },
            state: world
                .schema
                .iter()
                .map(|decl| (decl.name.clone(), decl.default.clone()))
                .collect(),
            token_budget: 8_000,
        },
    )
}

#[test]
fn the_resolver_and_the_capture_run_the_same_named_queries() {
    let world = harbour();
    let context = frozen(&world);
    let set = capture(&world.snapshot())
        .into_iter()
        .find(|set| set.variant() == world.kael_line())
        .expect("the captured line exists");

    let resolved: BTreeSet<_> = context.queries.iter().map(|query| query.name.clone()).collect();
    let captured: BTreeSet<_> = set.queries.keys().cloned().collect();
    assert_eq!(resolved, captured);
}

#[test]
fn every_candidate_the_resolver_considered_is_a_candidate_the_index_tracks() {
    // Equality rather than containment for the three scenario-independent
    // queries, and containment for `relevant_events`: the index takes the union
    // over scenarios there, which is a superset by construction. A subset would
    // be the bug — it would mean an edit the resolver saw that the index does
    // not watch.
    let world = harbour();
    let context = frozen(&world);
    let set = capture(&world.snapshot())
        .into_iter()
        .find(|set| set.variant() == world.kael_line())
        .expect("the captured line exists");

    for query in &context.queries {
        let captured = &set.queries[&query.name];
        let resolved: BTreeMap<_, _> = query.members.clone().into_iter().collect();
        assert!(
            resolved.keys().all(|id| captured.members.contains_key(id)),
            "{} lost a candidate the resolver considered",
            query.name
        );
        if query.name != "relevant_events" {
            assert_eq!(
                resolved.keys().collect::<Vec<_>>(),
                captured.members.keys().collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn every_source_the_resolver_hashed_is_watched_by_the_index() {
    // The resolver addresses a world record either as a dependency of its own
    // or as a member of a query. The index has to be watching it one way or the
    // other, and `state/schema` is the one deliberate exception: the index
    // watches the individual declarations a line's conditions name, which is
    // strictly finer than the whole schema.
    let world = harbour();
    let context = frozen(&world);
    let set = capture(&world.snapshot())
        .into_iter()
        .find(|set| set.variant() == world.kael_line())
        .expect("the captured line exists");

    let members: BTreeSet<_> =
        set.queries.values().flat_map(|query| query.members.keys().cloned()).collect();

    for address in context.dependencies.keys() {
        if address == "state/schema" || address.starts_with("scene/") {
            continue;
        }
        let watched = set.fields.contains_key(address)
            || address.rsplit('/').next().is_some_and(|id| members.contains(id));
        assert!(watched, "the index does not watch {address}");
    }
}

#[test]
fn the_two_hash_the_same_record_to_the_same_value() {
    // Shared `content_hash`, so a caller holding a receipt can compare a stored
    // dependency hash against a freshly captured one without translating
    // between two hashing schemes.
    let world = harbour();
    let context = frozen(&world);
    let set = capture(&world.snapshot())
        .into_iter()
        .find(|set| set.variant() == world.kael_line())
        .expect("the captured line exists");

    let fact = format!("world/facts/{}", world.world.facts[0].id);
    assert_eq!(
        context.dependencies.get(&fact).cloned(),
        set.fields.get(&fact).cloned().flatten(),
        "the two crates disagree about the hash of {fact}"
    );

    for query in &context.queries {
        for (id, hash) in &query.members {
            assert_eq!(set.queries[&query.name].members.get(id), Some(hash));
        }
    }
}
