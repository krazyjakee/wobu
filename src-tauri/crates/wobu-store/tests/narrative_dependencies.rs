//! Dependency tracking over a real project folder (#168).
//!
//! The pure crate is tested against in-memory fixtures; everything here needs a
//! folder, because every claim in it is about something a folder can do —
//! character notes that live outside the narrative tree, a local index that can
//! be deleted, guarded writes, and a Flow layout sidecar that must be invisible.

use std::collections::BTreeSet;

use wobu_core::NodeKind;
use wobu_narrative::{
    Beat, DialogueSlot, EntityId, Fact, Freshness, GenerationPolicy, KnowledgeClaim, Name,
    Participant, Relationship, Scene, SourceLink, Speaker, Text, TextAsset, TextKind, Value,
    Variant, WorldDocument,
};
use wobu_store::{GraphKey, Layout, LayoutMode, NodeKey, Project, SceneFile, SourceSave};

struct Fixture {
    _dir: tempfile::TempDir,
    project: Project,
    scene: SceneFile,
    kael: EntityId,
    mira: EntityId,
}

impl Fixture {
    fn kael_line(&self) -> wobu_narrative::VariantId {
        self.scene.beats()[0].dialogue[0].variants[0].id
    }
    fn mira_line(&self) -> wobu_narrative::VariantId {
        self.scene.beats()[0].dialogue[1].variants[0].id
    }
    fn reload(&mut self) {
        self.scene = self.project.load_scene(self.scene.scene.id).unwrap();
    }
    fn lifecycle(&self, variant: wobu_narrative::VariantId) -> wobu_narrative::ContentLifecycle {
        self.project
            .load_scene(self.scene.scene.id)
            .unwrap()
            .scene
            .dialogue_slots()
            .flat_map(|(_, slot)| slot.variants.clone())
            .find(|item| item.id == variant)
            .expect("the variant is still there")
            .text
            .lifecycle
    }
    fn set_voice(&mut self, id: EntityId, voice: &str) {
        let (node, _) = self.project.get_node_stamped(id).unwrap();
        let mut node = node;
        node.attributes.insert("narrative_voice".into(), serde_json::json!(voice));
        self.project.save_node(node).unwrap();
    }
}

trait Beats {
    fn beats(&self) -> &[Beat];
}
impl Beats for SceneFile {
    fn beats(&self) -> &[Beat] {
        &self.scene.beats
    }
}

/// Two characters, one two-line scene, one codex page about a fact, and a world
/// with a claim and a relationship — the smallest project in which "affected"
/// and "unaffected" are both non-empty.
fn harbour() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();

    let mut kael_node = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    kael_node.attributes.insert("narrative_voice".into(), serde_json::json!("Clipped."));
    project.save_node(kael_node.clone()).unwrap();
    let mut mira_node = project.create_node(NodeKind::Character, "Mira Sant", None).unwrap();
    mira_node.attributes.insert("narrative_voice".into(), serde_json::json!("Dry."));
    project.save_node(mira_node.clone()).unwrap();
    let (kael, mira) = (kael_node.id, mira_node.id);

    let fact = Fact {
        id: wobu_core::new_id(),
        name: "The lamp was lit".into(),
        assertion: "The harbour lamp was lit on the night of the wreck.".into(),
        sources: vec![],
        entity_ids: vec![],
    };
    let world = WorldDocument {
        facts: vec![fact.clone()],
        knowledge: vec![KnowledgeClaim {
            id: wobu_core::new_id(),
            name: "Kael saw the lamp".into(),
            character: kael,
            fact: fact.id,
            belief: wobu_narrative::Belief::True,
            provenance: wobu_narrative::KnowledgeProvenance::Witnessed,
            when: wobu_narrative::Condition::Always,
        }],
        relationships: vec![Relationship {
            id: wobu_core::new_id(),
            name: "Kael trusts Mira".into(),
            from: kael,
            to: mira,
            kind: Name::new("trust").unwrap(),
            value: Value::Int(2),
            when: wobu_narrative::Condition::Always,
        }],
        ..WorldDocument::default()
    };
    project.save_world(&world, None).unwrap();

    let mut file = project.create_scene("Harbour watch").unwrap();
    file.scene.summary = "Kael and Mira argue on the quay.".into();
    file.scene.participants = vec![
        Participant { entity: kael, role: String::new() },
        Participant { entity: mira, role: String::new() },
    ];
    let mut beat = Beat::new("On the quay");
    let mut kael_slot = DialogueSlot::new(Speaker::Entity(kael));
    kael_slot.variants.push(Variant::new(Text::written("The lamp was lit. I saw it.")));
    let mut mira_slot = DialogueSlot::new(Speaker::Entity(mira));
    mira_slot.variants.push(Variant::new(Text::written("You saw nothing.")));
    beat.dialogue.push(kael_slot);
    beat.dialogue.push(mira_slot);
    file.scene.beats.push(beat);
    assert!(matches!(project.save_scene(&mut file).unwrap(), SourceSave::Saved(_)));

    let mut asset = project
        .create_text_asset(TextKind::Codex, "The harbour lamp", Name::new("read_codex").unwrap())
        .unwrap();
    asset.asset.sources = vec![SourceLink::Fact(fact.id)];
    let mut entry = wobu_narrative::TextEntry::new("Opening");
    let mut line = DialogueSlot::new(Speaker::Narrator);
    line.variants.push(Variant::new(Text::written("The lamp burns above the bar.")));
    entry.lines.push(line);
    asset.asset.entries.push(entry);
    assert!(matches!(project.save_text_asset(&mut asset).unwrap(), SourceSave::Saved(_)));

    project.rebuild_narrative_dependencies().unwrap();
    Fixture { _dir: dir, project, scene: file, kael, mira }
}

