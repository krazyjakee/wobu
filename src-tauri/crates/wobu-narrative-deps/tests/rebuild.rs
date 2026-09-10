//! The index is a cache: it can be thrown away and it must come back identical.
//!
//! This is the same rule the SQLite index lives by, and it is what makes the
//! whole scheme safe to ship. If a rebuilt index could differ from an
//! incrementally maintained one, then losing the cache would silently change
//! which lines a project considers current, and the only way to find out would
//! be to ship the wrong words.

mod support;

use std::collections::BTreeSet;

use support::{harbour, variants};
use wobu_narrative_deps::{DependencyIndex, capture, edge_keys};

#[test]
fn a_rebuilt_index_equals_the_one_it_replaced() {
    let world = harbour();
    let first = world.index();
    let second = DependencyIndex::rebuild(capture(&world.snapshot()));

    assert_eq!(first, second);
    assert_eq!(first.fingerprints(), second.fingerprints());
    assert_eq!(first.edges(), second.edges());
    assert!(first.diff(&second).is_empty());
}

#[test]
fn capture_is_deterministic_across_runs_of_the_same_snapshot() {
    // Ordered maps everywhere, so the bytes a fingerprint is taken over are a
    // function of the content and not of the walk order. Without this the index
    // would report the whole project affected on every reopen.
    let world = harbour();
    let a = serde_json::to_string(&capture(&world.snapshot())).unwrap();
    let b = serde_json::to_string(&capture(&world.snapshot())).unwrap();
    assert_eq!(a, b);
}

#[test]
fn losing_the_cache_reports_every_line_as_untracked_and_nothing_as_changed() {
    // What a caller sees after the index is deleted: a full rebuild, not a
    // project that has quietly become stale. Recording every line as untracked
    // rather than as changed is what keeps a lost cache from marking a hundred
    // approved lines out of date.
    let world = harbour();
    let lost = DependencyIndex::default();
    let affected = lost.diff(&world.index());

    assert_eq!(affected.len(), world.index().len());
    assert!(affected.iter().all(|item| item.kind == wobu_narrative_deps::AffectedKind::Untracked
        && item.before.is_none()
        && item.reasons.is_empty()));
}

#[test]
fn the_candidate_set_covers_the_affected_set_for_every_edit() {
    // The reverse index is allowed to be generous and is not allowed to be
    // wrong. A planner that filters on candidates and then settles with `diff`
    // has to be able to trust that the filter never dropped an affected line.
    let mut world = harbour();
    let kael = world.kael;

    let before = world.index();
    let affected = world.affected(|world| {
        world.characters.get_mut(&kael).unwrap().voice = Some("Hoarse.".into());
        world.world.relationships.clear();
    });

    let changed_keys: BTreeSet<String> = BTreeSet::from([
        format!("field:character/{kael}/narrative_voice"),
        "query:directed_relationships".into(),
    ]);
    let candidates = before.candidates(&changed_keys);
    assert!(!candidates.is_empty());
    assert!(
        variants(&affected).is_subset(&candidates),
        "the reverse index dropped an affected line: {affected:?} against {candidates:?}"
    );
}

#[test]
fn every_recorded_address_and_member_is_reachable_through_an_edge() {
    // The reverse index is derived from the sets, so an address that no edge
    // points at is an address a change at which would be invisible to the
    // planner.
    let world = harbour();
    let index = world.index();
    for set in index.sets() {
        for key in edge_keys(set) {
            assert!(
                index.edges().get(&key).is_some_and(|ids| ids.contains(&set.variant())),
                "no edge for {key}"
            );
        }
    }
}

#[test]
fn an_absent_fact_still_earns_an_edge_so_its_arrival_is_visible() {
    // The `None` half of a field. Deleting the fact leaves the address in the
    // set and therefore in the reverse index, which is exactly what lets the
    // fact coming back flag the same lines.
    let mut world = harbour();
    let fact = world.world.facts[0].id;
    world.world.facts.retain(|f| f.id != fact);

    let index = world.index();
    let key = format!("field:world/facts/{fact}");
    assert!(index.edges().contains_key(&key), "a deleted fact left no edge behind");
    assert!(!index.candidates(&BTreeSet::from([key])).is_empty());
}
