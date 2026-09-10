//! The version boundary, held identically at every document type this crate
//! reads and writes.
//!
//! Four numbers, deliberately independent: a scene, a world, a state
//! declaration and a supporting text asset each version themselves, so adding a
//! field to one does not make every file of the other three look like it needs
//! migrating. That independence is only worth anything if it is enforced, and
//! the failure it prevents is quiet: a state document stamped `2` read as though
//! `2` meant what it means for a scene would be a file this build does not
//! understand, accepted.
//!
//! Every case below is the same three-part rule. A version this build cannot
//! read is refused *as a version*, before the shape is looked at, so the remedy
//! offered is "open it with a newer Wobu" rather than "delete the fields we do
//! not recognise". A version this build cannot produce is refused at the write
//! boundary too, because a reader that rejects what a writer emits is a project
//! that saves once and never opens again. And a file with no version at all is
//! its own error rather than a guess, because every guess available is wrong for
//! some file somebody already has.

use wobu_narrative::{
    Error, SCENE_SCHEMA_VERSION, SOURCE_SCHEMA_VERSION, Scene, SceneDocument, StateDocument,
    TEXT_SCHEMA_VERSION, TextAsset, TextAssetDocument, TextKind, WORLD_SCHEMA_VERSION,
    WorldDocument,
};

/// One document type: what it is called in a failure message, the highest
/// version it reads, how to write one down at any stated version, and how to
/// read one back.
///
/// A table rather than four near-identical tests, because the point being made
/// is that the rule is the same for all four and the *ceiling* is the only thing
/// that differs. Four hand-written copies would let one of them quietly stop
/// agreeing with the others.
struct Document {
    noun: &'static str,
    ceiling: u32,
    at: fn(u32) -> String,
    parse: fn(&str) -> Result<(), Error>,
}

const DOCUMENTS: [Document; 4] = [
    Document { noun: "scene", ceiling: SCENE_SCHEMA_VERSION, at: scene_at, parse: parse_scene },
    Document { noun: "world", ceiling: WORLD_SCHEMA_VERSION, at: world_at, parse: parse_world },
    Document { noun: "state", ceiling: SOURCE_SCHEMA_VERSION, at: state_at, parse: parse_state },
    Document { noun: "text", ceiling: TEXT_SCHEMA_VERSION, at: text_at, parse: parse_text },
];

/// Restate a document's version without touching anything else about it.
fn stamp(yaml: String, from: u32, to: u32) -> String {
    let old = format!("schema_version: {from}");
    assert!(yaml.contains(&old), "the fixture did not stamp {from}: {yaml}");
    yaml.replace(&old, &format!("schema_version: {to}"))
}

fn scene_at(version: u32) -> String {
    let yaml = SceneDocument::new(Scene::new("Council hearing")).to_yaml().unwrap();
    stamp(yaml, SCENE_SCHEMA_VERSION, version)
}

fn world_at(version: u32) -> String {
    let yaml = WorldDocument::default().to_yaml().unwrap();
    stamp(yaml, SOURCE_SCHEMA_VERSION, version)
}

fn state_at(version: u32) -> String {
    let yaml = StateDocument::new(Vec::new()).to_yaml().unwrap();
    stamp(yaml, SOURCE_SCHEMA_VERSION, version)
}

fn text_at(version: u32) -> String {
    let event = "player_passes_gate".parse().unwrap();
    let asset = TextAsset::new(TextKind::Bark, "Gate guard", event);
    let yaml = TextAssetDocument::new(asset).to_yaml().unwrap();
    stamp(yaml, TEXT_SCHEMA_VERSION, version)
}

fn parse_scene(yaml: &str) -> Result<(), Error> {
    SceneDocument::parse(yaml).map(drop)
}

fn parse_world(yaml: &str) -> Result<(), Error> {
    WorldDocument::parse(yaml).map(drop)
}

fn parse_state(yaml: &str) -> Result<(), Error> {
    StateDocument::parse(yaml).map(drop)
}

