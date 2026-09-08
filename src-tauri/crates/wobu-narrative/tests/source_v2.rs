use wobu_narrative::*;

fn legacy_scene() -> String {
    "schema_version: 1\nscene:\n  id: 01J00000000000000000000001\n  name: Legacy\n".into()
}
#[test]
fn old_documents_read_without_changing_embedded_scene_or_world_shapes() {
    let scene = SceneDocument::parse(&legacy_scene()).unwrap();
    assert_eq!(scene.schema_version, 1);
    assert_eq!(
        serde_json::to_value(&scene.scene).unwrap(),
        serde_json::json!({
            "id":"01J00000000000000000000001", "name":"Legacy"
        })
    );
    let world = WorldDocument::parse("schema_version: 1\n").unwrap();
    assert_eq!(world, WorldDocument::default());
    assert_eq!(
        serde_json::to_value(&world).unwrap(),
        serde_json::json!({
            "schema_version":1,"facts":[],"knowledge":[],"relationships":[],"events":[],"quests":[],"restrictions":[]
        })
    );
    assert_eq!(SceneDocument::new(scene.scene).schema_version, 2);
    assert_eq!(world.for_save().unwrap().schema_version, 2);
    assert_eq!(StateDocument::new(vec![]).schema_version, 1);
    assert!(StateDocument::parse("schema_version: 2\nvariables: []\n").is_err());
    assert!(StateDocument { schema_version: 2, variables: vec![] }.to_yaml().is_err());
}
#[test]
fn v1_cannot_claim_new_fields_even_when_empty_and_future_versions_fail_first() {
    for field in ["act_id: null", "arc_id: null", "tag_ids: []"] {
        let yaml = format!("{}  {field}\n", legacy_scene());
        assert!(SceneDocument::parse(&yaml).unwrap_err().to_string().contains("version 2"));
    }
    for field in ["acts", "arcs", "tags"] {
        assert!(WorldDocument::parse(&format!("schema_version: 1\n{field}: []\n")).is_err());
    }
    let future = legacy_scene().replace("schema_version: 1", "schema_version: 3");
    assert!(matches!(
        SceneDocument::parse(&future),
        Err(Error::UnsupportedSchemaVersion { found: 3, supported: 2 })
    ));
    let invalid = WorldDocument { schema_version: 3, ..Default::default() };
    assert!(invalid.for_save().is_err());
}
#[test]
fn unresolved_is_a_saveable_draft_with_exact_destination_diagnostics() {
    let mut scene = Scene::new("Unfinished");
    let mut beat = Beat::new("Question");
    beat.choices.push(Choice::new("Ask", Destination::Unresolved {}));
    beat.outcomes.push(Outcome::new(Destination::Unresolved {}));
    scene.beats.push(beat);
    let document = SceneDocument::new(scene.clone());
    let yaml = document.to_yaml().unwrap();
    assert!(yaml.contains("unresolved: {}"));
    assert_eq!(SceneDocument::parse(&yaml).unwrap(), document);
    assert!(SceneDocument::parse(&yaml.replace("schema_version: 2", "schema_version: 1")).is_err());
    let issues = scene.source_diagnostics(&StateSchema::default(), &SceneCatalog::unknown());
    assert_eq!(issues.len(), 2);
    for (issue, path) in issues {
        assert_eq!(issue.problem, Problem::UnresolvedDestination);
        assert_eq!(path.last(), Some(&SourcePathPart::Key("to".into())));
    }
}
#[test]
fn classifications_have_stable_identity_and_duplicate_scenes_keep_membership() {
    let record = NamedClassification { id: wobu_core::Id::generate(), name: "Act one".into() };
    let mut world =
        WorldDocument { acts: vec![record.clone()], ..Default::default() }.for_save().unwrap();
    let mut scene = Scene::new("Opening");
    scene.act_id = Some(record.id);
    scene.arc_id = Some(wobu_core::Id::generate());
    scene.tag_ids.push(wobu_core::Id::generate());
    let copy = scene.duplicated();
    assert_ne!(scene.id, copy.id);
    assert_eq!(
        (copy.act_id, copy.arc_id, copy.tag_ids),
        (scene.act_id, scene.arc_id, scene.tag_ids.clone())
    );
    world.acts[0].name = "Renamed act".into();
    assert_eq!(world.acts[0].id, record.id);
    assert_eq!(WorldDocument::parse(&world.to_yaml().unwrap()).unwrap(), world);
    world.tags.push(record);
    assert!(
        world
            .diagnose(
                &StateSchema::default(),
                &Default::default(),
                &Default::default(),
                &Default::default()
            )
            .iter()
            .any(|d| d.field == "id")
    );
}

#[test]
fn classification_diagnostics_address_missing_and_repeated_references() {
    let mut scene = Scene::new("Classified");
    let tag = wobu_core::Id::generate();
    scene.act_id = Some(wobu_core::Id::generate());
    scene.arc_id = Some(wobu_core::Id::generate());
    scene.tag_ids = vec![tag, tag];
    let world = WorldDocument::default();
    let issues = scene.classification_diagnostics(&world);
    assert_eq!(issues.len(), 4);
    assert_eq!(issues[0].1, vec!["scene".into(), "act_id".into()]);
    assert_eq!(issues[1].1, vec!["scene".into(), "arc_id".into()]);
    assert_eq!(issues[2].1, vec!["scene".into(), "tag_ids".into(), 0usize.into()]);
    assert!(matches!(issues[3].0.problem, Problem::DuplicateId { .. }));
}
