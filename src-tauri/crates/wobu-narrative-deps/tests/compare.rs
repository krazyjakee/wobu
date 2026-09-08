//! `compare` and `fingerprint` have to agree, in both directions.
//!
//! A reason with no fingerprint movement would be a change nothing keys on; a
//! fingerprint movement with no reason would be a line marked out of date that
//! nobody can explain, which is the failure mode that teaches people to ignore
//! the badge.

mod support;

use support::harbour;
use wobu_narrative_deps::{Producer, capture, compare};

#[test]
fn an_unchanged_set_has_no_reasons_and_the_same_fingerprint() {
    let world = harbour();
    for set in capture(&world.snapshot()) {
        assert!(compare(&set, &set).is_empty());
        assert_eq!(set.fingerprint(), set.clone().fingerprint());
    }
}

#[test]
fn every_structural_difference_produces_both_a_reason_and_a_new_fingerprint() {
    let world = harbour();
    let base = capture(&world.snapshot());
    let original = base[0].clone();

    let mut mutations = Vec::new();
    let mut edited = original.clone();
    let address = edited.fields.keys().next().unwrap().clone();
    edited.fields.insert(address.clone(), Some("0".repeat(64)));
    mutations.push(("a field value", edited));

    let mut edited = original.clone();
    edited.fields.insert(address.clone(), None);
    mutations.push(("a field emptied", edited));

    let mut edited = original.clone();
    edited.fields.remove(&address);
    mutations.push(("a field dropped", edited));

    let mut edited = original.clone();
    edited.fields.insert("world/facts/absent".into(), None);
    mutations.push(("a newly watched empty address", edited));

    let mut edited = original.clone();
    edited.queries.get_mut("relevant_events").unwrap().members.insert("x".into(), "y".into());
    mutations.push(("a query member", edited));

    let mut edited = original.clone();
    edited.queries.get_mut("relevant_events").unwrap().parameters = "{}".into();
    mutations.push(("query parameters", edited));

    let mut edited = original.clone();
    edited.queries.remove("restrictions");
    mutations.push(("a whole query", edited));

    let mut edited = original.clone();
    edited.versions.compiler_graph += 1;
    mutations.push(("a toolchain version", edited));

    let mut edited = original.clone();
    edited.producer = Some(Producer::of("openai", "gpt-5", &serde_json::json!({"a": 1})));
    mutations.push(("the producer", edited));

    for (what, edited) in mutations {
        assert_ne!(edited, original, "{what} did not actually change the set");
        assert!(!compare(&original, &edited).is_empty(), "{what} produced no reason");
        assert_ne!(
            original.fingerprint(),
            edited.fingerprint(),
            "{what} produced a reason but did not move the fingerprint"
        );
    }
}

#[test]
fn producer_settings_are_normalised_rather_than_taken_in_written_order() {
    // Two spellings of the same settings object. A fingerprint that disagreed
    // would invalidate every generated line the first time a serializer changed
    // its key order.
    let a = Producer::of("anthropic", "m", &serde_json::json!({"b": 2, "a": 1}));
    let b = Producer::of("anthropic", "m", &serde_json::json!({"a": 1, "b": 2}));
    assert_eq!(a, b);

    let scalar = Producer::of("anthropic", "m", &serde_json::json!(7));
    assert_eq!(scalar.settings.keys().collect::<Vec<_>>(), vec!["value"]);
}

#[test]
fn a_reason_names_a_source_a_context_and_a_line() {
    let world = harbour();
    let base = capture(&world.snapshot());
    let mut edited = base[0].clone();
    let address = edited.fields.keys().next().unwrap().clone();
    edited.fields.insert(address.clone(), Some("0".repeat(64)));

    let affected = wobu_narrative_deps::Affected {
        target: edited.target.clone(),
        kind: wobu_narrative_deps::AffectedKind::Changed,
        before: Some(base[0].fingerprint()),
        after: Some(edited.fingerprint()),
        reasons: compare(&base[0], &edited),
    };
    let rows = affected.explanations();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source, address);
    assert_eq!(rows[0].line, edited.target.line());
    assert!(!rows[0].context.is_empty());
}
