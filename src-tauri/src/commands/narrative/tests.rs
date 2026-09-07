//! What the narrative command layer promises, against a real project folder.
//!
//! These drive the free functions rather than the `#[tauri::command]` wrappers,
//! for the same reason `commands/mesh/tests.rs` does: the wrapper is one call
//! to `AppState::with`, and standing up a Tauri managed-state harness would
//! test Tauri. Everything below the wrapper — the precondition, the path
//! choice, the diagnostic keying, the promise that a layout write cannot fail —
//! is here.

use std::path::{Path, PathBuf};

use wobu_narrative::{
    Beat, BeatId, Choice, Destination, DialogueSlot, SceneId, Speaker, StateDocument,
};
use wobu_store::{GraphKey, Layout, NodeKey, Project};

use super::*;

/// A project folder that removes itself.
struct Temp(PathBuf);

impl Temp {
    fn new() -> Temp {
        let dir = std::env::temp_dir().join(format!("wobu-narrative-cmd-{}", wobu_core::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn project(temp: &Temp) -> Project {
    Project::create(temp.path(), "Ashfall").unwrap()
}

fn stamp_of(view: &SceneFileView) -> Precondition {
    Precondition::Stamp { stamp: view.stamp.clone().expect("a saved scene has a stamp") }
}

/* ── catalog ──────────────────────────────────────────────────────────────── */

#[test]
fn the_catalog_lists_scenes_and_names_the_files_it_could_not_read() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let created = project.create_scene("Council hearing").unwrap();

    // A scene a sync client copied half-written. It has no readable id, so it
    // cannot appear as a scene — and omitting it entirely would present
    // somebody's file as deleted.
    let broken = project.root().join("narrative/scenes/broken.yaml");
    std::fs::write(broken, "schema_version: 1\nscene:\n  id: [").unwrap();

    let view = scenes(&project).unwrap();
    assert_eq!(view.scenes.len(), 1);
    assert_eq!(view.scenes[0].id, created.scene.id);
    assert_eq!(view.scenes[0].slug, "council-hearing");
    assert_eq!(view.scenes[0].rel, "narrative/scenes/council-hearing.yaml");
    assert_eq!(view.unreadable.len(), 1, "{view:?}");
    assert_eq!(view.unreadable[0].rel, "narrative/scenes/broken.yaml");
}

/* ── the guarded write ────────────────────────────────────────────────────── */

#[test]
fn a_save_hands_back_the_precondition_for_the_next_one() {
    // The whole reason the stamp travels on the document: an editor that saves
    // twice in a row must not present the first save's precondition to the
    // second and park its own work as a conflict.
    let temp = Temp::new();
    let mut project = project(&temp);
    let created = project.create_scene("Council hearing").unwrap();
    let first = SceneFileView::of(&created);

    let mut scene = first.scene.clone();
    scene.summary = "The captain answers for the beacon.".into();
    let second = save_scene(&mut project, scene, None, &stamp_of(&first)).unwrap();

    let mut scene = second.scene.clone();
    scene.summary = "The captain answers for the beacon, badly.".into();
    let third = save_scene(&mut project, scene, None, &stamp_of(&second)).unwrap();

    assert_ne!(second.stamp, third.stamp);
    assert_eq!(
        project.load_scene(created.scene.id).unwrap().scene.summary,
        "The captain answers for the beacon, badly."
    );
}

#[test]
fn a_stale_precondition_parks_a_conflict_rather_than_overwriting() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let created = project.create_scene("Council hearing").unwrap();
    let opened = SceneFileView::of(&created);

    // Somebody else — a collaborator, the same user in a second tab — saves.
    let mut theirs = opened.scene.clone();
    theirs.summary = "theirs".into();
    save_scene(&mut project, theirs, None, &stamp_of(&opened)).unwrap();

    // Our editor still holds the version it opened.
    let mut ours = opened.scene.clone();
    ours.summary = "ours".into();
    let refused = save_scene(&mut project, ours, None, &stamp_of(&opened)).unwrap_err();

    assert_eq!(refused.code.as_str(), "write.conflict");
    assert!(refused.conflict_path.is_some(), "the loser has to be reachable: {refused:?}");
    assert_eq!(
        project.load_scene(created.scene.id).unwrap().scene.summary,
        "theirs",
        "the winner's file must be untouched"
    );
}

#[test]
fn believing_a_file_is_new_when_it_is_not_is_a_conflict_and_not_an_overwrite() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let created = project.create_scene("Council hearing").unwrap();

    let mut scene = created.scene.clone();
    scene.summary = "ours".into();
    let refused = save_scene(&mut project, scene, None, &Precondition::New).unwrap_err();

    assert_eq!(refused.code.as_str(), "write.conflict");
    assert_eq!(project.load_scene(created.scene.id).unwrap().scene.summary, "");
}

#[test]
fn the_current_precondition_is_the_undo_path_and_lands_over_this_sessions_own_save() {
    // Undo replays a version this session recorded. Refusing it because this
    // session's *own* later save moved the file would make every second press
    // of ⌘Z fail.
    let temp = Temp::new();
    let mut project = project(&temp);
    let created = project.create_scene("Council hearing").unwrap();
    let before = SceneFileView::of(&created);

    let mut after = before.scene.clone();
    after.summary = "edited".into();
    save_scene(&mut project, after, None, &stamp_of(&before)).unwrap();

    // The recorded inverse carries no stamp of its own.
    save_scene(&mut project, before.scene.clone(), Some(&before.slug), &Precondition::Current)
        .unwrap();
    assert_eq!(project.load_scene(created.scene.id).unwrap().scene.summary, "");
}

#[test]
fn undoing_a_delete_brings_the_scene_back_as_itself() {
    // Every id under the scene has to survive, or a locale row, a recording and
    // every destination pointing into it resolve to nothing.
    let temp = Temp::new();
    let mut project = project(&temp);
    let created = project.create_scene("Council hearing").unwrap();
    let mut file = created.clone();
    file.scene.beats.push(Beat::new("Present evidence"));
    project.save_scene(&mut file).unwrap();

    let recorded = SceneFileView::of(&file);
    let beat_id = recorded.scene.beats[0].id;
    project.delete_scene(recorded.scene.id).unwrap();
    assert!(project.scene_catalog().unwrap().find(recorded.scene.id).is_none());

    let restored = save_scene(
        &mut project,
        recorded.scene.clone(),
        Some(&recorded.slug),
        &Precondition::Current,
    )
    .unwrap();

    assert_eq!(restored.scene.id, recorded.scene.id);
    assert_eq!(restored.scene.beats[0].id, beat_id);
    assert_eq!(restored.rel, recorded.rel, "and at its own file");
}

#[test]
fn a_restore_never_lands_on_top_of_a_scene_that_took_the_name() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let recorded = SceneFileView::of(&project.create_scene("Council hearing").unwrap());
    project.delete_scene(recorded.scene.id).unwrap();

