//! Flow canvas layout as presentation metadata (#185).
//!
//! Every test here is one of the issue's acceptance criteria, driven over a
//! real project folder. The first two are the important ones: they are the
//! proof that moving a box cannot change the story, and everything else in
//! this file is about what happens when the cosmetic file is wrong.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use wobu_narrative::{
    Beat, Choice, Destination, DialogueSlot, Outcome, Scene, SceneId, Speaker, Text, Variant,
};
use wobu_store::{
    GraphKey, Group, GroupId, Layout, LayoutMode, LayoutNotice, LayoutSave, NodeKey, Project,
    SceneFile, SourceSave,
};

/// A scene with two beats, a choice and an outcome — four canvas nodes plus
/// the scene itself.
fn kiln_scene() -> Scene {
    let mut scene = Scene::new("Kiln Interrogation");
    let mut opening = Beat::new("Opening");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written("The kiln is still warm.")));
    opening.dialogue.push(slot);

    let mut verdict = Beat::new("Verdict");
    verdict.outcomes.push(Outcome::new(Destination::End { label: "silence".into() }));

    opening.choices.push(Choice::new("Press them", Destination::Beat(verdict.id)));
    scene.beats.push(opening);
    scene.beats.push(verdict);
    scene
}

fn arranged() -> (tempfile::TempDir, Project, SceneFile) {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let mut file = project.create_scene("Kiln Interrogation").unwrap();
    let scene = kiln_scene();
    file.scene = Scene { id: file.scene.id, ..scene };
    project.save_scene(&mut file).unwrap();
    (dir, project, file)
}

fn layout_path(project: &Project, scene: SceneId) -> PathBuf {
    wobu_store::narrative::layout::path_of(project.root(), &GraphKey::of_scene(scene)).unwrap()
}

/// Everything about the source that a layout edit must not move.
#[derive(Debug, PartialEq)]
struct SourceWitness {
    bytes: Vec<u8>,
    fingerprint: String,
    revisions: Vec<String>,
    mtime: std::time::SystemTime,
}

fn witness(project: &Project, file: &SceneFile) -> SourceWitness {
    let path = project.root().join(&file.rel);
    let scene = project.load_scene(file.scene.id).unwrap().scene;
    SourceWitness {
        bytes: fs::read(&path).unwrap(),
        fingerprint: project.narrative_fingerprint().unwrap(),
        revisions: scene
            .dialogue_slots()
            .flat_map(|(_, slot)| slot.variants.iter())
            .map(|variant| variant.text.revision.to_string())
            .collect(),
        mtime: fs::metadata(&path).unwrap().modified().unwrap(),
    }
}

fn drag_everything(project: &Project, scene: &Scene) -> Layout {
    let mut layout = project.scene_layout(scene).layout;
    layout.set_mode(LayoutMode::Manual);
    for (index, key) in wobu_store::narrative::layout::scene_keys(scene).into_iter().enumerate() {
        layout.place(key, index as f64 * 240.0, index as f64 * 96.0);
    }
    layout
}

/* ── the proof ────────────────────────────────────────────────────────────── */

#[test]
fn a_layout_only_change_leaves_the_source_byte_identical() {
    // The claim #185 exists to make, checked rather than asserted: the file's
    // bytes, its mtime, the revision every translation and recording is keyed
    // to, and the fingerprint a build would depend on are all unmoved by an
    // afternoon of dragging.
    let (_dir, project, file) = arranged();
    let before = witness(&project, &file);

    for round in 0..5 {
        let mut layout = drag_everything(&project, &file.scene);
        layout.place(NodeKey::Beat(file.scene.beats[0].id), round as f64 * 17.0, 4.0);
        layout.set_collapsed(NodeKey::Beat(file.scene.beats[1].id), round % 2 == 0);
        layout.upsert_group(Group {
            id: GroupId::new(),
            label: "Act one".into(),
            collapsed: false,
            members: vec![NodeKey::Beat(file.scene.beats[0].id)],
            updated_at: Utc::now(),
        });
        assert!(matches!(
            project.save_scene_layout(&file.scene, &layout).unwrap(),
            LayoutSave::Written(_)
        ));
    }

    assert_eq!(witness(&project, &file), before, "a layout-only edit moved the source");
    // And the arrangement really was written, so the equality above is not the
    // trivial one.
    assert!(layout_path(&project, file.scene.id).is_file());
    assert!(!project.scene_layout(&file.scene).layout.nodes.is_empty());
}