fn parse_text(yaml: &str) -> Result<(), Error> {
    TextAssetDocument::parse(yaml).map(drop)
}

#[test]
fn each_document_type_answers_for_its_own_ceiling_and_not_a_shared_one() {
    // Version 2 is an ordinary scene and an ordinary world, and is a file from
    // the future for a state declaration and a supporting text asset. Both
    // answers are correct and one shared constant could not give both.
    for document in &DOCUMENTS {
        for version in 1..=document.ceiling {
            (document.parse)(&(document.at)(version))
                .unwrap_or_else(|err| panic!("{} at version {version}: {err}", document.noun));
        }
        let above = document.ceiling + 1;
        assert_eq!(
            (document.parse)(&(document.at)(above)).unwrap_err(),
            Error::UnsupportedSchemaVersion { found: above, supported: document.ceiling },
            "{}",
            document.noun
        );
    }
}

#[test]
fn a_future_version_is_refused_before_its_shape_is_read() {
    // A file from a newer build is nearly always a newer version *and* a pile of
    // keys this build has never heard of. Reporting the keys first invites
    // exactly the wrong repair — hand-deleting them — which is how a newer
    // project becomes an older one with data missing.
    for document in &DOCUMENTS {
        let yaml = format!("{}epilogue: true\n", (document.at)(99));
        let err = (document.parse)(&yaml).unwrap_err();
        assert_eq!(
            err,
            Error::UnsupportedSchemaVersion { found: 99, supported: document.ceiling },
            "{}",
            document.noun
        );
        // And the remedy has to be in the message, not only in the type.
        assert!(err.to_string().contains("newer Wobu"), "{}: {err}", document.noun);
    }
}

#[test]
fn version_zero_is_refused_rather_than_read_as_the_oldest_supported() {
    // Nothing was ever written at version 0, so a file claiming it is either
    // corrupt or produced by something that is not Wobu. Rounding it up to the
    // oldest readable version would interpret whichever of those it is.
    for document in &DOCUMENTS {
        assert_eq!(
            (document.parse)(&(document.at)(0)).unwrap_err(),
            Error::UnsupportedSchemaVersion { found: 0, supported: document.ceiling },
            "{}",
            document.noun
        );
    }
}

#[test]
fn a_document_with_no_version_at_all_says_so() {
    for document in &DOCUMENTS {
        let yaml = (document.at)(1).replace("schema_version: 1\n", "");
        assert_eq!(
            (document.parse)(&yaml).unwrap_err(),
            Error::MissingSchemaVersion,
            "{}",
            document.noun
        );
    }
}

#[test]
fn a_version_this_build_cannot_produce_is_refused_at_the_write_boundary_too() {
    // The read check alone would let a caller that built a document in memory
    // write a version this build cannot read back, which is a project that saves
    // successfully once and fails to open afterwards.
    let scene = SceneDocument { schema_version: 3, scene: Scene::new("Council hearing") };
    assert_eq!(
        scene.to_yaml().unwrap_err(),
        Error::UnsupportedSchemaVersion { found: 3, supported: SCENE_SCHEMA_VERSION }
    );

    let world = WorldDocument { schema_version: 3, ..WorldDocument::default() };
    assert_eq!(
        world.to_yaml().unwrap_err(),
        Error::UnsupportedSchemaVersion { found: 3, supported: WORLD_SCHEMA_VERSION }
    );

    let state = StateDocument { schema_version: 2, variables: Vec::new() };
    assert_eq!(
        state.to_yaml().unwrap_err(),
        Error::UnsupportedSchemaVersion { found: 2, supported: SOURCE_SCHEMA_VERSION }
    );

    let event = "player_passes_gate".parse().unwrap();
    let text = TextAssetDocument {
        schema_version: 2,
        asset: TextAsset::new(TextKind::Bark, "Gate guard", event),
    };
    assert_eq!(
        text.to_yaml().unwrap_err(),
        Error::UnsupportedSchemaVersion { found: 2, supported: TEXT_SCHEMA_VERSION }
    );
}
