//! Narrative source in a real project folder (#153's core).
//!
//! The unit tests in `narrative::mod` cover the path helpers; these drive the
//! whole way a scene takes to disk and back, over a real filesystem, and then
//! do the things that actually happen to a project folder: copy it to another
//! machine, edit it in a text editor, race two writers at it, and open a
//! project that has never contained a scene at all.

use std::fs;
use std::path::Path;

use wobu_core::NodeKind;
use wobu_narrative::{
    Beat, Destination, DialogueSlot, Name, Outcome, Scene, SceneDocument, Speaker, StateDocument,
    Text, Value, VarType, VariableDecl, Variant,
};
use wobu_store::{Project, SourceSave};

/// A scene with enough in it to be worth persisting: a beat, a line, an end.
fn kiln_scene() -> Scene {
    let mut scene = Scene::new("Kiln Interrogation");
    scene.summary = "The guild asks about the fire.".into();
    let mut beat = Beat::new("Opening");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written("The kiln is still warm.")));
    beat.dialogue.push(slot);
    beat.outcomes.push(Outcome::new(Destination::End { label: "silence".into() }));
    scene.beats.push(beat);
    scene
}

fn project() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::create(dir.path(), "Ashfall").unwrap();
    (dir, project)
}

/// Copy a project folder byte for byte, as putting it on a USB stick does.
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

#[test]
fn an_art_only_project_opens_completely_unchanged() {
    // #153 and #185 both promise this, and it is the cheapest promise to break
    // by accident: one `ensure_dir` on the open path and every existing world
    // grows a folder it never asked for.
    let (dir, mut project) = project();
    project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    let root = project.root().to_path_buf();
    drop(project);

    let before = tree(&root);
    let project = Project::open_at_index(&root, &dir.path().join("reopen.sqlite")).unwrap();
    assert!(project.scene_catalog().unwrap().scenes.is_empty());
    assert!(project.state_document().unwrap().is_none());
    assert_eq!(project.state_schema().unwrap().len(), 0);
    assert_eq!(tree(&root), before, "opening an art-only project must not add a narrative tree");
    assert!(!root.join("narrative").exists());
}

/// Every path in a project folder, sorted. Directories included, because the
/// thing being guarded against is a directory appearing.
fn tree(root: &Path) -> Vec<String> {
    let mut paths: Vec<String> = walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.path().strip_prefix(root).ok().map(|p| p.display().to_string()))
        // The index probe and staging area churn on every open and are not
        // part of what the folder *is*.
        .filter(|p| !p.starts_with(".wobu"))
        .collect();
    paths.sort();
    paths
}

#[test]
fn a_scene_round_trips_through_the_folder() {
    let (_dir, mut project) = project();
    let mut file = project.create_scene("Kiln Interrogation").unwrap();
    file.scene = kiln_scene();
    let id = file.scene.id;
    assert!(matches!(project.save_scene(&mut file).unwrap(), SourceSave::Saved(_)));

    assert_eq!(file.rel, "narrative/scenes/kiln-interrogation.yaml");
    assert!(project.root().join(&file.rel).is_file());

    let read = project.load_scene(id).unwrap();
    assert_eq!(read.scene, file.scene);
    assert_eq!(read.scene.beats[0].dialogue[0].variants[0].text.body, "The kiln is still warm.");
}

#[test]
fn two_scenes_with_one_name_get_two_files() {
    let (_dir, mut project) = project();
    let first = project.create_scene("Kiln Interrogation").unwrap();
    let second = project.create_scene("Kiln Interrogation").unwrap();
    assert_ne!(first.rel, second.rel);
    assert_eq!(second.rel, "narrative/scenes/kiln-interrogation-2.yaml");
    assert_eq!(project.scene_catalog().unwrap().scenes.len(), 2);
}

#[test]
fn renaming_a_scene_does_not_move_its_file() {
    // A moved file is a delete and a create to every sync client, every
    // watcher and every git history looking at the folder — and it would
    // orphan anything else keyed to the path. The slug is minted once.
    let (_dir, mut project) = project();
    let mut file = project.create_scene("Kiln Interrogation").unwrap();
    let rel = file.rel.clone();
    file.scene.name = "The Long Question".into();
    project.save_scene(&mut file).unwrap();

    assert_eq!(file.rel, rel);
    assert!(project.root().join(&rel).is_file());
    assert_eq!(project.scene_catalog().unwrap().scenes[0].name, "The Long Question");
}