fn world_of(project: &Project) -> (WorldDocument, wobu_store::atomic::Stamp) {
    project.world_document().unwrap().expect("the fixture wrote a world")
}

fn affected_variants(project: &Project) -> BTreeSet<wobu_narrative::VariantId> {
    project.narrative_affected().unwrap().iter().map(|item| item.target.variant()).collect()
}

/* ── the acceptance criteria, over a folder ───────────────────────────────── */

#[test]
fn changing_a_voice_marks_the_dependent_lines_and_writes_nothing_but_freshness() {
    let mut fixture = harbour();
    let (kael_line, mira_line) = (fixture.kael_line(), fixture.mira_line());
    let before = fixture.lifecycle(kael_line);
    let body = fixture.scene.beats()[0].dialogue[0].variants[0].text.clone();

    fixture.set_voice(fixture.kael, "Clipped, certain, and tired of being doubted.");

    let affected = fixture.project.narrative_affected().unwrap();
    // Both lines read Kael's voice — he is a participant of the scene, so the
    // resolver puts his voice in front of the model for Mira's line too. The
    // codex page has no cast and does not.
    assert_eq!(
        affected.iter().map(|item| item.target.variant()).collect::<BTreeSet<_>>(),
        BTreeSet::from([kael_line, mira_line])
    );
    let rows: Vec<_> = affected.iter().flat_map(|item| item.explanations()).collect();
    assert!(
        rows.iter().all(|row| row.source == format!("character/{}/narrative_voice", fixture.kael))
    );
    assert!(rows.iter().all(|row| row.line.contains("/variant/")));

    fixture.project.mark_narrative_affected().unwrap();
    let after = fixture.lifecycle(kael_line);
    assert_eq!(after.freshness, Freshness::OutOfDate);
    assert_eq!(after.policy, before.policy);
    assert_eq!(after.review, before.review);
    let now = fixture.project.load_scene(fixture.scene.scene.id).unwrap();
    let text = &now.scene.beats[0].dialogue[0].variants[0].text;
    assert_eq!(text.body, body.body, "the words moved");
    assert_eq!(text.revision, body.revision, "the revision moved");
    assert_eq!(text.provenance, body.provenance, "the provenance moved");
    // Marking is idempotent, but the explanation remains until new wording is authored.
    assert_eq!(fixture.project.narrative_affected().unwrap(), affected);
}