    // Somebody makes a new scene with the same name while the delete is
    // undoable. It mints the slug that is now free.
    let squatter = project.create_scene("Council hearing").unwrap();
    assert_eq!(squatter.rel, recorded.rel);

    let restored = save_scene(
        &mut project,
        recorded.scene.clone(),
        Some(&recorded.slug),
        &Precondition::Current,
    )
    .unwrap();

    assert_ne!(restored.rel, squatter.rel, "a filename is not worth another scene");
    assert_eq!(restored.scene.id, recorded.scene.id, "but the scene is still itself");
    assert_eq!(project.scene_catalog().unwrap().scenes.len(), 2);
}

#[test]
fn a_stale_editor_cannot_write_a_scene_to_a_path_it_remembers() {
    // The catalog is authoritative for a scene that exists, so a caller's slug
    // is ignored rather than honoured.
    let temp = Temp::new();
    let mut project = project(&temp);
    let created = project.create_scene("Council hearing").unwrap();

    let saved = save_scene(
        &mut project,
        created.scene.clone(),
        Some("somewhere-else"),
        &Precondition::Current,
    )
    .unwrap();

    assert_eq!(saved.rel, "narrative/scenes/council-hearing.yaml");
    assert_eq!(project.scene_catalog().unwrap().scenes.len(), 1);
}

#[test]
fn renaming_moves_no_file_and_mints_no_id() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let mut created = project.create_scene("Council hearing").unwrap();
    created.scene.beats.push(Beat::new("Present evidence"));
    project.save_scene(&mut created).unwrap();
    let beat_id = created.scene.beats[0].id;

    let renamed = rename_scene(&mut project, created.scene.id, "The hearing".into()).unwrap();

    assert_eq!(renamed.scene.name, "The hearing");
    assert_eq!(renamed.rel, created.rel, "a moved file is a delete and a create to every watcher");
    assert_eq!(renamed.slug, "council-hearing");
    assert_eq!(renamed.scene.id, created.scene.id);
    assert_eq!(renamed.scene.beats[0].id, beat_id);
    assert_eq!(project.load_scene(created.scene.id).unwrap().scene.name, "The hearing");
}