#[test]
fn a_copied_project_folder_opens_with_its_scenes_on_a_fresh_machine() {
    // No absolute paths, no machine-local state: the whole point of the
    // folder format. The copy gets its own index, as a machine that has never
    // seen this project would.
    let (dir, mut project) = project();
    let mut file = project.create_scene("Kiln Interrogation").unwrap();
    file.scene = kiln_scene();
    project.save_scene(&mut file).unwrap();
    let fingerprint = project.narrative_fingerprint().unwrap();
    let id = file.scene.id;
    let root = project.root().to_path_buf();
    drop(project);

    let elsewhere = dir.path().join("fresh");
    copy_to(&root, &elsewhere);
    fs::remove_dir_all(elsewhere.join(".wobu")).ok();

    let copy = Project::open_at_index(&elsewhere, &dir.path().join("fresh.sqlite")).unwrap();
    assert_eq!(copy.load_scene(id).unwrap().scene, file.scene);
    assert_eq!(copy.narrative_fingerprint().unwrap(), fingerprint);
}

#[test]
fn a_scene_edited_outside_wobu_is_read_back() {
    // Obsidian, `vim`, a `git pull`. The folder is canonical, so whatever a
    // text editor left there is what the next read has to return.
    let (_dir, mut project) = project();
    let mut file = project.create_scene("Kiln Interrogation").unwrap();
    file.scene = kiln_scene();
    project.save_scene(&mut file).unwrap();

    let path = project.root().join(&file.rel);
    let text = fs::read_to_string(&path).unwrap().replace("Kiln Interrogation", "Kiln, Later");
    fs::write(&path, text).unwrap();

    assert_eq!(project.load_scene(file.scene.id).unwrap().scene.name, "Kiln, Later");
}

#[test]
fn a_second_writer_saving_first_parks_a_conflict_rather_than_losing_words() {
    // Source keeps the never-merge rule in full. Layout is the documented
    // exception and this test is here to make sure the exception did not leak.
    let (_dir, mut project) = project();
    let mut mine = project.create_scene("Kiln Interrogation").unwrap();
    mine.scene = kiln_scene();
    project.save_scene(&mut mine).unwrap();

    // Nadia is another machine, which is to say: bytes changing under us.
    let path = project.root().join(&mine.rel);
    let mut theirs = mine.scene.clone();
    theirs.summary = "Nadia's summary".into();
    fs::write(&path, SceneDocument::new(theirs).to_yaml().unwrap()).unwrap();

    mine.scene.summary = "My summary".into();
    let outcome = project.save_scene(&mut mine).unwrap();
    let SourceSave::Conflict { conflict_path } = outcome else {
        panic!("expected a conflict, got {outcome:?}");
    };
    assert!(conflict_path.starts_with("narrative/scenes/"));
    assert!(project.root().join(&conflict_path).is_file());
    // Nadia's words are still the ones on the canonical path, and mine are
    // still on disk beside them. Nobody lost a sentence.
    assert!(fs::read_to_string(&path).unwrap().contains("Nadia's summary"));
    assert!(
        fs::read_to_string(project.root().join(&conflict_path)).unwrap().contains("My summary")
    );
    // And the conflict sibling is not itself a scene.
    assert_eq!(project.scene_catalog().unwrap().scenes.len(), 1);
}

#[test]
fn a_truncated_scene_file_is_reported_rather_than_hidden() {
    let (_dir, mut project) = project();
    let file = project.create_scene("Kiln Interrogation").unwrap();
    fs::write(project.root().join(&file.rel), "schema_version: 1\nscene: [").unwrap();

    let catalog = project.scene_catalog().unwrap();
    assert!(catalog.scenes.is_empty());
    assert_eq!(catalog.unreadable.len(), 1, "a broken file must not read as a deleted one");
    assert_eq!(catalog.unreadable[0].rel, file.rel);
}

