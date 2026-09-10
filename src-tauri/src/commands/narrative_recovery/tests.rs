use super::*;

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("wobu-recovery-ui-{}", wobu_core::new_id())))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn list_exposes_named_metadata_without_retained_bytes_and_restore_preserves_the_original() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Recovery").unwrap();
    let file = project.create_scene("Council hearing").unwrap();
    let path = project.root().join(&file.rel);
    let original = std::fs::read(&path).unwrap();
    project.delete_narrative_file(&file.rel, file.stamp.as_ref().unwrap()).unwrap();
    let rows = list(&project).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "Council hearing");
    assert_eq!(rows[0].target, file.rel);
    assert!(!rows[0].restored);
    let json = serde_json::to_value(&rows[0]).unwrap();
    assert!(json.get("original").is_none());
    assert_eq!(json.as_object().unwrap().len(), 5);
    assert!(!path.exists());
    assert!(matches!(restore(&mut project, rows[0].id).unwrap(), Restoration::Saved { .. }));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(list(&project).unwrap()[0].restored);
    assert!(restore(&mut project, wobu_core::new_id()).is_err());
}
#[test]
fn restore_returns_conflict_path_and_preserves_both_named_versions() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "Recovery").unwrap();
    let mut file = project.create_scene("Original council").unwrap();
    let path = project.root().join(&file.rel);
    let original = std::fs::read(&path).unwrap();
    project.delete_narrative_file(&file.rel, file.stamp.as_ref().unwrap()).unwrap();
    let id = list(&project).unwrap()[0].id;
    file.scene.name = "Newer council".into();
    file.stamp = None;
    project.save_scene(&mut file).unwrap();
    let newer = std::fs::read(&path).unwrap();
    let Restoration::Conflict { conflict_path, .. } = restore(&mut project, id).unwrap() else {
        panic!("expected preserved competitor")
    };
    assert_eq!(std::fs::read(path).unwrap(), newer);
    let conflict = std::path::PathBuf::from(&conflict_path);
    let conflict = if conflict.is_absolute() { conflict } else { project.root().join(conflict) };
    assert_eq!(std::fs::read(conflict).unwrap(), original);
    assert!(
        list(&project).unwrap()[0].restored,
        "status indicates request, not that old content won"
    );
}