#[test]
fn renaming_reads_its_precondition_rather_than_being_handed_a_stale_one() {
    // The Library row that offers Rename holds a name and an id, never a
    // version of the file. Whatever it could have sent would be stale by
    // definition, so the precondition is read in the same critical section as
    // the write — and the rename lands on top of an edit made since.
    let temp = Temp::new();
    let mut project = project(&temp);
    let created = project.create_scene("Council hearing").unwrap();
    let opened = SceneFileView::of(&created);

    let mut edited = opened.scene.clone();
    edited.summary = "edited elsewhere".into();
    save_scene(&mut project, edited, None, &stamp_of(&opened)).unwrap();

    let renamed = rename_scene(&mut project, created.scene.id, "The hearing".into()).unwrap();
    assert_eq!(renamed.scene.name, "The hearing");
    assert_eq!(renamed.scene.summary, "edited elsewhere", "a rename changes one field");
}

/* ── declared state ───────────────────────────────────────────────────────── */

#[test]
fn state_that_does_not_hold_together_is_refused_where_the_author_can_see_it() {
    let temp = Temp::new();
    let mut project = project(&temp);

    let yaml = "schema_version: 1\nvariables:\n  - name: trust\n    type: !int { min: 0, max: 10 }\n    default: 99\n";
    let document = StateDocument::parse(yaml).unwrap();
    let refused = save_state(&mut project, document, &Precondition::New).unwrap_err();

    assert_eq!(refused.code.as_str(), "node.invalid");
    assert!(project.state_document().unwrap().is_none(), "and nothing was written");
}

#[test]
fn a_project_with_no_state_file_reads_as_an_empty_schema_rather_than_an_error() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let document = StateDocument::parse(
        "schema_version: 1\nvariables:\n  - name: trust\n    type: !int { min: 0, max: 10 }\n    default: 0\n",
    )
    .unwrap();

    let saved = save_state(&mut project, document, &Precondition::New).unwrap();
    assert!(saved.stamp.is_some());
    assert_eq!(project.state_schema().unwrap().len(), 1);
}

/* ── diagnostics ──────────────────────────────────────────────────────────── */

#[test]
fn diagnostics_are_keyed_by_the_ids_the_canvas_and_the_list_both_hold() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let mut file = project.create_scene("Council hearing").unwrap();

    let mut beat = Beat::new("Present evidence");
    // A destination that names a beat which is not in this scene.
    beat.choices.push(Choice::new("Show the logbook", Destination::Beat(BeatId::new())));
    beat.dialogue.push(DialogueSlot::new(Speaker::Narrator));
    let beat_id = beat.id;
    let choice_id = beat.choices[0].id;
    let slot_id = beat.dialogue[0].id;
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();

    let found = diagnostics(&project, file.scene.id, None).unwrap();

    let dangling =
        found.iter().find(|d| d.code == "dangling_beat").unwrap_or_else(|| panic!("{found:?}"));
    assert_eq!(dangling.kind, "choice");
    assert_eq!(dangling.beat_id, Some(beat_id));
    assert_eq!(dangling.choice_id, Some(choice_id));
    assert!(dangling.destination, "the canvas draws this without matching on codes");

    let missing =
        found.iter().find(|d| d.code == "missing_text").unwrap_or_else(|| panic!("{found:?}"));
    assert_eq!(missing.kind, "dialogueSlot");
    assert_eq!(missing.slot_id, Some(slot_id));
    assert!(!missing.destination);
}

#[test]
fn an_unsaved_scene_is_diagnosed_as_the_writer_sees_it() {
    // A canvas connection made thirty seconds ago must stop being reported as
    // broken before it is saved, or the list is one people learn to ignore.
    let temp = Temp::new();
    let mut project = project(&temp);
    let mut file = project.create_scene("Council hearing").unwrap();
    file.scene.beats.push(Beat::new("Present evidence"));
    project.save_scene(&mut file).unwrap();

    let saved = diagnostics(&project, file.scene.id, None).unwrap();
    assert!(saved.iter().any(|d| d.code == "no_destination"), "{saved:?}");

    let mut edited = file.scene.clone();
    edited.beats[0]
        .choices
        .push(Choice::new("Show the logbook", Destination::End { label: "heard".into() }));
    let live = diagnostics(&project, file.scene.id, Some(edited)).unwrap();
    assert!(!live.iter().any(|d| d.code == "no_destination"), "{live:?}");
}