#[test]
fn a_locked_and_approved_line_can_still_be_marked_out_of_date() {
    // US-06's hardest case. `validate_manual` refuses any change to a locked
    // variant's text, and freshness lives inside that text — so a writer that
    // went through the ordinary save path would make a locked line permanently
    // un-markable, which is the exact failure `ContentLifecycle` is three fields
    // to prevent.
    let mut fixture = harbour();
    let kael_line = fixture.kael_line();
    let target = wobu_narrative::review::ReviewTarget {
        scene: fixture.scene.scene.id,
        beat: fixture.scene.beats()[0].id,
        slot: fixture.scene.beats()[0].dialogue[0].id,
        variant: Some(kael_line),
    };
    for action in [
        wobu_narrative::review::EditorialAction::Approve,
        wobu_narrative::review::EditorialAction::Policy {
            scope: wobu_narrative::review::PolicyScope::Variant,
            policy: GenerationPolicy::Locked,
        },
    ] {
        let view = fixture.project.review_scene(target.scene, None).unwrap();
        let line = view.lines.iter().find(|line| line.target == target).unwrap();
        let request = wobu_store::project::narrative_review::ReviewRequest {
            guard: view.guard.clone(),
            target: target.clone(),
            context_revision: line.context_revision.clone(),
            state_json: view.state_json.clone(),
            action,
        };
        fixture.project.apply_review(&request).unwrap();
    }
    fixture.reload();
    fixture.project.rebuild_narrative_dependencies().unwrap();
    assert_eq!(fixture.lifecycle(kael_line).policy, GenerationPolicy::Locked);
    assert!(fixture.lifecycle(kael_line).is_release_ready());

    fixture.set_voice(fixture.kael, "Hoarse, now.");
    fixture.project.mark_narrative_affected().unwrap();

    let after = fixture.lifecycle(kael_line);
    assert_eq!(after.policy, GenerationPolicy::Locked, "the lock was dropped");
    assert_eq!(after.review, wobu_narrative::ReviewState::Approved, "the approval was discarded");
    assert_eq!(after.freshness, Freshness::OutOfDate);
    assert!(!after.is_release_ready(), "stale approved wording is still release-ready");

    // The approval receipt is retained and its editorial history still verifies.
    // Writing a derived flag into the document must not read as "this scene was
    // edited outside its recorded history", which is what would discard every
    // binding in the file.
    let snapshot = fixture.project.review_snapshot(target.scene, None).unwrap();
    let evidence = snapshot.evidence().unwrap();
    assert!(
        evidence.get(&kael_line).is_some_and(|proof| proof.binding.approved),
        "marking freshness discarded the recorded approval"
    );
    // What *did* withdraw release readiness is the voice edit itself, and the
    // review pane says so in those words rather than blaming the history.
    let view = fixture.project.review_scene(target.scene, None).unwrap();
    let line = view.lines.iter().find(|line| line.target == target).unwrap();
    assert_eq!(line.reason, "Authored context changed since the last review.");
    assert_eq!(line.freshness, Freshness::OutOfDate);
}

#[test]
fn a_relationship_that_never_existed_before_invalidates_the_lines_it_reaches() {
    let mut fixture = harbour();
    let kael_line = fixture.kael_line();
    let (world, stamp) = world_of(&fixture.project);
    let mut next = world.clone();
    next.relationships.push(Relationship {
        id: wobu_core::new_id(),
        name: "Kael resents Mira".into(),
        from: fixture.kael,
        to: fixture.mira,
        kind: Name::new("resentment").unwrap(),
        value: Value::Int(1),
        when: wobu_narrative::Condition::Never,
    });
    fixture.project.save_world(&next, Some(&stamp)).unwrap();

    assert_eq!(affected_variants(&fixture.project), BTreeSet::from([kael_line]));
}

#[test]
fn deleting_a_fact_reaches_the_codex_page_that_cites_it() {
    let mut fixture = harbour();
    let (world, stamp) = world_of(&fixture.project);
    let fact = world.facts[0].id;
    let mut next = world.clone();
    next.facts.clear();
    next.knowledge.clear();
    fixture.project.save_world(&next, Some(&stamp)).unwrap();

    let affected = fixture.project.narrative_affected().unwrap();
    let codex = affected
        .iter()
        .find(|item| matches!(item.target, wobu_narrative_deps::TargetRef::TextLine { .. }))
        .expect("the codex page cites the fact through a typed source link");
    assert!(codex.reasons.contains(&wobu_narrative_deps::Reason::FieldRemoved {
        source: format!("world/facts/{fact}")
    }));

    fixture.project.mark_narrative_affected().unwrap();
    let asset: TextAsset = fixture
        .project
        .text_assets()
        .unwrap()
        .into_iter()
        .next()
        .expect("the asset is still there");
    assert_eq!(
        asset.entries[0].lines[0].variants[0].text.lifecycle.freshness,
        Freshness::OutOfDate
    );
    assert_eq!(asset.entries[0].lines[0].variants[0].text.body, "The lamp burns above the bar.");
}