#[test]
fn no_coordinate_can_reach_the_source_document() {
    // Two halves. The source file does not contain the words a position would
    // be written with, and `wobu-narrative` refuses one if anybody tries.
    let (_dir, project, file) = arranged();
    let layout = drag_everything(&project, &file.scene);
    project.save_scene_layout(&file.scene, &layout).unwrap();

    let yaml = fs::read_to_string(project.root().join(&file.rel)).unwrap();
    let keys: Vec<String> = yaml
        .lines()
        .filter_map(|line| line.trim_start().trim_start_matches("- ").split_once(':'))
        .map(|(key, _)| key.trim().to_string())
        .collect();
    for forbidden in ["x", "y", "width", "height", "collapsed", "group", "layout", "annotations"] {
        assert!(!keys.iter().any(|key| key == forbidden), "source names `{forbidden}`:\n{yaml}");
    }

    let smuggled = yaml.replace("beats:", "x: 240\nbeats:");
    assert!(
        wobu_narrative::SceneDocument::parse(&smuggled).is_err(),
        "the source model accepted a coordinate"
    );
}

#[test]
fn source_and_layout_live_in_directories_that_cannot_be_confused() {
    // Every rule that has to exclude layout — a compiler input set, a build
    // fingerprint, an export — is a directory prefix rather than a filename
    // pattern, because a prefix cannot be got subtly wrong.
    let (_dir, project, file) = arranged();
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();

    let mut source = Vec::new();
    let mut presentation = Vec::new();
    for entry in walkdir::WalkDir::new(project.root().join("narrative")).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(project.root()).unwrap().display().to_string();
        if wobu_store::narrative::layout::is_layout_path(&rel.replace('\\', "/")) {
            presentation.push(rel);
        } else {
            source.push(rel);
        }
    }
    assert_eq!(source, vec![file.rel.clone()]);
    assert_eq!(presentation.len(), 1);
    assert!(presentation[0].ends_with(".json"), "layout is JSON, source is YAML");
}

/* ── graceful degradation ─────────────────────────────────────────────────── */

#[test]
fn a_scene_with_no_layout_file_opens_automatically() {
    let (_dir, project, file) = arranged();
    let loaded = project.scene_layout(&file.scene);
    assert!(loaded.layout.nodes.is_empty());
    assert_eq!(loaded.layout.mode, LayoutMode::Automatic);
    assert!(loaded.notices.iter().any(|n| matches!(n, LayoutNotice::Missing { .. })));
    assert!(loaded.notices.iter().all(|n| !n.is_blocking()));
    // Every node the scene has is reported as needing an automatic position.
    let unplaced = unplaced(&loaded.notices);
    assert_eq!(unplaced.len(), wobu_store::narrative::layout::scene_keys(&file.scene).len());
}

#[test]
fn a_deleted_layout_file_opens_the_scene_and_can_be_written_again() {
    let (_dir, project, file) = arranged();
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();
    fs::remove_file(layout_path(&project, file.scene.id)).unwrap();

    let loaded = project.scene_layout(&file.scene);
    assert!(loaded.notices.iter().any(|n| matches!(n, LayoutNotice::Missing { .. })));
    assert!(loaded.layout.nodes.is_empty());
    // The scene itself is untouched, and a fresh arrangement lands normally.
    assert_eq!(project.load_scene(file.scene.id).unwrap().scene, file.scene);
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();
    assert!(!project.scene_layout(&file.scene).layout.nodes.is_empty());
}

#[test]
fn a_corrupt_layout_file_opens_the_scene_and_its_bytes_are_kept() {
    let (_dir, project, file) = arranged();
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();
    let path = layout_path(&project, file.scene.id);
    fs::write(&path, "{\"schemaVersion\": 1, \"nodes\": {\"beat:").unwrap();

    let loaded = project.scene_layout(&file.scene);
    assert!(loaded.notices.iter().any(|n| matches!(n, LayoutNotice::Unreadable { .. })));
    assert!(loaded.notices.iter().all(|n| !n.is_blocking()));
    assert_eq!(project.load_scene(file.scene.id).unwrap().scene, file.scene, "the scene opened");

    // Saving over it parks the broken bytes rather than deleting them, and
    // then writes a good file. A cosmetic sidecar must not wedge the canvas,
    // and it must not eat evidence either.
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();
    assert!(!project.scene_layout(&file.scene).layout.nodes.is_empty());
    let parked: Vec<PathBuf> = siblings(&path, "corrupt");
    assert_eq!(parked.len(), 1, "the broken bytes should have been parked");
    assert!(fs::read_to_string(&parked[0]).unwrap().contains("\"nodes\""));
}

