use std::path::PathBuf;
use wobu_store::{Project, SourceSave, atomic};
struct Fixture {
    dir: PathBuf,
    project: Project,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wobu-deletions-{}", wobu_core::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let project = Project::create(&dir, "Ashfall").unwrap();
        Self { dir, project }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
#[test]
fn delete_restore_and_index_rebuild_are_portable_repeat_safe_and_preserve_identity() {
    let mut f = Fixture::new();
    let scene = f.project.create_scene("Council").unwrap();
    let path = f.project.root().join(&scene.rel);
    let original = std::fs::read_to_string(&path).unwrap();
    f.project.delete_scene(scene.scene.id).unwrap();
    assert!(!path.exists());
    let deletion = f.project.narrative_deletions().unwrap().remove(0);
    assert!(!deletion.restored);
    assert_eq!(deletion.deletion.original, original);
    assert!(!f.project.apply_narrative_deletions().unwrap());
    // A raw old copy cannot silently undo a canonical deletion after a rebuild.
    std::fs::write(&path, &original).unwrap();
    f.project.index().clear().unwrap();
    f.project.rescan().unwrap();
    assert!(!path.exists());
    assert!(matches!(
        f.project.restore_narrative_deletion(deletion.deletion.id).unwrap(),
        SourceSave::Saved(_)
    ));
    assert_eq!(f.project.load_scene(scene.scene.id).unwrap().scene, scene.scene);
    f.project.index().clear().unwrap();
    f.project.rescan().unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    assert!(!f.project.apply_narrative_deletions().unwrap());
    assert!(f.project.narrative_deletions().unwrap()[0].restored);
    // A later deletion is a separate operation, not revoked by the old restore.
    f.project.delete_scene(scene.scene.id).unwrap();
    f.project.apply_narrative_deletions().unwrap();
    assert!(!path.exists());
}
#[test]
fn delete_and_restore_never_overwrite_a_concurrent_newer_version() {
    let mut f = Fixture::new();
    let scene = f.project.create_scene("Council").unwrap();
    let path = f.project.root().join(&scene.rel);
    let (original, stamp) = atomic::read_stamped(&path).unwrap().unwrap();
    let newer = original.replace("Council", "New council");
    std::fs::write(&path, &newer).unwrap();
    assert!(!f.project.delete_narrative_file(&scene.rel, &stamp).unwrap());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);
    std::fs::write(&path, &original).unwrap();
    f.project.delete_narrative_file(&scene.rel, &stamp).unwrap();
    let deletion = f.project.narrative_deletions().unwrap().remove(0).deletion;
    std::fs::write(&path, &newer).unwrap();
    assert!(!f.project.apply_narrative_deletions().unwrap());
    let SourceSave::Conflict { conflict_path } =
        f.project.restore_narrative_deletion(deletion.id).unwrap()
    else {
        panic!("newer version must win")
    };
    assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);
    assert_eq!(std::fs::read_to_string(f.project.root().join(conflict_path)).unwrap(), original);
}
#[test]
fn interrupted_delete_and_restore_finish_from_canonical_records_without_cache() {
    let mut f = Fixture::new();
    let scene = f.project.create_scene("Council").unwrap();
    let path = f.project.root().join(&scene.rel);
    let (original, stamp) = atomic::read_stamped(&path).unwrap().unwrap();
    let deletion = wobu_store::NarrativeDeletion {
        schema_version: 1,
        id: wobu_core::new_id(),
        name: "Council".into(),
        target: scene.rel.clone(),
        hash: stamp.hash,
        original: original.clone(),
    };
    let marker = f.project.root().join(deletion.rel());
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(marker, serde_json::to_string(&deletion).unwrap()).unwrap();
    f.project.rescan().unwrap();
    assert!(!path.exists());
    let restoration = wobu_store::NarrativeRestoration {
        schema_version: 1,
        id: wobu_core::new_id(),
        deletion: deletion.id,
    };
    let marker = f.project.root().join(restoration.rel());
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(marker, serde_json::to_string(&restoration).unwrap()).unwrap();
    f.project.rescan().unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}