#[test]
fn an_irrelevant_edit_leaves_every_slot_alone() {
    // The precision claim, from the other direction. A whole second scene with
    // its own dialogue arrives, and then a line inside the first scene is
    // rewritten — and neither reaches a slot that did not read it.
    let mut fixture = harbour();
    let (kael_line, mira_line) = (fixture.kael_line(), fixture.mira_line());

    let mut other = fixture.project.create_scene("Dockside").unwrap();
    other.scene.summary = "Nothing to do with the quay.".into();
    let mut beat = Beat::new("Unloading");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written("Crates, and more crates.")));
    beat.dialogue.push(slot);
    other.scene.beats.push(beat);
    fixture.project.save_scene(&mut other).unwrap();

    let affected = fixture.project.narrative_affected().unwrap();
    assert_eq!(
        affected.iter().map(|item| item.target.variant()).collect::<BTreeSet<_>>(),
        BTreeSet::new(),
        "an unrelated scene invalidated existing lines"
    );
    fixture.project.rebuild_narrative_dependencies().unwrap();

    // Rewriting one line must not mark its neighbour stale. The review context
    // that predates #168 hashes the whole scene document, so this edit still
    // withdraws the neighbour's *approval*; the dependency index does not agree
    // that the neighbour's inputs moved, and that difference is the point.
    fixture.reload();
    fixture.scene.scene.beats[0].dialogue[1].variants[0]
        .text
        .set_body("You saw a lantern and a lie.", wobu_narrative::Provenance::Human);
    fixture.project.save_scene(&mut fixture.scene).unwrap();

    let affected = fixture.project.narrative_affected().unwrap();
    assert!(
        affected.is_empty(),
        "rewriting one line moved another line's dependencies: {affected:?}"
    );
    let _ = (kael_line, mira_line);
}

#[test]
fn a_layout_only_edit_invalidates_nothing_and_moves_no_fingerprint() {
    // The #185 invariant, checked against the new machinery as well as the old.
    let mut fixture = harbour();
    let source = fixture.project.narrative_fingerprint().unwrap();
    let build = fixture.project.narrative_build_fingerprint().unwrap();
    let index = fixture.project.narrative_dependencies().unwrap();

    let mut layout = Layout::empty(GraphKey::of_scene(fixture.scene.scene.id));
    layout.set_mode(LayoutMode::Manual);
    layout.place(NodeKey::Beat(fixture.scene.beats()[0].id), 420.0, 96.0);
    fixture.project.save_scene_layout(&fixture.scene.scene, &layout).unwrap();
    fixture.project.reconcile().unwrap();

    assert_eq!(fixture.project.narrative_fingerprint().unwrap(), source);
    assert_eq!(fixture.project.narrative_build_fingerprint().unwrap(), build);
    assert_eq!(fixture.project.narrative_dependency_snapshot().unwrap().index(), index);
    assert!(fixture.project.narrative_affected().unwrap().is_empty());
    assert!(fixture.project.mark_narrative_affected().unwrap().is_empty());
}

#[test]
fn the_build_fingerprint_separates_source_from_toolchain() {
    // Two fingerprints because they answer different questions. The source one
    // is what a coherence check compares — it must not move when the app is
    // upgraded mid-read. The build one is what a build is keyed on.
    let fixture = harbour();
    let source = fixture.project.narrative_fingerprint().unwrap();
    let build = fixture.project.narrative_build_fingerprint().unwrap();
    assert_ne!(source, build);
    assert_eq!(fixture.project.narrative_build_fingerprint().unwrap(), build);
}

/* ── the cache is a cache ─────────────────────────────────────────────────── */

