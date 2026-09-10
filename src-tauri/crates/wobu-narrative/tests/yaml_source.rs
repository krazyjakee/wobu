//! The YAML source form: it round-trips exactly, and it refuses rather than
//! quietly losing what it does not understand.

mod support;

use support::{ashfall_state, council_hearing};
use wobu_narrative::{Error, Provenance, SCENE_SCHEMA_VERSION, SceneDocument, StateDocument, Text};

#[test]
fn a_scene_survives_a_round_trip_unchanged() {
    let council = council_hearing();
    let document = SceneDocument::new(council.scene.clone());

    let yaml = document.to_yaml().unwrap();
    let back = SceneDocument::parse(&yaml).unwrap();

    assert_eq!(back, document);
    // And again, so the second serialisation is byte-identical to the first —
    // otherwise every open-and-save would show as a diff to a collaborator.
    assert_eq!(back.to_yaml().unwrap(), yaml);
}

#[test]
fn every_id_survives_a_round_trip_exactly() {
    let council = council_hearing();
    let yaml = SceneDocument::new(council.scene.clone()).to_yaml().unwrap();
    let back = SceneDocument::parse(&yaml).unwrap().scene;

    assert_eq!(back.id, council.scene.id);
    for (before, after) in council.scene.beats.iter().zip(&back.beats) {
        assert_eq!(before.id, after.id);
        for (before, after) in before.dialogue.iter().zip(&after.dialogue) {
            assert_eq!(before.id, after.id);
            for (before, after) in before.variants.iter().zip(&after.variants) {
                assert_eq!(before.id, after.id);
                assert_eq!(before.text.revision, after.text.revision);
            }
        }
        for (before, after) in before.choices.iter().zip(&after.choices) {
            assert_eq!(before.id, after.id);
            assert_eq!(before.to, after.to);
        }
        for (before, after) in before.outcomes.iter().zip(&after.outcomes) {
            assert_eq!(before.id, after.id);
            assert_eq!(before.to, after.to);
        }
    }
}

#[test]
fn unicode_multiline_and_whitespace_survive_byte_for_byte() {
    // Every shape that has cost somebody a round trip: curly quotes, an em
    // dash, non-Latin scripts, a blank line, a leading tab, a trailing newline,
    // and a line that looks like YAML syntax.
    let awkward = "「灯台が燃えた」— she said, ash and all.\n\n\
                   \tI have the logbook: “Ashfall, third bell”.\n\
                   - not a list item\n\
                   key: not a mapping\n  \n";

    let mut council = council_hearing();
    council.scene.beats[0].dialogue[0].variants[0].text = Text::written(awkward);
    let revision = council.scene.beats[0].dialogue[0].variants[0].text.revision.clone();

    let yaml = SceneDocument::new(council.scene).to_yaml().unwrap();
    let back = SceneDocument::parse(&yaml).unwrap().scene;
    let text = &back.beats[0].dialogue[0].variants[0].text;

    assert_eq!(text.body, awkward);
    // The revision is derived from the body, so an exact round trip of the body
    // and a matching revision are the same claim checked two ways.
    assert_eq!(text.revision, revision);
    assert!(text.revision_matches());
}

#[test]
fn provenance_and_the_three_lifecycle_dimensions_survive() {
    let mut council = council_hearing();
    let text = &mut council.scene.beats[1].dialogue[0].variants[0].text;
    text.set_body("A drafted line.", Provenance::Generated { fingerprint: "job-7".into() });
    text.lifecycle.review = wobu_narrative::ReviewState::Approved;
    text.lifecycle.mark_out_of_date();
    let expected = text.clone();

    let yaml = SceneDocument::new(council.scene).to_yaml().unwrap();
    let back = SceneDocument::parse(&yaml).unwrap().scene;

    assert_eq!(back.beats[1].dialogue[0].variants[0].text, expected);
    assert!(!back.beats[1].dialogue[0].variants[0].text.lifecycle.is_release_ready());
}

#[test]
fn tombstones_survive_so_a_reload_can_still_explain_a_break() {
    let mut council = council_hearing();
    let verdict = council.scene.beats[2].id;
    council.scene.remove_beat(verdict, Some("folded into the epilogue".into()));

    let yaml = SceneDocument::new(council.scene).to_yaml().unwrap();
    let back = SceneDocument::parse(&yaml).unwrap().scene;

    let found = back.destination_issues(&wobu_narrative::SceneCatalog::unknown());
    assert!(found.iter().any(|d| d.to_string().contains("folded into the epilogue")), "{found:?}");
}