#[test]
fn a_link_to_a_scene_that_exists_is_not_reported_as_broken() {
    // The catalog always comes from the project, so a cross-scene destination
    // is checked against the scenes that really exist.
    let temp = Temp::new();
    let mut project = project(&temp);
    let other = project.create_scene("Verdict").unwrap();
    let mut file = project.create_scene("Council hearing").unwrap();

    let mut beat = Beat::new("Present evidence");
    beat.choices.push(Choice::new("Move to a verdict", Destination::Scene(other.scene.id)));
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();

    let found = diagnostics(&project, file.scene.id, None).unwrap();
    assert!(!found.iter().any(|d| d.code == "unknown_scene"), "{found:?}");

    project.delete_scene(other.scene.id).unwrap();
    let found = diagnostics(&project, file.scene.id, None).unwrap();
    assert!(found.iter().any(|d| d.code == "unknown_scene"), "{found:?}");
}

/* ── layout ───────────────────────────────────────────────────────────────── */

#[test]
fn moving_a_box_does_not_move_the_narrative_fingerprint() {
    // #185's central claim, checked at the command layer rather than only in
    // the store: the fingerprint walks source paths and structurally cannot
    // reach `narrative/layout/`.
    let temp = Temp::new();
    let mut project = project(&temp);
    let mut file = project.create_scene("Council hearing").unwrap();
    file.scene.beats.push(Beat::new("Present evidence"));
    project.save_scene(&mut file).unwrap();

    let before = project.narrative_fingerprint().unwrap();

    let mut layout = Layout::empty(GraphKey::of_scene(file.scene.id));
    layout.place(NodeKey::Beat(file.scene.beats[0].id), 120.0, 40.0);
    assert_eq!(layout_save(&project, &layout), LayoutSaveView::Written);

    assert_eq!(project.narrative_fingerprint().unwrap(), before);
    assert_eq!(project.load_scene(file.scene.id).unwrap().scene, file.scene);
}

#[test]
fn a_layout_save_reports_rather_than_fails() {
    // The canvas autosaves one of these per drag. A failure that reached the
    // error surface would be a toast per rectangle.
    let temp = Temp::new();
    let project = project(&temp);

    let layout = Layout::empty(GraphKey::of_scene(SceneId::new()));
    match layout_save(&project, &layout) {
        LayoutSaveView::Unwritable { reason } => assert!(!reason.is_empty()),
        other => panic!("a missing scene must not be a rejected promise: {other:?}"),
    }
}

#[test]
fn a_scene_that_was_never_arranged_loads_with_a_non_blocking_notice() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let mut file = project.create_scene("Council hearing").unwrap();
    file.scene.beats.push(Beat::new("Present evidence"));
    project.save_scene(&mut file).unwrap();

    let loaded = layout_get(&project, &GraphKey::of_scene(file.scene.id));
    assert!(!loaded.notices.is_empty());
    assert!(loaded.notices.iter().all(|n| !n.blocking), "{:?}", loaded.notices);
    let unplaced = loaded.notices.iter().find(|n| n.kind == "unplaced").expect("laid out on open");
    assert!(
        unplaced.nodes.iter().any(|key| key.starts_with("beat:")),
        "a notice has to name nodes in the canvas's own vocabulary: {unplaced:?}"
    );
}

#[test]
fn a_saved_arrangement_comes_back_and_a_deleted_beats_position_does_not() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let mut file = project.create_scene("Council hearing").unwrap();
    file.scene.beats.push(Beat::new("Present evidence"));
    project.save_scene(&mut file).unwrap();
    let beat_id = file.scene.beats[0].id;

    let mut layout = Layout::empty(GraphKey::of_scene(file.scene.id));
    layout.place(NodeKey::Beat(beat_id), 120.0, 40.0);
    layout_save(&project, &layout);

    let loaded = layout_get(&project, &GraphKey::of_scene(file.scene.id));
    let placed = loaded.layout.nodes.get(&NodeKey::Beat(beat_id)).expect("saved position");
    assert_eq!((placed.x, placed.y), (120.0, 40.0));

    file.scene.beats.clear();
    project.save_scene(&mut file).unwrap();
    let loaded = layout_get(&project, &GraphKey::of_scene(file.scene.id));
    assert!(!loaded.layout.nodes.contains_key(&NodeKey::Beat(beat_id)));
    assert!(loaded.notices.iter().any(|n| n.kind == "stale"), "{:?}", loaded.notices);
}