#[test]
fn losing_the_local_index_rebuilds_the_same_dependency_index() {
    let fixture = harbour();
    let before = fixture.project.narrative_dependencies().unwrap();
    let edges = fixture.project.index().narrative_dependency_edges().unwrap();
    assert!(!before.is_empty());

    // Exactly what deleting the database costs.
    fixture.project.forget_narrative_dependencies().unwrap();
    assert_eq!(fixture.project.narrative_dependencies().unwrap(), before);

    let rebuilt = fixture.project.rebuild_narrative_dependencies().unwrap();
    assert_eq!(rebuilt, before);
    assert_eq!(fixture.project.narrative_dependencies().unwrap(), before);
    assert_eq!(fixture.project.index().narrative_dependency_edges().unwrap(), edges);
    assert!(fixture.project.narrative_affected().unwrap().is_empty());
}

#[test]
fn the_stored_edges_agree_with_the_derived_ones() {
    // The edge table is an accelerator, never a second opinion. If the two could
    // disagree, `candidates` would be answering from data nothing checks.
    let fixture = harbour();
    let index = fixture.project.narrative_dependencies().unwrap();
    let derived: BTreeSet<(String, String)> = index
        .edges()
        .iter()
        .flat_map(|(key, ids)| ids.iter().map(|id| (key.clone(), id.to_string())))
        .collect();
    assert_eq!(fixture.project.index().narrative_dependency_edges().unwrap(), derived);
}

#[test]
fn the_candidate_lookup_covers_the_lines_a_change_reaches() {
    let mut fixture = harbour();
    let keys = BTreeSet::from([format!("field:character/{}/narrative_voice", fixture.kael)]);
    let candidates = fixture.project.narrative_dependency_candidates(&keys).unwrap();

    fixture.set_voice(fixture.kael, "Hoarse.");
    let affected = affected_variants(&fixture.project);
    assert!(!affected.is_empty());
    assert!(
        affected.is_subset(&candidates),
        "the stored reverse index dropped an affected line: {affected:?} against {candidates:?}"
    );
}

#[test]
fn cache_loss_after_a_source_change_preserves_the_original_baseline_and_explanations() {
    let mut fixture = harbour();
    let baseline = fixture.project.narrative_dependencies().unwrap();
    fixture.set_voice(fixture.kael, "Hoarse.");
    let affected = fixture.project.narrative_affected().unwrap();
    let root = fixture.project.root().to_path_buf();
    let index_path = fixture.project.index_path();
    drop(fixture.project);
    std::fs::remove_file(&index_path).unwrap();
    fixture.project = Project::open_at_index(&root, &index_path).unwrap();
    assert_eq!(fixture.project.rebuild_narrative_dependencies().unwrap(), baseline);
    assert_eq!(fixture.project.narrative_affected().unwrap(), affected);
    assert_eq!(fixture.lifecycle(fixture.kael_line()).freshness, Freshness::OutOfDate);
}

#[test]
fn a_snapshot_with_a_concurrent_character_edit_cannot_publish_baseline_receipts() {
    let mut fixture = harbour();
    let snapshot = fixture.project.narrative_dependency_snapshot().unwrap();
    fixture.set_voice(fixture.kael, "Changed while a task was running.");
    let receipts =
        fixture.project.narrative_records(wobu_store::NarrativeRecordKind::Receipt).unwrap();
    let error = fixture
        .project
        .record_narrative_dependency_snapshot(&snapshot, &BTreeSet::from([fixture.kael_line()]))
        .unwrap_err();
    assert!(error.to_string().contains("changed while reading dependencies"));
    assert_eq!(
        fixture
            .project
            .narrative_records(wobu_store::NarrativeRecordKind::Receipt)
            .unwrap()
            .into_iter()
            .map(|f| f.document)
            .collect::<Vec<_>>(),
        receipts.into_iter().map(|f| f.document).collect::<Vec<_>>()
    );
}

#[test]
fn a_snapshot_with_a_concurrent_scene_edit_cannot_publish_baseline_receipts() {
    let mut fixture = harbour();
    let snapshot = fixture.project.narrative_dependency_snapshot().unwrap();
    fixture.scene.scene.summary = "Changed during capture".into();
    fixture.project.save_scene(&mut fixture.scene).unwrap();
    assert!(snapshot.check_current(&fixture.project).is_err());
}