#[test]
fn a_layout_written_by_a_newer_wobu_is_read_as_absent_and_never_downgraded() {
    let (_dir, project, file) = arranged();
    let path = layout_path(&project, file.scene.id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let newer = format!(
        "{{\"schemaVersion\": 99, \"graph\": {{\"kind\": \"scene\", \"scene\": \"{}\"}}, \
         \"somethingNew\": true}}\n",
        file.scene.id
    );
    fs::write(&path, &newer).unwrap();

    let loaded = project.scene_layout(&file.scene);
    assert!(loaded.notices.iter().any(|n| matches!(n, LayoutNotice::NewerSchema { .. })));
    assert!(loaded.layout.nodes.is_empty(), "a shape we do not know is not adopted");

    let outcome =
        project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();
    assert!(matches!(outcome, LayoutSave::Deferred { found: 99, .. }), "got {outcome:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), newer, "we must not overwrite a newer file");
}

#[test]
fn a_layout_file_describing_another_graph_is_ignored_rather_than_adopted() {
    // What copying a folder and hand-renaming a file inside it looks like.
    let (_dir, project, file) = arranged();
    let mine = layout_path(&project, file.scene.id);
    let mut theirs = Layout::empty(GraphKey::of_scene(SceneId::new()));
    theirs.place(NodeKey::Beat(file.scene.beats[0].id), 999.0, 999.0);
    fs::create_dir_all(mine.parent().unwrap()).unwrap();
    fs::write(&mine, serde_json::to_string_pretty(&theirs).unwrap()).unwrap();

    let loaded = project.scene_layout(&file.scene);
    assert!(loaded.notices.iter().any(|n| matches!(n, LayoutNotice::WrongGraph { .. })));
    assert!(loaded.layout.nodes.is_empty());
}

#[test]
fn a_new_beat_with_no_saved_position_is_reported_rather_than_placed_at_zero() {
    let (_dir, project, mut file) = arranged();
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();

    file.scene.beats.push(Beat::new("Aftermath"));
    let fresh = file.scene.beats.last().unwrap().id;
    let loaded = project.scene_layout(&file.scene);
    assert_eq!(unplaced(&loaded.notices), vec![NodeKey::Beat(fresh)]);
    assert!(!loaded.layout.nodes.contains_key(&NodeKey::Beat(fresh)));
}

/* ── ids, not names ───────────────────────────────────────────────────────── */

#[test]
fn renaming_and_reordering_keeps_every_position() {
    let (_dir, mut project, mut file) = arranged();
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();
    let before = project.scene_layout(&file.scene).layout.nodes.clone();

    file.scene.name = "The Long Question".into();
    file.scene.beats[0].title = "A Warm Room".into();
    let moved = file.scene.beats[1].id;
    assert!(file.scene.reorder_beat(moved, 0));
    project.save_scene(&mut file).unwrap();

    let after = project.scene_layout(&file.scene);
    assert_eq!(after.layout.nodes, before, "renaming or reordering moved a box");
    assert!(!after.notices.iter().any(|n| matches!(n, LayoutNotice::Unplaced { .. })));
}

#[test]
fn duplicating_a_beat_allocates_a_fresh_position() {
    let (_dir, mut project, mut file) = arranged();
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();

    let original = file.scene.beats[0].id;
    let copy = file.scene.duplicate_beat(original).unwrap();
    project.save_scene(&mut file).unwrap();

    let loaded = project.scene_layout(&file.scene);
    assert!(loaded.layout.nodes.contains_key(&NodeKey::Beat(original)), "the original stayed put");
    assert!(unplaced(&loaded.notices).contains(&NodeKey::Beat(copy)), "the copy needs a position");
    // The copy's choices are new slots too, so they are unplaced as well.
    let copied = file.scene.beat(copy).unwrap();
    for choice in &copied.choices {
        assert!(unplaced(&loaded.notices).contains(&NodeKey::Choice(choice.id)));
    }
}

#[test]
fn deleting_a_beat_collects_its_position_without_error() {
    let (_dir, mut project, mut file) = arranged();
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();

    let doomed = file.scene.beats[1].id;
    file.scene.remove_beat(doomed, Some("cut".into())).unwrap();
    project.save_scene(&mut file).unwrap();

    let loaded = project.scene_layout(&file.scene);
    assert!(
        loaded.notices.iter().any(|n| matches!(n, LayoutNotice::Stale { nodes } if nodes
            .contains(&NodeKey::Beat(doomed)))),
        "the deleted beat should be reported as collected: {:?}",
        loaded.notices
    );
    assert!(!loaded.layout.nodes.contains_key(&NodeKey::Beat(doomed)));

    // And the collection is durable once something saves.
    project.save_scene_layout(&file.scene, &loaded.layout).unwrap();
    let text = fs::read_to_string(layout_path(&project, file.scene.id)).unwrap();
    assert!(!text.contains(&doomed.to_string()));
}

#[test]
fn an_annotation_pinned_to_a_deleted_beat_keeps_its_words() {
    let (_dir, mut project, mut file) = arranged();
    let mut layout = drag_everything(&project, &file.scene);
    let doomed = file.scene.beats[1].id;
    layout.upsert_annotation(wobu_store::Annotation {
        id: wobu_store::AnnotationId::new(),
        body: "Ask Nadia whether this lands".into(),
        x: 10.0,
        y: 10.0,
        width: None,
        height: None,
        attached_to: Some(NodeKey::Beat(doomed)),
        updated_at: Utc::now(),
    });
    project.save_scene_layout(&file.scene, &layout).unwrap();

    file.scene.remove_beat(doomed, None).unwrap();
    project.save_scene(&mut file).unwrap();

    let loaded = project.scene_layout(&file.scene);
    let notes: Vec<&wobu_store::Annotation> = loaded.layout.annotations.values().collect();
    assert_eq!(notes.len(), 1, "a note is the one thing in this file a person wrote");
    assert_eq!(notes[0].body, "Ask Nadia whether this lands");
    assert_eq!(notes[0].attached_to, None, "it lost its anchor, not its words");
}

/* ── two writers ──────────────────────────────────────────────────────────── */

#[test]
fn two_writers_dragging_different_boxes_both_keep_their_move() {
    // The reason this file merges at all. Nadia is another machine, which is
    // to say bytes changing under us between our read and our write.
    let (_dir, project, file) = arranged();
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();

    let hers = NodeKey::Beat(file.scene.beats[0].id);
    let mine = NodeKey::Beat(file.scene.beats[1].id);

    // Both of us read the same starting arrangement.
    let mut ours = project.scene_layout(&file.scene).layout;
    let mut nadia = project.scene_layout(&file.scene).layout;

    // She drags her box and her machine's write lands first.
    nadia.place(hers, 1000.0, 1000.0);
    fs::write(layout_path(&project, file.scene.id), serde_json::to_string_pretty(&nadia).unwrap())
        .unwrap();

    // We drag ours, from the copy we read before hers arrived.
    ours.place(mine, 2000.0, 2000.0);
    project.save_scene_layout(&file.scene, &ours).unwrap();

    let merged = project.scene_layout(&file.scene).layout;
    assert_eq!(merged.nodes[&hers].x, 1000.0, "Nadia's drag was lost");
    assert_eq!(merged.nodes[&mine].x, 2000.0, "our drag was lost");
    // Nothing was parked: a layout race is not a conflict anybody has to look
    // at, and a diff card over a rectangle would train people to dismiss them.
    assert!(siblings(&layout_path(&project, file.scene.id), "conflict").is_empty());
}

#[test]
fn two_writers_dragging_the_same_box_resolve_to_the_later_edit() {
    let (_dir, project, file) = arranged();
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();
    let contested = NodeKey::Beat(file.scene.beats[0].id);

    let mut nadia = project.scene_layout(&file.scene).layout;
    nadia.place(contested, 1000.0, 1000.0);
    nadia.nodes.get_mut(&contested).unwrap().updated_at = Utc::now() + Duration::seconds(30);
    fs::write(layout_path(&project, file.scene.id), serde_json::to_string_pretty(&nadia).unwrap())
        .unwrap();

    let mut ours = project.scene_layout(&file.scene).layout;
    ours.place(contested, 5.0, 5.0);
    project.save_scene_layout(&file.scene, &ours).unwrap();

    let merged = project.scene_layout(&file.scene).layout;
    assert_eq!(merged.nodes[&contested].x, 1000.0, "the later edit should win");
}

#[test]
fn a_layout_edit_during_a_source_save_blocks_neither() {
    // The acceptance criterion in one test: a layout write must never make a
    // source save conflict, and a source save must never make a layout write
    // fail. They are different files, and this is what proves it stays that
    // way.
    let (_dir, mut project, mut file) = arranged();
    let layout = drag_everything(&project, &file.scene);

    // The canvas saves an arrangement while the editor is holding an
    // unsaved scene with a stamp from before it.
    for _ in 0..3 {
        assert!(matches!(
            project.save_scene_layout(&file.scene, &layout).unwrap(),
            LayoutSave::Written(_)
        ));
    }

    file.scene.summary = "The guild asks about the fire.".into();
    let outcome = project.save_scene(&mut file).unwrap();
    assert!(matches!(outcome, SourceSave::Saved(_)), "a layout write conflicted a source save");

    // And the other order.
    file.scene.summary = "They ask twice.".into();
    project.save_scene(&mut file).unwrap();
    assert!(matches!(
        project.save_scene_layout(&file.scene, &layout).unwrap(),
        LayoutSave::Written(_)
    ));
}

#[test]
fn a_layout_file_never_reaches_the_conflict_card() {
    // Layout merges, so it never parks a sibling — and a corrupt sidecar is
    // parked under a different marker on purpose, so the card that arbitrates
    // somebody's only copy of a paragraph is never handed a rectangle.
    let (_dir, project, file) = arranged();
    let path = layout_path(&project, file.scene.id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "not json at all").unwrap();
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();

    assert_eq!(siblings(&path, "corrupt").len(), 1);
    assert!(project.conflicts().unwrap().is_empty(), "a layout file must not raise a diff");
}

#[test]
fn re_saving_an_identical_arrangement_does_not_touch_the_file() {
    // Every watcher on the share, and every sync client the folder sits
    // under, treats a moved stamp as somebody's edit. A canvas that rewrote
    // its sidecar on every open would wake all of them for nothing.
    let (_dir, project, file) = arranged();
    let layout = drag_everything(&project, &file.scene);
    project.save_scene_layout(&file.scene, &layout).unwrap();

    let path = layout_path(&project, file.scene.id);
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let bytes = fs::read(&path).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));

    let reloaded = project.scene_layout(&file.scene).layout;
    project.save_scene_layout(&file.scene, &reloaded).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before, "the stamp moved");
}

