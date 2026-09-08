use serde_json::json;
use std::path::PathBuf;
use wobu_store::{
    NarrativeRecordDocument as Document, NarrativeRecordFile as File, NarrativeRecordKind as Kind,
    Project, SourceSave,
};
struct Fixture {
    dir: PathBuf,
    project: Project,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("wobu-records-{}", wobu_core::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let project = Project::create(&dir, "Ashfall").unwrap();
        Self { dir, project }
    }
    fn record(kind: Kind) -> File {
        File {
            document: Document::new(
                kind,
                wobu_core::new_id(),
                "Council",
                json!({"version":1,"note":"Human words 🌋","policy":"locked"}),
            ),
            stamp: None,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
#[test]
fn all_record_kinds_are_portable_and_replaceable_records_preserve_two_writers() {
    let mut f = Fixture::new();
    for kind in Kind::ALL {
        let mut file = Fixture::record(kind);
        f.project.save_narrative_record(&mut file).unwrap();
        let saved = f.project.narrative_record(kind, file.document.id).unwrap().unwrap();
        assert_eq!(saved.document, file.document);
        assert_eq!(saved.stamp, file.stamp);
        assert!(file.document.rel().starts_with("narrative/"));
        let mut rival = saved.clone();
        file.document.name = "Rewritten title".into();
        if kind.immutable() {
            assert!(f.project.save_narrative_record(&mut file).is_err());
            continue;
        }
        assert!(matches!(
            f.project.save_narrative_record(&mut file).unwrap(),
            SourceSave::Saved(_)
        ));
        rival.document.payload["note"] = json!("Other author's paragraph");
        let SourceSave::Conflict { conflict_path } =
            f.project.save_narrative_record(&mut rival).unwrap()
        else {
            panic!("conflict expected")
        };
        assert!(
            std::fs::read_to_string(f.project.root().join(conflict_path))
                .unwrap()
                .contains("Other author's paragraph")
        );
        assert_eq!(
            f.project.narrative_record(kind, file.document.id).unwrap().unwrap().document,
            file.document
        );
    }
}
#[test]
fn future_unknown_and_identity_mismatched_records_are_not_downgraded() {
    let mut f = Fixture::new();
    let mut file = Fixture::record(Kind::Scenario);
    f.project.save_narrative_record(&mut file).unwrap();
    let path = f.project.root().join(file.document.rel());
    let raw = std::fs::read_to_string(&path).unwrap();
    for invalid in [
        raw.replace("\"schema_version\": 1", "\"schema_version\": 9"),
        raw.replace("\"payload\":", "\"unknown\":true,\"payload\":"),
        raw.replace(&file.document.id.to_string(), &wobu_core::new_id().to_string()),
    ] {
        std::fs::write(&path, &invalid).unwrap();
        assert!(f.project.narrative_record(Kind::Scenario, file.document.id).is_err());
        assert!(f.project.save_narrative_record(&mut file).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
    }
}
#[test]
fn complete_publications_have_one_visibility_boundary_and_missing_objects_are_errors() {
    let mut f = Fixture::new();
    let id = wobu_core::new_id();
    let mut a = Fixture::record(Kind::Proposal).document;
    let b = Fixture::record(Kind::Receipt).document;
    let SourceSave::Saved(stamp) = f
        .project
        .publish_narrative_records(id, "Proposal and receipt", &[a.clone(), b.clone()], None)
        .unwrap()
    else {
        panic!("saved")
    };
    let published = f.project.narrative_publication(id).unwrap().unwrap();
    let manifest = published.manifest;
    let records = published.records;
    assert_eq!(records, vec![a.clone(), b.clone()]);
    // An interrupted second revision can leave an immutable object, but never
    // publishes it by directory discovery without its manifest.
    a.name = "Unpublished next revision".into();
    let text = serde_json::to_string_pretty(&a).unwrap() + "\n";
    let hash = wobu_store::atomic::hash_bytes(text.as_bytes());
    std::fs::write(f.project.root().join(format!("narrative/objects/{hash}.json")), text).unwrap();
    assert_eq!(f.project.narrative_publication(id).unwrap().unwrap().records, records);
    assert!(matches!(
        f.project.publish_narrative_records(id, "Next", &[a.clone(), b], Some(&stamp)).unwrap(),
        SourceSave::Saved(_)
    ));
    assert_eq!(f.project.narrative_publication(id).unwrap().unwrap().records[0], a);
    std::fs::remove_file(
        f.project.root().join(format!("narrative/objects/{}.json", manifest.records[1].hash)),
    )
    .unwrap();
    assert!(f.project.narrative_publication(id).is_err());
    f.project.reconcile().unwrap();
    assert!(f.project.narrative_index().unwrap().iter().any(|entry| entry.rel == manifest.rel()
        && entry.document.is_none()
        && entry.error.is_some()));
}
#[test]
fn index_rebuild_and_external_edits_never_own_accepted_text_or_protection() {
    let mut f = Fixture::new();
    let mut file = Fixture::record(Kind::Policy);
    f.project.save_narrative_record(&mut file).unwrap();
    let before = f.project.narrative_index().unwrap();
    assert_eq!(before.len(), 1);
    f.project.index().clear().unwrap();
    assert!(f.project.narrative_index().unwrap().is_empty());
    f.project.rescan().unwrap();
    assert_eq!(f.project.narrative_index().unwrap(), before);
    let path = f.project.root().join(file.document.rel());
    file.document.name = "External title".into();
    std::fs::write(&path, serde_json::to_string(&file.document).unwrap()).unwrap();
    assert!(f.project.reconcile_paths(std::slice::from_ref(&path)).unwrap());
    assert_eq!(f.project.narrative_index().unwrap()[0].name, "External title");
    std::fs::remove_file(&path).unwrap();
    assert!(f.project.reconcile().unwrap());
    assert!(f.project.narrative_index().unwrap().is_empty());
}
#[test]
fn copying_canonical_files_to_a_fresh_project_rebuilds_the_same_document() {
    let mut a = Fixture::new();
    let mut file = Fixture::record(Kind::Receipt);
    a.project.save_narrative_record(&mut file).unwrap();
    let elsewhere = a.dir.join("copied-world");
    copy_folder(a.project.root(), &elsewhere);
    let copy = Project::open_at_index(&elsewhere, &a.dir.join("fresh-machine.sqlite")).unwrap();
    assert_eq!(a.project.narrative_index().unwrap(), copy.narrative_index().unwrap());
    assert_eq!(
        copy.narrative_record(Kind::Receipt, file.document.id).unwrap().unwrap().document,
        file.document
    );
}
#[cfg(unix)]
#[test]
fn record_reads_and_writes_refuse_symlink_ancestors() {
    let mut f = Fixture::new();
    let mut file = Fixture::record(Kind::Scenario);
    f.project.save_narrative_record(&mut file).unwrap();
    let source = f.project.root().join("narrative/scenarios");
    let moved = f.project.root().join("other");
    std::fs::rename(&source, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &source).unwrap();
    assert!(f.project.narrative_record(Kind::Scenario, file.document.id).is_err());
    assert!(f.project.save_narrative_record(&mut file).is_err());
}

fn copy_folder(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == ".wobu" {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_folder(&entry.path(), &target)
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
#[test]
fn immutable_receipt_identity_survives_publication_updates_and_both_write_routes() {
    let mut f = Fixture::new();
    let receipt = Fixture::record(Kind::Receipt).document;
    let id = wobu_core::new_id();
    let SourceSave::Saved(stamp) = f
        .project
        .publish_narrative_records(id, "First", std::slice::from_ref(&receipt), None)
        .unwrap()
    else {
        panic!("saved")
    };
    let mut different = receipt.clone();
    different.payload["note"] = json!("Changed receipt");
    assert!(
        f.project
            .publish_narrative_records(id, "Next", std::slice::from_ref(&different), Some(&stamp))
            .is_err()
    );
    let mut standalone = File { document: different, stamp: None };
    assert!(f.project.save_narrative_record(&mut standalone).is_err());
    standalone.document = receipt.clone();
    f.project.save_narrative_record(&mut standalone).unwrap();
    assert_eq!(f.project.narrative_publication(id).unwrap().unwrap().records, vec![receipt]);
    let entries = f.project.narrative_index().unwrap();
    assert!(
        entries
            .iter()
            .filter(|entry| matches!(
                entry.kind,
                wobu_store::NarrativeFileKind::Object
                    | wobu_store::NarrativeFileKind::ReceiptBinding
            ))
            .all(|entry| !entry.visible)
    );
    let mut direct = Fixture::record(Kind::Receipt);
    f.project.save_narrative_record(&mut direct).unwrap();
    // Even a legacy standalone receipt with no binding must reserve its actual
    // bytes, not the first newly proposed payload claiming the old identity.
    std::fs::remove_file(
        f.project.root().join(format!("narrative/receipt-bindings/{}.json", direct.document.id)),
    )
    .unwrap();
    direct.document.name = "Reused identity".into();
    assert!(
        f.project
            .publish_narrative_records(wobu_core::new_id(), "Collision", &[direct.document], None)
            .is_err()
    );
}