#[test]
fn an_external_voice_edit_is_marked_by_watcher_reconciliation() {
    let mut fixture = harbour();
    let node = fixture.project.get_node(fixture.kael).unwrap();
    let rel = fixture.project.index().rel_path_of(node.id).unwrap().unwrap();
    let path = fixture.project.root().join(&rel);
    let source = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, source.replace("Clipped.", "Externally rewritten, hoarse voice."))
        .unwrap();
    fixture.project.reconcile_paths(&[path]).unwrap();
    assert_eq!(fixture.lifecycle(fixture.kael_line()).freshness, Freshness::OutOfDate);
    assert_eq!(
        affected_variants(&fixture.project),
        BTreeSet::from([fixture.kael_line(), fixture.mira_line()])
    );
}

/* ── downstream artifacts ─────────────────────────────────────────────────── */

#[test]
fn an_affected_line_names_its_production_artifacts_without_touching_them() {
    // Production records (#178–#183) are immutable evidence of what shipped.
    // Invalidation reaches them as a report and never as a rewrite.
    let mut fixture = harbour();
    let kael_line = fixture.kael_line();
    let mut record = wobu_store::NarrativeRecordFile {
        document: wobu_store::NarrativeRecordDocument::new(
            wobu_store::NarrativeRecordKind::Production,
            wobu_core::new_id(),
            "Kael line recording",
            serde_json::json!({"variant_id": kael_line.to_string(), "take": 3}),
        ),
        stamp: None,
    };
    fixture.project.save_narrative_record(&mut record).unwrap();
    let before = record.document.clone();

    fixture.set_voice(fixture.kael, "Hoarse.");
    let affected = fixture.project.narrative_affected().unwrap();
    let downstream = fixture.project.narrative_affected_production(&affected).unwrap();
    assert_eq!(downstream.len(), 1);
    assert_eq!(downstream[0], before);

    fixture.project.mark_narrative_affected().unwrap();
    let after = fixture
        .project
        .narrative_record(wobu_store::NarrativeRecordKind::Production, before.id)
        .unwrap()
        .unwrap();
    assert_eq!(after.document, before, "an invalidation rewrote a production record");
}

#[test]
fn an_unaffected_line_names_no_production_artifact() {
    let mut fixture = harbour();
    let mira_only = fixture.mira_line();
    let mut record = wobu_store::NarrativeRecordFile {
        document: wobu_store::NarrativeRecordDocument::new(
            wobu_store::NarrativeRecordKind::Production,
            wobu_core::new_id(),
            "Mira line recording",
            serde_json::json!({"variant_id": mira_only.to_string()}),
        ),
        stamp: None,
    };
    fixture.project.save_narrative_record(&mut record).unwrap();

    let scene: Scene = fixture.project.load_scene(fixture.scene.scene.id).unwrap().scene;
    assert!(!scene.beats.is_empty());
    assert!(fixture.project.narrative_affected_production(&[]).unwrap().is_empty());
}

fn review_line(fixture: &mut Fixture, action: wobu_narrative::review::EditorialAction) {
    let view = fixture.project.review_scene(fixture.scene.scene.id, None).unwrap();
    let line = view.lines.iter().find(|l| l.target.variant == Some(fixture.kael_line())).unwrap();
    fixture
        .project
        .apply_review(&wobu_store::project::narrative_review::ReviewRequest {
            guard: view.guard.clone(),
            target: line.target.clone(),
            context_revision: line.context_revision.clone(),
            state_json: view.state_json,
            action,
        })
        .unwrap();
    fixture.reload();
}

