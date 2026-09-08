use super::*;
use wobu_narrative::Fact;

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("wobu-world-cmd-{}", wobu_core::new_id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn edited() -> WorldDocument {
    WorldDocument {
        facts: vec![Fact {
            id: wobu_core::new_id(),
            name: "Arrival".into(),
            assertion: "The ship arrived.".into(),
            sources: vec![],
            entity_ids: vec![],
        }],
        ..WorldDocument::default()
    }
}

#[test]
fn missing_world_reads_empty_and_stale_writes_park_the_loser() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "World").unwrap();
    let initial = get(&project).unwrap();
    assert!(initial.stamp.is_none());
    let first = save(&mut project, edited(), &Precondition::New).unwrap();
    let mut next = first.document.clone();
    next.facts[0].name = "Renamed".into();
    save(&mut project, next.clone(), &Precondition::Stamp { stamp: first.stamp.clone().unwrap() })
        .unwrap();
    assert!(
        save(&mut project, first.document, &Precondition::Stamp { stamp: first.stamp.unwrap() })
            .is_err()
    );
    assert_eq!(get(&project).unwrap().document, next);
    assert!(
        std::fs::read_dir(project.root().join("narrative")).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".conflict-"))
    );
}

#[test]
fn restore_compares_snapshot_and_never_overwrites_external_edits() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "World").unwrap();
    let old = WorldDocument::default();
    let first = save(&mut project, edited(), &Precondition::New).unwrap();
    let undone = restore(&mut project, old.clone(), &first.document).unwrap();
    restore(&mut project, first.document.clone(), &undone.document).unwrap();
    let mut external = first.document.clone();
    external.facts[0].assertion = "Someone else's correction.".into();
    let current = get(&project).unwrap();
    save(&mut project, external.clone(), &Precondition::Stamp { stamp: current.stamp.unwrap() })
        .unwrap();
    assert!(restore(&mut project, old, &first.document).is_err());
    assert_eq!(get(&project).unwrap().document, external);
}

#[test]
fn current_future_version_and_malformed_disk_cannot_be_overwritten() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "World").unwrap();
    assert!(save(&mut project, edited(), &Precondition::Current).is_err());
    let mut future = edited();
    future.schema_version = 999;
    assert!(save(&mut project, future, &Precondition::New).is_err());
    let saved = save(&mut project, edited(), &Precondition::New).unwrap();
    let path = project.root().join("narrative/world.yaml");
    std::fs::write(&path, "schema_version: 999\nfuture: data\n").unwrap();
    assert!(
        save(&mut project, edited(), &Precondition::Stamp { stamp: saved.stamp.unwrap() }).is_err()
    );
    assert!(std::fs::read_to_string(path).unwrap().contains("future: data"));
}

#[test]
fn world_edits_affect_source_hash_but_layout_does_not() {
    let temp = Temp::new();
    let mut project = Project::create(&temp.0, "World").unwrap();
    let before = project.narrative_fingerprint().unwrap();
    save(&mut project, edited(), &Precondition::New).unwrap();
    let after = project.narrative_fingerprint().unwrap();
    assert_ne!(before, after);
    std::fs::create_dir_all(project.root().join("narrative/layout")).unwrap();
    std::fs::write(project.root().join("narrative/layout/anything.json"), "{}").unwrap();
    assert_eq!(after, project.narrative_fingerprint().unwrap());
}