/* ── the folder travels ───────────────────────────────────────────────────── */

#[test]
fn a_copied_project_folder_restores_the_arrangement_on_a_fresh_machine() {
    let (dir, project, file) = arranged();
    let mut layout = drag_everything(&project, &file.scene);
    layout.set_mode(LayoutMode::Manual);
    project.save_scene_layout(&file.scene, &layout).unwrap();
    let root = project.root().to_path_buf();
    let mine = project.scene_layout(&file.scene).layout;
    drop(project);

    let elsewhere = dir.path().join("fresh");
    copy_to(&root, &elsewhere);
    // A machine that has never seen this project has no index and no `.wobu`.
    fs::remove_dir_all(elsewhere.join(".wobu")).ok();

    let copy = Project::open_at_index(&elsewhere, &dir.path().join("fresh.sqlite")).unwrap();
    let scene = copy.load_scene(file.scene.id).unwrap().scene;
    let theirs = copy.scene_layout(&scene);
    assert_eq!(theirs.layout, mine, "the arrangement did not travel");
    assert_eq!(theirs.layout.mode, LayoutMode::Manual);
    assert!(theirs.notices.is_empty());
}

#[test]
fn deleting_a_scene_takes_its_layout_with_it() {
    let (_dir, mut project, file) = arranged();
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();
    let path = layout_path(&project, file.scene.id);
    assert!(path.is_file());

    project.delete_scene(file.scene.id).unwrap();
    assert!(!path.exists());
}

