use wobu_narrative::{NamedClassification, WorldDocument};
use wobu_store::{Project, SourceSave, atomic};

#[test]
fn explicit_world_writes_upgrade_with_cas_and_cannot_overwrite_future_source() {
    let temp = tempfile::tempdir().unwrap();
    let mut project = Project::create(temp.path(), "Legacy world").unwrap();
    let path = project.root().join("narrative/world.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes = "schema_version: 1\nfacts: []\n";
    std::fs::write(&path, bytes).unwrap();
    project.rebuild_index().unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
    let (mut world, old_stamp) = project.world_document().unwrap().unwrap();
    assert_eq!(world.schema_version, 1);
    let id = wobu_core::Id::generate();
    world.acts.push(NamedClassification { id, name: "First act".into() });
    assert!(matches!(project.save_world(&world, Some(&old_stamp)).unwrap(), SourceSave::Saved(_)));
    let (saved, _) = project.world_document().unwrap().unwrap();
    assert_eq!(saved.schema_version, 2);
    assert_eq!(saved.acts[0].id, id);
    world.acts[0].name = "Stale edit".into();
    assert!(matches!(
        project.save_world(&world, Some(&old_stamp)).unwrap(),
        SourceSave::Conflict { .. }
    ));
    assert_eq!(project.world_document().unwrap().unwrap().0, saved);
    let future = "schema_version: 3\nunknown_future_content: preserve\n";
    std::fs::write(&path, future).unwrap();
    let (_, future_stamp) = atomic::read_stamped(&path).unwrap().unwrap();
    assert!(project.save_world(&WorldDocument::default(), Some(&future_stamp)).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), future);
}