#[test]
fn an_unknown_field_is_refused_with_a_location() {
    let council = council_hearing();
    let yaml = SceneDocument::new(council.scene).to_yaml().unwrap();
    // A layout coordinate is the exact field this model must never acquire, and
    // the exact thing a well-meaning tool would try to write into it.
    let tampered = yaml.replace("  name: Council hearing\n", "  name: Council hearing\n  x: 240\n");

    let err = SceneDocument::parse(&tampered).unwrap_err();
    let Error::Source { location, message } = &err else { panic!("{err:?}") };
    assert!(message.contains("x"), "{message}");
    let location = location.expect("an unknown field has a position in the file");
    let expected = tampered.lines().position(|l| l.trim() == "x: 240").unwrap() + 1;
    assert_eq!(location.line, expected, "{message}");
    assert!(message.contains(&format!("line {expected}")), "{message}");
}

#[test]
fn a_mistyped_key_is_refused_rather_than_dropped() {
    // The failure this rule exists to prevent: serde's default would skip
    // `must_conveys`, the next save would write the file back without it, and
    // the author would never be told their brief had gone.
    let council = council_hearing();
    let yaml = SceneDocument::new(council.scene)
        .to_yaml()
        .unwrap()
        .replace("    must_convey:\n", "    must_conveys:\n");
    assert!(matches!(SceneDocument::parse(&yaml), Err(Error::Source { .. })));
}

#[test]
fn a_newer_schema_version_is_refused_as_a_version_and_not_as_a_shape() {
    let council = council_hearing();
    let yaml = SceneDocument::new(council.scene)
        .to_yaml()
        .unwrap()
        .replace("schema_version: 2", "schema_version: 3")
        .replace("  name: Council hearing\n", "  name: Council hearing\n  epilogue: true\n");

    let err = SceneDocument::parse(&yaml).unwrap_err();
    assert_eq!(err, Error::UnsupportedSchemaVersion { found: 3, supported: SCENE_SCHEMA_VERSION });
    // The remedy has to be in the message, or the obvious response is to delete
    // the fields this build does not recognise.
    assert!(err.to_string().contains("newer Wobu"), "{err}");
}

#[test]
fn an_older_schema_version_is_refused_rather_than_guessed_at() {
    let council = council_hearing();
    let yaml = SceneDocument::new(council.scene)
        .to_yaml()
        .unwrap()
        .replace("schema_version: 2", "schema_version: 0");
    assert!(matches!(
        SceneDocument::parse(&yaml),
        Err(Error::UnsupportedSchemaVersion { found: 0, .. })
    ));
}

#[test]
fn a_file_with_no_version_is_refused() {
    let council = council_hearing();
    let yaml =
        SceneDocument::new(council.scene).to_yaml().unwrap().replace("schema_version: 2\n", "");
    assert_eq!(SceneDocument::parse(&yaml).unwrap_err(), Error::MissingSchemaVersion);
}

#[test]
fn malformed_yaml_reports_where() {
    let err = SceneDocument::parse("schema_version: 1\nscene:\n  id: [\n").unwrap_err();
    let Error::Source { location, .. } = err else { panic!("{err:?}") };
    assert!(location.is_some(), "a syntax error must carry a position");
}

#[test]
fn a_malformed_id_is_refused_by_name() {
    let council = council_hearing();
    let id = council.scene.id.to_string();
    let yaml = SceneDocument::new(council.scene).to_yaml().unwrap().replace(&id, "not-a-ulid");
    let err = SceneDocument::parse(&yaml).unwrap_err();
    assert!(matches!(err, Error::Source { .. }), "{err:?}");
}

#[test]
fn the_state_document_round_trips_and_rebuilds_its_schema() {
    let schema = ashfall_state();
    let document = StateDocument::new(schema.iter().cloned().collect());

    let yaml = document.to_yaml().unwrap();
    let back = StateDocument::parse(&yaml).unwrap();

    assert_eq!(back, document);
    assert_eq!(back.schema().unwrap(), schema);
}

#[test]
fn a_variable_name_yaml_would_re_read_as_a_boolean_is_refused() {
    // `off` written bare comes back as `false`, so the comparison an author
    // wrote against it would silently stop matching.
    let yaml = "schema_version: 1\nvariables:\n- name: off\n  type: bool\n  default: false\n";
    assert!(matches!(StateDocument::parse(yaml), Err(Error::Source { .. })));
}