#[test]
fn a_sweep_collects_orphan_layouts_and_leaves_live_ones_alone() {
    let (_dir, project, file) = arranged();
    project.save_scene_layout(&file.scene, &drag_everything(&project, &file.scene)).unwrap();

    // A layout for a scene that is not in the folder — what a crash between
    // the two deletes, or a scene removed with a text editor, leaves behind.
    let orphan = SceneId::new();
    let mut stray = Layout::empty(GraphKey::of_scene(orphan));
    stray.place(NodeKey::Scene(orphan), 1.0, 1.0);
    fs::write(layout_path(&project, orphan), serde_json::to_string_pretty(&stray).unwrap())
        .unwrap();

    let removed = project.sweep_scene_layouts().unwrap();
    assert_eq!(removed.len(), 1);
    assert!(removed[0].contains(&orphan.to_string()));
    assert!(layout_path(&project, file.scene.id).is_file(), "the live one must survive");
}

#[test]
fn an_arc_graph_has_its_own_layout_keyed_by_scene_id() {
    let (_dir, mut project, file) = arranged();
    let other = project.create_scene("The Long Question").unwrap();

    let mut arc = Layout::empty(GraphKey::Arc { arc: "The Kiln Job".into() });
    arc.place(NodeKey::Scene(file.scene.id), 0.0, 0.0);
    arc.place(NodeKey::Scene(other.scene.id), 240.0, 0.0);
    let ghost = SceneId::new();
    arc.place(NodeKey::Scene(ghost), 480.0, 0.0);
    project.save_arc_layout(&arc).unwrap();

    let loaded = project.arc_layout("The Kiln Job");
    assert_eq!(
        loaded.layout.nodes.len(),
        2,
        "a scene that is not in the project is not on the arc"
    );
    assert!(loaded.layout.nodes.contains_key(&NodeKey::Scene(file.scene.id)));
    assert!(!loaded.layout.nodes.contains_key(&NodeKey::Scene(ghost)));
    assert!(
        project.root().join("narrative/layout/arcs/the-kiln-job.json").is_file(),
        "the arc slug names the file"
    );
}