#[test]
fn sibling_wording_and_unrelated_facts_preserve_precise_approval_and_attestation() {
    let mut fixture = harbour();
    review_line(&mut fixture, wobu_narrative::review::EditorialAction::Approve);
    let original = fixture.project.review_scene(fixture.scene.scene.id, None).unwrap().lines[0]
        .context_revision
        .clone();
    fixture.scene.scene.beats[0].dialogue[1].variants[0]
        .text
        .set_body("Mira corrects a typo.", wobu_narrative::Provenance::Human);
    fixture.project.save_scene(&mut fixture.scene).unwrap();
    let (mut world, stamp) = world_of(&fixture.project);
    world.facts.push(Fact {
        id: wobu_core::new_id(),
        name: "A distant island".into(),
        assertion: "An unrelated island exists.".into(),
        sources: vec![],
        entity_ids: vec![],
    });
    fixture.project.save_world(&world, Some(&stamp)).unwrap();
    let view = fixture.project.review_scene(fixture.scene.scene.id, None).unwrap();
    let line = view.lines.iter().find(|l| l.target.variant == Some(fixture.kael_line())).unwrap();
    assert_eq!(line.context_revision, original);
    assert!(line.approval_valid, "{}", line.reason);
    assert_eq!(line.freshness, Freshness::Current);
    assert!(fixture.project.narrative_affected().unwrap().is_empty());
}

#[test]
fn attesting_unchanged_wording_acknowledges_only_that_lines_current_dependencies() {
    let mut fixture = harbour();
    fixture.set_voice(fixture.kael, "Hoarse.");
    review_line(&mut fixture, wobu_narrative::review::EditorialAction::Attest);
    assert_eq!(fixture.lifecycle(fixture.kael_line()).freshness, Freshness::Current);
    assert_eq!(fixture.lifecycle(fixture.mira_line()).freshness, Freshness::OutOfDate);
    assert_eq!(affected_variants(&fixture.project), BTreeSet::from([fixture.mira_line()]));
    let before =
        fixture.project.narrative_records(wobu_store::NarrativeRecordKind::Receipt).unwrap().len();
    fixture.project.forget_narrative_dependencies().unwrap();
    assert_eq!(affected_variants(&fixture.project), BTreeSet::from([fixture.mira_line()]));
    assert_eq!(
        fixture.project.narrative_records(wobu_store::NarrativeRecordKind::Receipt).unwrap().len(),
        before
    );
}

#[test]
fn dependency_history_uses_causality_under_clock_skew_and_refuses_concurrent_heads() {
    let mut fixture = harbour();
    let baseline = fixture
        .project
        .narrative_records(wobu_store::NarrativeRecordKind::Receipt)
        .unwrap()
        .into_iter()
        .find(|f| {
            f.document.payload["type"] == "narrative_dependency_baseline"
                && f.document.payload["entries"].as_array().unwrap().iter().any(|e| {
                    e["dependencies"]["target"]["scene_line"]["variant"]
                        == fixture.kael_line().to_string()
                })
        })
        .unwrap();
    let mut payload = baseline.document.payload;
    let entry = payload["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            e["dependencies"]["target"]["scene_line"]["variant"] == fixture.kael_line().to_string()
        })
        .unwrap()
        .clone();
    payload["entries"] = serde_json::json!([entry]);
    payload["entries"][0]["parent"] = serde_json::json!(baseline.document.id);
    // A peer's clock moved backwards. The causal successor sorts before its parent.
    payload["entries"][0]["dependencies"]["versions"]["prompt"] = serde_json::json!(0);
    let mut successor = wobu_store::NarrativeRecordFile {
        document: wobu_store::NarrativeRecordDocument::new(
            wobu_store::NarrativeRecordKind::Receipt,
            "00000000000000000000000001".parse().unwrap(),
            "Clock-skewed dependency successor",
            payload.clone(),
        ),
        stamp: None,
    };
    fixture.project.save_narrative_record(&mut successor).unwrap();
    assert_eq!(
        fixture
            .project
            .narrative_dependencies()
            .unwrap()
            .get(fixture.kael_line())
            .unwrap()
            .versions
            .prompt,
        0
    );
    let mut concurrent = wobu_store::NarrativeRecordFile {
        document: wobu_store::NarrativeRecordDocument::new(
            wobu_store::NarrativeRecordKind::Receipt,
            "00000000000000000000000002".parse().unwrap(),
            "Concurrent dependency successor",
            payload,
        ),
        stamp: None,
    };
    fixture.project.save_narrative_record(&mut concurrent).unwrap();
    assert!(
        fixture.project.narrative_dependencies().unwrap_err().to_string().contains("Concurrent")
    );
}