#[test]
fn a_declared_state_file_reads_the_way_a_person_would_write_it() {
    // Not a formatting assertion for its own sake: the Source view is an
    // authoring surface (US-14), so the shape a writer has to type by hand is
    // part of the contract.
    let yaml = "\
schema_version: 1
variables:
- name: beacon_quest
  type: !enum
    members: [investigating, resolved]
  default: investigating
- name: trust
  type: !int
    min: -100
    max: 100
  default: 0
  owner: narrative
- name: difficulty
  type: bool
  default: false
  owner: host
  description: Set by the game before the scene starts.
";
    let schema = StateDocument::parse(yaml).unwrap().schema().unwrap();
    assert_eq!(schema.len(), 3);
    // Omitted `owner` defaults to the narrative owning its own state.
    assert_eq!(schema.iter().next().unwrap().owner, wobu_narrative::Owner::Narrative);
}

#[test]
fn a_scene_reads_the_way_a_person_would_write_it() {
    let yaml = "\
schema_version: 1
scene:
  id: 01J8Z00000000000000000SCEN
  name: A quiet room
  beats:
  - id: 01J8Z00000000000000000BEAT
    title: Wait
    choices:
    - id: 01J8Z0000000000000000CH0CE
      label: Leave
      requires: !compare
        var: has_logbook
        op: eq
        value: !literal true
      effects:
      - !add
        var: trust
        by: 5
      - !command
        name: start_combat
      to: !end
        label: Gone
";
    let scene = SceneDocument::parse(yaml).unwrap().scene;
    assert_eq!(scene.name, "A quiet room");
    assert_eq!(scene.beats[0].choices[0].effects.len(), 2);
    assert!(scene.destination_issues(&wobu_narrative::SceneCatalog::unknown()).is_empty());
}

#[test]
fn nested_conditions_write_canonical_maps_and_read_mixed_legacy_tags() {
    use wobu_narrative::{CompareOp, Comparison, Condition, Name, Operand, Value};
    let mut scene = council_hearing().scene;
    scene.entry = Some(Condition::Not(Box::new(Condition::Compare(Comparison {
        var: Name::new("has_logbook").unwrap(),
        op: CompareOp::Eq,
        value: Operand::Literal(Value::Bool(true)),
    }))));
    let document = SceneDocument::new(scene);
    let yaml = document.to_yaml().unwrap();
    assert!(yaml.contains("not:\n"));
    assert!(!yaml.contains("!compare"));
    assert_eq!(SceneDocument::parse(&yaml).unwrap(), document);
    let mixed = yaml.replace("not:\n      compare:", "not: !compare");
    assert_ne!(mixed, yaml);
    assert_eq!(SceneDocument::parse(&mixed).unwrap(), document);
    assert_eq!(SceneDocument::parse(&mixed).unwrap().to_yaml().unwrap(), yaml);
    assert!(SceneDocument::parse(&mixed.replace("var: has_logbook", "typo: has_logbook")).is_err());
}

#[test]
fn state_enum_and_integer_declarations_normalize_legacy_tags_without_changing_values() {
    let legacy = "schema_version: 1\nvariables:\n- name: trust\n  type: !int {min: -100, max: 100}\n  default: -17\n- name: quest\n  type: !enum {members: [started, finished]}\n  default: started\n";
    let document = StateDocument::parse(legacy).unwrap();
    let yaml = document.to_yaml().unwrap();
    assert!(yaml.contains("int:"));
    assert!(yaml.contains("enum:"));
    assert!(!yaml.contains("!int"));
    assert_eq!(StateDocument::parse(&yaml).unwrap(), document);
    assert_eq!(StateDocument::parse(&yaml).unwrap().to_yaml().unwrap(), yaml);
    assert!(StateDocument::parse(&yaml.replace("min:", "minimum:")).is_err());
}

#[test]
fn a_bad_legacy_payload_reports_the_field_instead_of_rejecting_its_valid_tag() {
    let source = "schema_version: 1\nscene:\n  id: 01J00000000000000000000001\n  name: Council\n  entry: !compare\n    var: has_logbook\n    op: eq\n    typo: !literal true\n";
    let error = SceneDocument::parse(source).unwrap_err();
    assert!(error.to_string().contains("unknown field `typo`"), "{error}");
    let Error::Source { location: Some(location), .. } = error else {
        panic!("located error required")
    };
    assert_eq!(location.line, 8);
}
