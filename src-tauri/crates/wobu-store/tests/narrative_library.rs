use std::fs;
use wobu_narrative::{
    Beat, DialogueSlot, Freshness, GenerationPolicy, Intent, NamedClassification, Participant,
    Provenance, Quest, Scene, SceneDocument, Speaker, Text, Variant, WorldDocument,
};
use wobu_store::{
    Project,
    project::narrative_library::{LibraryQuery, QueryError},
};

fn authored(name: &str, words: &str) -> Scene {
    let mut scene = Scene::new(name);
    let mut beat = Beat::new("Evidence");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written(words)));
    beat.dialogue.push(slot);
    scene.beats.push(beat);
    scene
}
fn put(project: &Project, slug: &str, scene: &Scene) -> std::path::PathBuf {
    let path = project.root().join(format!("narrative/scenes/{slug}.yaml"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, SceneDocument::new(scene.clone()).to_yaml().unwrap()).unwrap();
    path
}
fn fixture() -> (tempfile::TempDir, Project, Scene, Scene) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::create(dir.path(), "Discovery").unwrap();
    let a = authored("Council", "The beacon went dark.");
    let b = authored("Harbour", "A witness arrived.");
    put(&project, "council", &a);
    put(&project, "harbour", &b);
    (dir, project, a, b)
}
#[test]
fn bounded_search_rebuilds_from_files_and_returns_stable_line_targets() {
    let (_dir, mut project, a, _) = fixture();
    let query = LibraryQuery { query: "BEACON".into(), limit: 1, ..Default::default() };
    let page = project.library_query(&query).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.rows.len(), 1);
    assert_eq!(
        page.rows[0].matches[0].line_id.as_deref(),
        Some(a.beats[0].dialogue[0].id.to_string().as_str())
    );
    assert_eq!(page.rows[0].summary.id, a.id.to_string());
    let before = serde_json::to_value(page).unwrap();
    project.rebuild_index().unwrap();
    assert_eq!(serde_json::to_value(project.library_query(&query).unwrap()).unwrap(), before);
    assert_eq!(project.load_scene(a.id).unwrap().scene, a);
}
#[test]
fn same_mtime_duplicate_identity_and_malformed_source_are_never_hidden() {
    let (_dir, project, a, b) = fixture();
    project.library_query(&Default::default()).unwrap();
    let path = project.root().join("narrative/scenes/harbour.yaml");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let bytes = fs::read_to_string(&path).unwrap();
    fs::write(&path, bytes.replace(&b.id.to_string(), &a.id.to_string())).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert!(project.load_scene(a.id).unwrap_err().to_string().contains("ambiguous"));
    let page = project.library_query(&Default::default()).unwrap();
    assert_eq!(page.unreadable_total, 2);
    assert!(page.rows.is_empty());
    fs::remove_file(&path).unwrap();
    assert_eq!(project.load_scene(a.id).unwrap().scene.id, a.id);
    fs::write(&path, "schema_version: 2\nscene: [").unwrap();
    let page = project.library_query(&Default::default()).unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.unreadable_total, 1);
}
#[test]
fn stale_pages_and_oversized_queries_require_an_explicit_restart() {
    let (_dir, project, mut a, _) = fixture();
    let first = project.library_query(&LibraryQuery { limit: 1, ..Default::default() }).unwrap();
    a.name = "Renamed".into();
    put(&project, "council", &a);
    let next =
        LibraryQuery { offset: 1, limit: 1, revision: Some(first.revision), ..Default::default() };
    assert!(matches!(project.library_query(&next), Err(QueryError::StaleRevision)));
    assert!(project.library_query(&LibraryQuery { limit: 101, ..Default::default() }).is_err());
    assert!(
        project
            .library_query(&LibraryQuery { query: "x".repeat(501), ..Default::default() })
            .is_err()
    );
    let lookup = project
        .library_query(&LibraryQuery {
            ids: vec![a.id.to_string(), wobu_core::Id::generate().to_string()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(lookup.rows.len(), 1);
    assert_eq!(lookup.missing_ids.len(), 1);
}
#[test]
fn combined_classifications_and_multiple_quests_keep_one_scene_row() {
    let (_dir, mut project, mut a, _) = fixture();
    let record = NamedClassification { id: wobu_core::Id::generate(), name: "Arrival".into() };
    let tag = NamedClassification { id: wobu_core::Id::generate(), name: "Politics".into() };
    let arc = NamedClassification { id: wobu_core::Id::generate(), name: "Inquiry".into() };
    a.act_id = Some(record.id);
    a.arc_id = Some(arc.id);
    a.tag_ids.push(tag.id);
    put(&project, "council", &a);
    let quest = |name: &str| Quest {
        id: wobu_core::Id::generate(),
        name: name.into(),
        summary: String::new(),
        stages: vec![wobu_narrative::Name::new("started").unwrap().into()],
        initial: "started".parse().unwrap(),
        transitions: vec![],
        scene_ids: vec![a.id, a.id],
    };
    let world = WorldDocument {
        acts: vec![record.clone()],
        arcs: vec![arc.clone()],
        tags: vec![tag.clone()],
        quests: vec![quest("Inquiry"), quest("Council")],
        ..Default::default()
    };
    project.save_world(&world, None).unwrap();
    for quest in &world.quests {
        let page = project
            .library_query(&LibraryQuery {
                act: record.id.to_string(),
                arc: arc.id.to_string(),
                tag: tag.id.to_string(),
                quest: quest.id.to_string(),
                query: "beacon".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].quests.len(), 2);
        assert_eq!(page.rows[0].act.as_ref().unwrap().name, "Arrival");
        assert_eq!(page.rows[0].arc.as_ref().unwrap().name, "Inquiry");
    }
}
#[test]
fn generated_draft_search_is_explicit_and_filters_describe_the_same_variant() {
    let (_dir, project, mut a, _) = fixture();
    let slot = &mut a.beats[0].dialogue[0];
    slot.variants[0].text.set_body(
        "Secret generated words",
        Provenance::Generated { fingerprint: "fixture".into() },
    );
    slot.variants[0].text.lifecycle.policy = GenerationPolicy::Generated;
    let mut approved = Variant::new(Text::written("Approved words"));
    approved.text.lifecycle.review = wobu_narrative::ReviewState::Approved;
    slot.variants.push(approved);
    put(&project, "council", &a);
    let query = LibraryQuery { query: "Secret".into(), ..Default::default() };
    assert_eq!(project.library_query(&query).unwrap().total, 0);
    assert_eq!(
        project.library_query(&LibraryQuery { include_drafts: true, ..query }).unwrap().total,
        1
    );
    let mismatch = LibraryQuery {
        policy: "generated".into(),
        review: "approved".into(),
        ..Default::default()
    };
    assert_eq!(project.library_query(&mismatch).unwrap().total, 0);
}

#[test]
fn snippets_map_expanding_lowercase_back_to_original_characters() {
    let (_dir, project, mut scene, _) = fixture();
    scene.beats[0].dialogue[0].variants[0].text =
        Text::written(format!("{} unique-target {}", "İ".repeat(300), "z".repeat(500)));
    put(&project, "council", &scene);
    for query in ["unique-target".to_owned(), format!("unique-target {}", "z".repeat(480))] {
        let page = project.library_query(&LibraryQuery { query, ..Default::default() }).unwrap();
        let snippet = &page.rows[0].matches[0].snippet;
        assert!(snippet.contains("unique-target"), "{snippet}");
        assert!(snippet.chars().count() <= 182);
    }
}

#[test]
fn unreadable_files_and_matches_are_bounded_independently() {
    let (_dir, project, mut scene, _) = fixture();
    for index in 0..12 {
        scene.beats[0].dialogue[0]
            .variants
            .push(Variant::new(Text::written(format!("beacon {index}"))));
    }
    put(&project, "council", &scene);
    for index in 0..7 {
        fs::write(project.root().join(format!("narrative/scenes/broken-{index}.yaml")), "scene: [")
            .unwrap();
    }
    let query = LibraryQuery { query: "beacon".into(), limit: 3, ..Default::default() };
    let first = project.library_query(&query).unwrap();
    assert_eq!(first.rows[0].match_count, 13);
    assert_eq!(first.rows[0].matches.len(), 5);
    assert_eq!(first.unreadable_total, 7);
    assert_eq!(first.unreadable.len(), 3);
    let second = project
        .library_query(&LibraryQuery {
            revision: Some(first.revision),
            unreadable_offset: first.unreadable_next_offset.unwrap(),
            ..query
        })
        .unwrap();
    assert_eq!(second.unreadable.len(), 3);
    assert_ne!(first.unreadable[0].rel, second.unreadable[0].rel);
}

#[test]
fn world_membership_changes_invalidate_pages_without_changing_scenes() {
    let (_dir, mut project, _, _) = fixture();
    let first = project.library_query(&Default::default()).unwrap();
    let world = WorldDocument {
        tags: vec![NamedClassification { id: wobu_core::Id::generate(), name: "New facet".into() }],
        ..Default::default()
    };
    project.save_world(&world, None).unwrap();
    assert!(matches!(
        project
            .library_query(&LibraryQuery { revision: Some(first.revision), ..Default::default() }),
        Err(QueryError::StaleRevision)
    ));
    assert_eq!(
        project.library_query(&Default::default()).unwrap().facets.tags[0].name,
        "New facet"
    );
}

#[test]
fn intent_participant_and_lifecycle_filters_keep_exact_saved_targets() {
    let (_dir, project, mut scene, _) = fixture();
    let character = wobu_core::Id::generate();
    scene.participants.push(Participant { entity: character, role: "Witness".into() });
    scene.beats[0]
        .intents
        .push(Intent { subject: Speaker::Narrator, intent: "Convince the harbour council".into() });
    let text = &mut scene.beats[0].dialogue[0].variants[0].text;
    text.lifecycle.policy = GenerationPolicy::Locked;
    text.lifecycle.review = wobu_narrative::ReviewState::Approved;
    text.lifecycle.freshness = Freshness::OutOfDate;
    put(&project, "council", &scene);
    let query = LibraryQuery {
        query: "convince".into(),
        participant: character.to_string(),
        policy: "locked".into(),
        review: "approved".into(),
        freshness: "out_of_date".into(),
        ..Default::default()
    };
    let page = project.library_query(&query).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(
        page.rows[0].matches[0].beat_id.as_deref(),
        Some(scene.beats[0].id.to_string().as_str())
    );
    assert!(page.rows[0].matches[0].line_id.is_none());
    for mismatch in [
        LibraryQuery { participant: wobu_core::Id::generate().to_string(), ..query.clone() },
        LibraryQuery { freshness: "current".into(), ..query.clone() },
        LibraryQuery { review: "draft".into(), ..query.clone() },
    ] {
        assert_eq!(project.library_query(&mismatch).unwrap().total, 0);
    }
    // A source-order change must not redirect a search result to another variant.
    scene.beats[0].dialogue[0].variants.insert(0, Variant::new(Text::written("Different words")));
    put(&project, "council", &scene);
    let page = project
        .library_query(&LibraryQuery { query: "beacon".into(), ..Default::default() })
        .unwrap();
    assert_eq!(
        page.rows[0].matches[0].variant_id.as_deref(),
        Some(scene.beats[0].dialogue[0].variants[1].id.to_string().as_str())
    );
}

#[test]
fn whole_arc_keeps_authored_exit_ids_without_prose_and_observes_external_damage() {
    use wobu_narrative::{Choice, Destination, Outcome};
    let (_dir, project, mut a, b) = fixture();
    a.beats[0].choices.push(Choice::new("Take the ferry", Destination::Scene(b.id)));
    a.beats[0].outcomes.push(Outcome::new(Destination::Unresolved {}));
    a.beats[0].outcomes.push(Outcome::new(Destination::End { label: "Harbour".into() }));
    put(&project, "council", &a);
    let arc = project.narrative_arc().unwrap();
    assert_eq!(arc.scenes.len(), 2);
    let details = arc
        .scenes
        .iter()
        .find(|one| one.summary.id == a.id.to_string())
        .unwrap()
        .arc
        .as_ref()
        .unwrap();
    assert_eq!(details.exits.len(), 2);
    assert_eq!(details.exits[0].route_id, a.beats[0].choices[0].id.to_string());
    assert_eq!(details.exits[0].beat_id, a.beats[0].id.to_string());
    assert_eq!(details.exits[0].to, Some(b.id.to_string()));
    assert_eq!(details.exits[1].to, None);
    let wire = serde_json::to_string(&arc).unwrap();
    assert!(!wire.contains("The beacon went dark."));
    assert!(!wire.contains("A witness arrived."));
    fs::write(project.root().join("narrative/scenes/harbour.yaml"), "scene: [").unwrap();
    let damaged = project.narrative_arc().unwrap();
    assert_eq!(damaged.scenes.len(), 1);
    assert_eq!(damaged.unreadable.len(), 1);
    assert_ne!(damaged.revision, arc.revision);
    assert_eq!(damaged.scenes[0].arc.as_ref().unwrap().exits[0].to, Some(b.id.to_string()));
}