/* ── the bridge contract ──────────────────────────────────────────────────── */

// `src/lib/api/narrative.ts` hand-writes the TypeScript for everything below.
// Nothing generates one side from the other, so these feed the Rust side the
// literal JSON the webview sends, and read back the literal keys it destructures.

#[test]
fn a_precondition_arrives_in_the_three_shapes_the_webview_can_send() {
    let stamp: Precondition = serde_json::from_value(serde_json::json!({
        "kind": "stamp",
        "stamp": { "mtime_ms": 1_700_000_000_000i64, "size": 42, "hash": "abc" },
    }))
    .unwrap();
    match stamp {
        Precondition::Stamp { stamp } => assert_eq!(stamp.size, 42),
        other => panic!("{other:?}"),
    }

    assert_eq!(
        serde_json::from_value::<Precondition>(serde_json::json!({ "kind": "new" })).unwrap(),
        Precondition::New
    );
    assert_eq!(
        serde_json::from_value::<Precondition>(serde_json::json!({ "kind": "current" })).unwrap(),
        Precondition::Current
    );
}

#[test]
fn a_graph_key_arrives_in_the_shape_the_layout_file_already_uses() {
    // The same spelling the sidecar stores, so a layout the webview loaded can
    // be handed straight back without a translation step that could disagree.
    let scene = SceneId::new();
    let graph: GraphKey = serde_json::from_value(serde_json::json!({
        "kind": "scene",
        "scene": scene.to_string(),
    }))
    .unwrap();
    assert_eq!(graph, GraphKey::of_scene(scene));

    let graph: GraphKey =
        serde_json::from_value(serde_json::json!({ "kind": "arc", "arc": "beacon" })).unwrap();
    assert_eq!(graph, GraphKey::Arc { arc: "beacon".into() });
}

#[test]
fn a_diagnostic_serialises_flat_with_the_ids_both_surfaces_key_on() {
    let beat = BeatId::new();
    let choice = ChoiceId::new();
    let view = DiagnosticView::of(&Diagnostic {
        site: Site::Destination(DestinationSite::Choice { beat, choice }),
        problem: Problem::DanglingBeat { beat },
    });
    let json = serde_json::to_value(&view).unwrap();

    assert_eq!(json["kind"], "choice");
    assert_eq!(json["code"], "dangling_beat");
    assert_eq!(json["destination"], true);
    assert_eq!(json["beatId"], beat.to_string());
    assert_eq!(json["choiceId"], choice.to_string());
    // Absent rather than null: TypeScript reads these as optional fields.
    assert!(json.get("slotId").is_none(), "{json}");
    assert!(json.get("entityId").is_none(), "{json}");
    assert!(!json["message"].as_str().unwrap().is_empty());
}

#[test]
fn a_layout_save_outcome_is_discriminated_by_a_field_the_webview_switches_on() {
    let json = serde_json::to_value(LayoutSaveView::Written).unwrap();
    assert_eq!(json["outcome"], "written");

    let json = serde_json::to_value(LayoutSaveView::Deferred {
        rel: "narrative/layout/scenes/x.json".into(),
        found: 2,
        supported: 1,
    })
    .unwrap();
    assert_eq!(json["outcome"], "deferred");
    assert_eq!(json["supported"], 1);

    let json =
        serde_json::to_value(LayoutSaveView::Unwritable { reason: "read only".into() }).unwrap();
    assert_eq!(json["outcome"], "unwritable");
    assert_eq!(json["reason"], "read only");
}

#[test]
fn a_scene_crosses_the_bridge_in_the_source_files_own_spelling() {
    // Deliberate, and pinned so nobody "fixes" it: the Source tab edits this
    // YAML directly, and a camelCase mirror would be a second copy of the
    // narrative model to keep in step with `deny_unknown_fields`.
    let temp = Temp::new();
    let mut project = project(&temp);
    let mut file = project.create_scene("Council hearing").unwrap();
    file.scene.beats.push(Beat::new("Present evidence"));
    file.scene.beats[0].must_convey.push("the beacon was lit".into());
    project.save_scene(&mut file).unwrap();

    let json = serde_json::to_value(SceneFileView::of(&file)).unwrap();
    assert!(json.get("stamp").is_some(), "the precondition travels with the document");
    assert_eq!(json["scene"]["beats"][0]["must_convey"][0], "the beacon was lit");
    // And the envelope around it is camelCase like every other command payload.
    assert!(json["rel"].as_str().unwrap().starts_with("narrative/scenes/"));
}