/* ── the watcher ──────────────────────────────────────────────────────────── */

#[test]
fn the_watcher_notices_the_narrative_tree_appearing_and_then_writes_inside_it() {
    // A project that has no narrative has no `narrative/` directory to watch,
    // and creating one at open would change every art-only folder. So the
    // watcher has to pick the tree up when it appears — from a peer, a `git
    // pull`, or this machine's first scene.
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let root = project.root().to_path_buf();
    assert!(!root.join("narrative").exists());

    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let _watcher = wobu_store::Watcher::start(&root, move |_| {
        counter.fetch_add(1, Ordering::Relaxed);
        true
    })
    .unwrap();

    // Creating the tree fires through the root's own watch.
    project.create_scene("Kiln Interrogation").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1200));
    let after_create = hits.load(Ordering::Relaxed);
    assert!(after_create >= 1, "the narrative tree appearing was not noticed");

    // And now writes *inside* it are seen, which is the part that needs the
    // watch to have been attached after the fact.
    fs::write(root.join("narrative/scenes/kiln-interrogation.yaml"), "schema_version: 1\n")
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert!(
        hits.load(Ordering::Relaxed) > after_create,
        "an external edit inside narrative/ was not noticed"
    );
}

/* ── helpers ──────────────────────────────────────────────────────────────── */

fn unplaced(notices: &[LayoutNotice]) -> Vec<NodeKey> {
    notices
        .iter()
        .find_map(|notice| match notice {
            LayoutNotice::Unplaced { nodes } => Some(nodes.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn siblings(path: &Path, marker: &str) -> Vec<PathBuf> {
    let needle = format!(".{marker}-");
    let Ok(entries) = fs::read_dir(path.parent().unwrap()) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate.file_name().is_some_and(|n| n.to_string_lossy().contains(&needle))
        })
        .collect();
    found.sort();
    found
}

fn copy_to(root: &Path, destination: &Path) {
    for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
        let relative = entry.path().strip_prefix(root).unwrap();
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).unwrap();
        } else if entry.file_type().is_file() {
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}
