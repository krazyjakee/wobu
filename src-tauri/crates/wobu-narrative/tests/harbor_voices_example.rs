//! The checked-in supporting text example, held to the rules it demonstrates.
//!
//! `examples/narrative/harbor-voices/` contains one original asset of each of
//! the six kinds (#167). It is documentation, and documentation that stops
//! parsing is worse than none — a reader who copies it into a project and gets
//! an error learns the wrong thing about the format. Embedding the real files
//! with `include_str!` means the example cannot drift from the model without
//! this test failing at compile time or at the first assertion.

use wobu_narrative::*;

const STATE: &str = include_str!("../../../../examples/narrative/harbor-voices/state.yaml");

/// The six documents, in the order the Text library offers their templates.
const ASSETS: [(TextKind, &str); 6] = [
    (
        TextKind::Bark,
        include_str!("../../../../examples/narrative/harbor-voices/texts/seawall-keeper-bark.yaml"),
    ),
    (
        TextKind::Ambient,
        include_str!("../../../../examples/narrative/harbor-voices/texts/quay-talk-ambient.yaml"),
    ),
    (
        TextKind::Reaction,
        include_str!(
            "../../../../examples/narrative/harbor-voices/texts/mara-logbook-reaction.yaml"
        ),
    ),
    (
        TextKind::Codex,
        include_str!("../../../../examples/narrative/harbor-voices/texts/harbour-watch-codex.yaml"),
    ),
    (
        TextKind::QuestSummary,
        include_str!(
            "../../../../examples/narrative/harbor-voices/texts/missing-light-quest-summary.yaml"
        ),
    ),
    (
        TextKind::Journal,
        include_str!(
            "../../../../examples/narrative/harbor-voices/texts/watch-nights-journal.yaml"
        ),
    ),
];

fn schema() -> StateSchema {
    StateDocument::parse(STATE).expect("the example state parses").schema().expect("it type-checks")
}

fn parsed() -> Vec<TextAsset> {
    ASSETS
        .iter()
        .map(|(_, yaml)| TextAssetDocument::parse(yaml).expect("the example parses").asset)
        .collect()
}

#[test]
fn the_example_covers_every_kind_exactly_once() {
    let kinds: Vec<TextKind> = parsed().iter().map(|asset| asset.kind).collect();
    assert_eq!(kinds, ASSETS.iter().map(|(kind, _)| *kind).collect::<Vec<_>>());
    assert_eq!(kinds.len(), TextKind::ALL.len());
}

#[test]
fn every_example_asset_is_free_of_diagnostics() {
    let schema = schema();
    for asset in parsed() {
        let problems = asset.diagnostics(&schema);
        assert!(problems.is_empty(), "{}: {problems:?}", asset.name);
    }
}

#[test]
fn every_example_wording_still_hashes_to_its_recorded_revision() {
    // The check a locale pack and a recording script both depend on. A hand
    // edit to one of these files that forgets the revision is exactly the
    // silent breakage this asserts against.
    for asset in parsed() {
        for (_, slot) in asset.lines() {
            for variant in &slot.variants {
                assert!(
                    variant.text.revision_matches(),
                    "{}: {} no longer matches its revision",
                    asset.name,
                    variant.text.body
                );
            }
        }
    }
}

#[test]
fn the_example_round_trips_byte_for_byte() {
    // Reading and rewriting a checked-in document must produce the same bytes,
    // or opening the example in Wobu and saving it would show a diff nobody
    // made.
    for (_, yaml) in ASSETS {
        let document = TextAssetDocument::parse(yaml).expect("the example parses");
        assert_eq!(document.to_yaml().expect("it serializes"), yaml);
    }
}

#[test]
fn the_ambient_example_is_an_exchange_between_two_named_speakers() {
    let ambient = parsed().into_iter().find(|a| a.kind == TextKind::Ambient).expect("one ambient");
    for entry in &ambient.entries {
        assert!(entry.lines.len() > 1, "an exchange is more than one line");
        let speakers: Vec<_> = entry.lines.iter().map(|slot| &slot.speaker).collect();
        assert_ne!(speakers[0], speakers[1], "consecutive lines are different voices");
    }
    assert_eq!(ambient.participants.len(), 2);
}

#[test]
fn the_prose_examples_have_no_cast_and_no_character_voice() {
    for asset in parsed().into_iter().filter(|a| a.kind.voice() == Voice::Standalone) {
        assert!(asset.participants.is_empty(), "{}", asset.name);
        for (_, slot) in asset.lines() {
            assert!(slot.speaker.entity().is_none(), "{}", asset.name);
        }
    }
}

#[test]
fn the_bark_example_has_enough_entries_for_its_shuffle_to_mean_anything() {
    let bark = parsed().into_iter().find(|a| a.kind == TextKind::Bark).expect("one bark");
    assert_eq!(bark.repeat, RepeatPolicy::Shuffle);
    assert!(bark.entries.len() >= 4, "a two-line bark rotation is not a demonstration");
}

#[test]
fn every_example_condition_type_checks_against_the_example_state() {
    // Belt and braces beside `diagnostics`: this is the assertion that would
    // still fail if somebody weakened the diagnostic and not the model.
    let schema = schema();
    for asset in parsed() {
        let conditions = asset
            .trigger
            .when
            .iter()
            .chain(asset.entries.iter().filter_map(|entry| entry.when.as_ref()))
            .chain(
                asset
                    .lines()
                    .flat_map(|(_, slot)| &slot.variants)
                    .filter_map(|variant| variant.when.as_ref()),
            );
        for condition in conditions {
            schema.check_condition(condition).expect("the example type-checks");
        }
    }
}