#[test]
fn declared_state_persists_and_type_checks() {
    let (_dir, mut project) = project();
    let trusted = VariableDecl {
        name: Name::new("trusted").unwrap(),
        ty: VarType::Bool,
        default: Value::Bool(false),
        owner: Default::default(),
        description: String::new(),
    };
    let document = StateDocument::new(vec![trusted]);
    assert!(matches!(project.save_state(&document, None).unwrap(), SourceSave::Saved(_)));

    let (read, stamp) = project.state_document().unwrap().unwrap();
    assert_eq!(read.variables.len(), 1);
    let schema = project.state_schema().unwrap();
    assert_eq!(schema.get(&Name::new("trusted").unwrap()).unwrap().ty, VarType::Bool);

    // The stamp is the precondition, and it travels with the read rather than
    // living in the index — a local cache must never own the thing that stops
    // two writers clobbering each other.
    fs::write(project.root().join("narrative/state.yaml"), "schema_version: 1\nvariables: []\n")
        .unwrap();
    let outcome = project.save_state(&document, Some(&stamp)).unwrap();
    assert!(matches!(outcome, SourceSave::Conflict { .. }), "got {outcome:?}");
}

#[test]
fn deleting_a_scene_removes_its_file_and_leaves_the_rest_alone() {
    let (_dir, mut project) = project();
    let doomed = project.create_scene("Kiln Interrogation").unwrap();
    let kept = project.create_scene("The Long Question").unwrap();

    project.delete_scene(doomed.scene.id).unwrap();
    assert!(!project.root().join(&doomed.rel).exists());
    assert!(project.root().join(&kept.rel).is_file());
    assert_eq!(project.scene_catalog().unwrap().scenes.len(), 1);
}

#[test]
fn a_read_only_share_refuses_a_scene_save_rather_than_failing_half_way() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let (dir, mut project) = project();
        let mut file = project.create_scene("Kiln Interrogation").unwrap();
        project.save_scene(&mut file).unwrap();
        let root = project.root().to_path_buf();
        drop(project);

        // The staging directory as well as the root: `is_writable` probes
        // `.wobu/tmp`, which is where every write actually lands first, so a
        // share that is read-only only at the top is not the case under test.
        let staging = root.join(".wobu").join("tmp");
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o555)).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();
        // Running as root ignores the mode bits, and a test that silently
        // proves nothing is worse than one that is skipped out loud.
        let enforced = fs::write(staging.join("probe"), b"").is_err();
        if enforced {
            let mut project = Project::open_at_index(&root, &dir.path().join("ro.sqlite")).unwrap();
            assert!(project.is_read_only());
            // Reading still works: the folder is the truth and it is readable.
            assert_eq!(project.scene_catalog().unwrap().scenes.len(), 1);
            assert!(project.save_scene(&mut file).is_err());
        }
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn a_narrative_conflict_reaches_the_card_and_can_be_resolved() {
    // Never-merge is only worth anything if the losing version is reachable
    // from the UI. A sibling nobody can see is a sibling nobody resolves, and
    // eventually somebody deletes the folder full of them.
    let (_dir, mut project) = project();
    let mut mine = project.create_scene("Kiln Interrogation").unwrap();
    mine.scene = kiln_scene();
    project.save_scene(&mut mine).unwrap();

    let path = project.root().join(&mine.rel);
    let mut theirs = mine.scene.clone();
    theirs.summary = "Nadia's summary".into();
    fs::write(&path, SceneDocument::new(theirs).to_yaml().unwrap()).unwrap();
    mine.scene.summary = "My summary".into();
    project.save_scene(&mut mine).unwrap();

    let cards = project.conflicts().unwrap();
    assert_eq!(cards.len(), 1, "the sibling should be on a card");
    let card = &cards[0];
    assert_eq!(card.node_rel_path, mine.rel);
    // No node id — scenes are not in the index — but a name a person can read.
    assert_eq!(card.node_id, None);
    assert_eq!(card.node_name.as_deref(), Some("Kiln Interrogation"));
    assert!(card.parked.contains("My summary"));

    let resolved = project
        .resolve_conflict(&card.rel_path, wobu_store::Keep::Parked, &card.current_hash)
        .unwrap();
    assert!(matches!(resolved, wobu_store::Resolved::Done), "got {resolved:?}");
    assert_eq!(project.load_scene(mine.scene.id).unwrap().scene.summary, "My summary");
    assert!(project.conflicts().unwrap().is_empty());
    // And resolving a scene must not put it in the navigator's broken-file
    // list: a YAML document is not a node that failed to parse.
    assert!(project.corrupt_files().unwrap().is_empty());
}
