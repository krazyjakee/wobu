use serde_json::json;
use wobu_narrative::{Beat, DialogueSlot, GenerationPolicy, Speaker, Text, Variant};
use wobu_store::{
    NarrativeApplied, NarrativeIncoming, NarrativeRecordDocument as Document,
    NarrativeRecordFile as File, NarrativeRecordKind as Kind, NarrativeSyncEntry, Project,
};

fn fixture() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::create(dir.path(), "Ashfall").unwrap();
    (dir, project)
}
fn incoming(project: &Project, rel: &str) -> NarrativeIncoming {
    let entry = project.narrative_manifest().unwrap().into_iter().find(|e| e.rel == rel).unwrap();
    project.narrative_outgoing(&entry).unwrap().unwrap()
}
fn copy(source: &Project, target: &mut Project) {
    for entry in source.narrative_manifest().unwrap() {
        assert!(matches!(
            target
                .apply_narrative_from_peer(
                    "writer",
                    &source.narrative_outgoing(&entry).unwrap().unwrap()
                )
                .unwrap(),
            NarrativeApplied::Agreed { .. }
        ));
    }
}
#[test]
fn peer_bases_are_local_disposable_and_concurrent_bytes_remain_recoverable() {
    let (_a, mut a) = fixture();
    let (_b, mut b) = fixture();
    let mut file = File {
        document: Document::new(Kind::Scenario, wobu_core::new_id(), "Kiln", json!({"version":1})),
        stamp: None,
    };
    a.save_narrative_record(&mut file).unwrap();
    copy(&a, &mut b);
    file.document.name = "One-sided edit".into();
    a.save_narrative_record(&mut file).unwrap();
    let update = incoming(&a, &file.document.rel());
    assert!(matches!(
        b.apply_narrative_from_peer("writer", &update).unwrap(),
        NarrativeApplied::Agreed { changed: true }
    ));
    // Unsharing forgets narrative bases in the same transaction as node bases.
    b.index().forget_peer("writer").unwrap();
    file.document.name = "Edit after unshare".into();
    a.save_narrative_record(&mut file).unwrap();
    assert!(matches!(
        b.apply_narrative_from_peer("writer", &incoming(&a, &file.document.rel())).unwrap(),
        NarrativeApplied::Conflict { .. }
    ));
    // A fresh derived index has no authority to infer which side edited last.
    let root = b.root().to_path_buf();
    drop(b);
    let mut b = Project::open_at_index(&root, &_b.path().join("fresh.sqlite")).unwrap();
    file.document.name = "Later edit".into();
    a.save_narrative_record(&mut file).unwrap();
    let update = incoming(&a, &file.document.rel());
    let NarrativeApplied::Conflict { path } =
        b.apply_narrative_from_peer("writer", &update).unwrap()
    else {
        panic!("missing base must conflict")
    };
    assert!(std::fs::read_to_string(root.join(&path)).unwrap().contains("Later edit"));
    assert_eq!(
        b.narrative_record(Kind::Scenario, file.document.id).unwrap().unwrap().document.name,
        "One-sided edit"
    );
    assert_eq!(
        b.apply_narrative_from_peer("writer", &update).unwrap(),
        NarrativeApplied::Conflict { path }
    );
}
#[test]
fn a_peer_cannot_replace_or_downgrade_protected_text_even_with_an_agreed_base() {
    for policy in [GenerationPolicy::Edited, GenerationPolicy::Locked] {
        let (_a, mut a) = fixture();
        let (_b, mut b) = fixture();
        let mut file = a.create_scene("Kiln").unwrap();
        let mut beat = Beat::new("Opening");
        let mut slot = DialogueSlot::new(Speaker::Narrator);
        let mut variant = Variant::new(Text::written("The kiln is warm."));
        variant.text.lifecycle.policy = policy;
        slot.variants.push(variant);
        beat.dialogue.push(slot);
        file.scene.beats.push(beat);
        a.save_scene(&mut file).unwrap();
        copy(&a, &mut b);
        // Simulate an untrusted peer bypassing its own UI protection.
        let mut document = wobu_narrative::SceneDocument::new(file.scene.clone());
        let text = &mut document.scene.beats[0].dialogue[0].variants[0].text;
        text.body = "The peer rewrote the author.".into();
        text.lifecycle.policy = GenerationPolicy::Generated;
        let raw = document.to_yaml().unwrap();
        let update = NarrativeIncoming {
            rel: file.rel.clone(),
            hash: wobu_store::atomic::hash_bytes(raw.as_bytes()),
            text: raw,
        };
        assert!(matches!(
            b.apply_narrative_from_peer("writer", &update).unwrap(),
            NarrativeApplied::Conflict { .. }
        ));
        assert_eq!(
            b.load_scene(file.scene.id).unwrap().scene.beats[0].dialogue[0].variants[0].text.body,
            "The kiln is warm."
        );
    }
}
#[test]
fn publications_and_deletions_replicate_with_portable_dependencies_and_explicit_restore() {
    let (_a, mut a) = fixture();
    let (_b, mut b) = fixture();
    let receipt =
        Document::new(Kind::Receipt, wobu_core::new_id(), "Receipt", json!({"version":1}));
    let publication = wobu_core::new_id();
    a.publish_narrative_records(publication, "Kiln", &[receipt], None).unwrap();
    copy(&a, &mut b);
    assert!(b.narrative_publication(publication).unwrap().is_some());
    let file = a.create_scene("Kiln").unwrap();
    copy(&a, &mut b);
    a.delete_narrative_file(&file.rel, file.stamp.as_ref().unwrap()).unwrap();
    copy(&a, &mut b);
    b.apply_narrative_deletions().unwrap();
    assert!(!b.root().join(&file.rel).exists());
    let deletion = a.narrative_deletions().unwrap().pop().unwrap().deletion.id;
    a.restore_narrative_deletion(deletion).unwrap();
    copy(&a, &mut b);
    b.apply_narrative_deletions().unwrap();
    assert_eq!(b.load_scene(file.scene.id).unwrap().scene, file.scene);
}
#[test]
fn hashes_paths_future_versions_and_size_bounds_are_enforced_before_writing() {
    let (_dir, mut project) = fixture();
    let mut file = File {
        document: Document::new(Kind::Scenario, wobu_core::new_id(), "Kiln", json!({"version":1})),
        stamp: None,
    };
    project.save_narrative_record(&mut file).unwrap();
    let valid = incoming(&project, &file.document.rel());
    let (_other, mut receiver) = fixture();
    let mut invalid = valid.clone();
    invalid.rel = "narrative/scenarios/../../secrets.json".into();
    assert!(receiver.apply_narrative_from_peer("peer", &invalid).is_err());
    let mut invalid = valid.clone();
    invalid.hash = "a".repeat(64);
    assert!(receiver.apply_narrative_from_peer("peer", &invalid).is_err());
    let mut future = file.document.clone();
    future.schema_version = 99;
    let raw = serde_json::to_string(&future).unwrap();
    let invalid = NarrativeIncoming {
        rel: valid.rel.clone(),
        hash: wobu_store::atomic::hash_bytes(raw.as_bytes()),
        text: raw,
    };
    assert!(receiver.apply_narrative_from_peer("peer", &invalid).is_err());
    let text = " ".repeat(wobu_store::MAX_NARRATIVE_FILE_BYTES + 1);
    let invalid = NarrativeIncoming {
        rel: valid.rel.clone(),
        hash: wobu_store::atomic::hash_bytes(text.as_bytes()),
        text,
    };
    assert!(receiver.apply_narrative_from_peer("peer", &invalid).is_err());
    assert!(receiver.narrative_manifest().unwrap().is_empty());
    assert!(
        receiver
            .record_narrative_agreed(
                "peer",
                &NarrativeSyncEntry { rel: valid.rel, hash: "not a hash".into() }
            )
            .is_err()
    );
}