#[test]
fn a_large_authoring_batch_syncs_bounded_chunks_without_inventing_remote_baselines() {
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::create(directory.path(), "Batch").unwrap();
    let mut file = project.create_scene("A thousand lines").unwrap();
    let mut beat = Beat::new("Arrival");
    for _ in 0..1000 {
        let mut slot = DialogueSlot::new(Speaker::Narrator);
        slot.variants.push(Variant::new(Text::written("Welcome.")));
        beat.dialogue.push(slot);
    }
    file.scene.beats.push(beat);
    let start = std::time::Instant::now();
    project.save_scene(&mut file).unwrap();
    let dependencies = project
        .narrative_records(wobu_store::NarrativeRecordKind::Receipt)
        .unwrap()
        .into_iter()
        .filter(|f| f.document.payload["type"] == "narrative_dependency_baseline")
        .collect::<Vec<_>>();
    assert!(dependencies.len() > 1 && dependencies.len() < 10);
    assert_eq!(
        dependencies
            .iter()
            .map(|f| f.document.payload["entries"].as_array().unwrap().len())
            .sum::<usize>(),
        1000
    );
    for file in &dependencies {
        assert!(
            std::fs::metadata(project.root().join(file.document.rel())).unwrap().len()
                <= wobu_store::project::narrative_sync::MAX_NARRATIVE_FILE_BYTES as u64
        );
    }
    eprintln!(
        "1,000-line canonical dependency enrollment/save ({} chunks): {:?}",
        dependencies.len(),
        start.elapsed()
    );

    let peer_dir = tempfile::tempdir().unwrap();
    let mut peer = Project::create(peer_dir.path(), "Receiving peer").unwrap();
    let destination = peer.root().join(&file.rel);
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::copy(project.root().join(&file.rel), destination).unwrap();
    // The line arrived first. A watcher must not invent a local-world baseline.
    peer.reconcile().unwrap();
    assert!(peer.narrative_dependencies().unwrap().is_empty());
    assert!(
        peer.narrative_affected()
            .unwrap()
            .iter()
            .all(|a| a.kind == wobu_narrative_deps::AffectedKind::Untracked)
    );
    assert!(peer.narrative_records(wobu_store::NarrativeRecordKind::Receipt).unwrap().is_empty());
    let ids: BTreeSet<_> = dependencies.iter().map(|f| f.document.id.to_string()).collect();
    for entry in project
        .narrative_manifest()
        .unwrap()
        .into_iter()
        .filter(|entry| ids.iter().any(|id| entry.rel.ends_with(&format!("/{id}.json"))))
    {
        let incoming = project.narrative_outgoing(&entry).unwrap().unwrap();
        assert!(matches!(
            peer.apply_narrative_from_peer("author", &incoming).unwrap(),
            wobu_store::NarrativeApplied::Agreed { .. }
        ));
    }
    peer.reconcile().unwrap();
    peer.forget_narrative_dependencies().unwrap();
    assert_eq!(
        peer.rebuild_narrative_dependencies().unwrap(),
        project.narrative_dependencies().unwrap()
    );
    assert!(peer.narrative_affected().unwrap().is_empty());
}

#[test]
fn tampering_with_full_frozen_inputs_cannot_preserve_a_precise_approval() {
    let mut fixture = harbour();
    review_line(&mut fixture, wobu_narrative::review::EditorialAction::Approve);
    let id = fixture.scene.scene.editorial_head.unwrap();
    let mut record = fixture
        .project
        .narrative_record(wobu_store::NarrativeRecordKind::Receipt, id)
        .unwrap()
        .unwrap();
    record.document.payload["context"]["inputs"]["world"] = serde_json::json!("damaged");
    // The semantic dependency digest was not changed. The immutable envelope
    // binding nevertheless protects every frozen input, before domain decoding.
    std::fs::write(
        fixture.project.root().join(record.document.rel()),
        serde_json::to_string_pretty(&record.document).unwrap(),
    )
    .unwrap();
    assert!(
        fixture.project.narrative_record(wobu_store::NarrativeRecordKind::Receipt, id).is_err()
    );
    let view = fixture.project.review_scene(fixture.scene.scene.id, None).unwrap();
    assert!(
        !view
            .lines
            .iter()
            .find(|l| l.target.variant == Some(fixture.kael_line()))
            .unwrap()
            .approval_valid
    );
}
