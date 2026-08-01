//! What may and may not happen to a conflict sibling.
//!
//! `multi_writer.rs` proves the siblings get *created* and that nobody's text
//! is lost at the moment of the race. This file covers the hours afterwards,
//! while one is sitting in the folder waiting for a person: every routine
//! thing the app does to a project folder — reopening it, rescanning it,
//! rebuilding the index, sweeping abandoned staging files — has to walk past
//! it without touching it.
//!
//! The rule the whole issue turns on, from #18: *no code path may ever delete
//! or overwrite a conflict sibling without the user asking.* A sibling is by
//! construction the last surviving copy of a paragraph somebody typed, so a
//! sweep that reaps one is indistinguishable, from the outside, from the app
//! quietly eating an afternoon's work.

use std::fs;
use std::path::{Path, PathBuf};

use wobu_core::{Node, NodeKind};
use wobu_store::conflict::{Keep, Resolved};
use wobu_store::{Project, SaveOutcome};

/// A project with one character, and a conflict sibling beside that
/// character's file: Nadia saved while Jake was typing, and Jake lost.
fn world_with_a_conflict() -> (tempfile::TempDir, Project, Node, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let node = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    let path = node_path(&project, &node);

    let mut jakes = project.get_node(node.id).unwrap();
    nadia_writes(&path, "nadia got here first");
    jakes.notes_raw = "jake's only copy of this paragraph".into();

    let SaveOutcome::Conflict { conflict_path } = project.save_node(jakes).unwrap() else {
        panic!("the fixture needs a conflict and did not get one")
    };
    let sibling = wobu_store::paths::from_rel_string(project.root(), &conflict_path);
    assert!(sibling.is_file());
    (dir, project, node, path, sibling)
}

fn node_path(project: &Project, node: &Node) -> PathBuf {
    let rel = project.index().rel_path_of(node.id).unwrap().expect("indexed");
    wobu_store::paths::from_rel_string(project.root(), &rel)
}

/// Somebody on another machine saves their version over the node file, keeping
/// the frontmatter so the result still parses.
fn nadia_writes(path: &Path, body: &str) {
    let original = fs::read_to_string(path).unwrap();
    let (frontmatter, _) = original.split_once("\n---\n").unwrap();
    fs::write(path, format!("{frontmatter}\n---\n\n## Notes\n\n{body}\n")).unwrap();
}

/// Backdate a file, since nothing in a test is going to wait an hour for the
/// staging sweep's grace period.
fn age_file(path: &Path, age: std::time::Duration) {
    let when = std::time::SystemTime::now() - age;
    let f = fs::File::options().write(true).open(path).unwrap();
    f.set_times(fs::FileTimes::new().set_modified(when)).unwrap();
}

fn rel_of(project: &Project, path: &Path) -> String {
    wobu_store::paths::to_rel_string(path.strip_prefix(project.root()).unwrap())
}

/* ── the non-negotiable ───────────────────────────────────────────────────── */

#[test]
fn a_conflict_sibling_survives_a_rescan_an_index_rebuild_and_a_staging_sweep() {
    // The single most important assertion in #18. Every one of these operations
    // walks the project folder, and two of them delete things — `sweep_staging`
    // reaps by age, and `rebuild_index` throws away and rewrites state. None of
    // them may touch a sibling, ever, for any reason.
    let (dir, project, _node, path, sibling) = world_with_a_conflict();
    let root = project.root().to_path_buf();
    let bytes = fs::read(&sibling).unwrap();
    let winner = fs::read(&path).unwrap();

    // Old enough that anything deciding by age would have taken it. The staging
    // sweep's grace is an hour; this is two days.
    age_file(&sibling, std::time::Duration::from_secs(48 * 60 * 60));

    let mut project = project;
    project.rescan().unwrap();
    assert_eq!(fs::read(&sibling).unwrap(), bytes, "a rescan modified the sibling");

    project.rebuild_index().unwrap();
    assert_eq!(fs::read(&sibling).unwrap(), bytes, "an index rebuild modified the sibling");

    project.reconcile().unwrap();
    assert_eq!(fs::read(&sibling).unwrap(), bytes, "a reconcile modified the sibling");

    // Reopening runs `sweep_staging` before anything else reads the folder, and
    // then a full scan or a reconcile depending on the index. Both paths.
    drop(project);
    let reopened = Project::open(&root).unwrap();
    assert!(sibling.is_file(), "reopening the project deleted the sibling");
    assert_eq!(fs::read(&sibling).unwrap(), bytes, "reopening the project modified the sibling");

    fs::remove_file(wobu_store::paths::index_path(&reopened.id())).ok();
    drop(reopened);
    let cold = Project::open(&root).unwrap();
    assert_eq!(
        fs::read(&sibling).unwrap(),
        bytes,
        "a cold open — index thrown away, full scan — modified the sibling",
    );

    assert_eq!(fs::read(&path).unwrap(), winner, "the winner's file was modified too");
    drop(cold);
    drop(dir);
}

#[test]
fn a_conflict_sibling_is_never_indexed_as_a_node() {
    // The other half of surviving: it must not become a *node*, or it appears
    // in the navigator as a ghost duplicate — and, far worse, becomes a save
    // target, so that editing the ghost would start a second conflict.
    let (_dir, mut project, node, _path, sibling) = world_with_a_conflict();

    project.rescan().unwrap();

    let named: Vec<_> = project
        .list_nodes()
        .unwrap()
        .into_iter()
        .filter(|n| n.name == "Kael Vantris")
        .collect();
    assert_eq!(named.len(), 1, "the sibling was scanned as a node: {named:?}");
    assert_eq!(named[0].id, node.id);

    // And it is not recorded as broken either. A sibling is a perfectly good
    // file; a "1 file could not be read" banner for one would be a lie the user
    // cannot act on.
    let rel = rel_of(&project, &sibling);
    assert!(
        project.corrupt_files().unwrap().iter().all(|c| c.rel_path != rel),
        "the sibling was reported as a corrupt file",
    );
}

#[test]
fn deleting_the_node_leaves_its_conflict_siblings_alone() {
    // Tempting to tidy: the entity is gone, so the siblings look like litter.
    // They are not. The user deleted a node; they did not say what should
    // happen to a version of it they have never been shown, and that version
    // may be the only copy of something they want back.
    let (_dir, mut project, node, _path, sibling) = world_with_a_conflict();
    let bytes = fs::read(&sibling).unwrap();

    project.delete_node(node.id).unwrap();

    assert!(sibling.is_file(), "deleting the node took the conflict sibling with it");
    assert_eq!(fs::read(&sibling).unwrap(), bytes);
}

#[test]
fn a_further_save_to_the_node_does_not_disturb_an_existing_sibling() {
    // Ordinary work carrying on around an unresolved conflict.
    let (_dir, mut project, node, path, sibling) = world_with_a_conflict();
    let bytes = fs::read(&sibling).unwrap();

    let mut nadia = project.get_node(node.id).unwrap();
    nadia.notes_raw = "nadia keeps working".into();
    assert!(matches!(project.save_node(nadia).unwrap(), SaveOutcome::Saved(_)));

    assert_eq!(fs::read(&sibling).unwrap(), bytes, "a later save overwrote the sibling");
    assert!(fs::read_to_string(&path).unwrap().contains("nadia keeps working"));
}

/* ── listing ──────────────────────────────────────────────────────────────── */

#[test]
fn the_conflict_list_carries_both_versions_and_says_who_lost() {
    let (_dir, project, node, _path, sibling) = world_with_a_conflict();

    let conflicts = project.conflicts().unwrap();
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    let c = &conflicts[0];

    assert_eq!(c.rel_path, rel_of(&project, &sibling));
    assert_eq!(c.node_id, Some(node.id), "the card has to name the entity, not a path");
    assert_eq!(c.node_name.as_deref(), Some("Kael Vantris"));
    assert!(c.parked.contains("jake's only copy of this paragraph"));
    assert!(c.current.contains("nadia got here first"));
    assert!(c.saved_at.is_some(), "the card shows a time and needs one");
    assert!(c.user.is_some(), "the card shows a name and needs one");
    // Written by this process, so the session user is the one on the filename.
    assert!(c.mine, "a sibling this session parked should read as ours");
}

#[test]
fn a_folder_with_no_conflicts_lists_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    assert!(project.conflicts().unwrap().is_empty());
}

#[test]
fn two_conflicts_on_one_node_are_both_listed_newest_first() {
    // Losing twice is ordinary — an Enhance that finished late, then a manual
    // save. Collapsing them to one card would silently pick a version.
    let (_dir, mut project, node, path, first) = world_with_a_conflict();

    let mut jakes = project.get_node(node.id).unwrap();
    nadia_writes(&path, "priya three");
    jakes.notes_raw = "jake's second lost paragraph".into();
    let SaveOutcome::Conflict { conflict_path } = project.save_node(jakes).unwrap() else {
        panic!("expected a second conflict")
    };

    let conflicts = project.conflicts().unwrap();
    assert_eq!(conflicts.len(), 2, "{conflicts:?}");
    let paths: Vec<&str> = conflicts.iter().map(|c| c.rel_path.as_str()).collect();
    assert!(paths.contains(&rel_of(&project, &first).as_str()));
    assert!(paths.contains(&conflict_path.as_str()));
    // Both diffs are against the same current file, so both cards tell the
    // truth about what they would be replacing.
    assert!(conflicts.iter().all(|c| c.current.contains("priya three")));
}

#[test]
fn a_sibling_whose_node_file_was_deleted_is_still_listed() {
    // At this point the sibling is the only copy of anything that node ever
    // was. Hiding it because there is nothing to diff against would be the
    // exact moment the text becomes unreachable.
    let (_dir, mut project, _node, path, sibling) = world_with_a_conflict();
    fs::remove_file(&path).unwrap();
    project.reconcile().unwrap();

    let conflicts = project.conflicts().unwrap();
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert!(conflicts[0].current.is_empty());
    assert!(conflicts[0].parked.contains("jake's only copy of this paragraph"));
    assert!(sibling.is_file());
}

/* ── resolution ───────────────────────────────────────────────────────────── */

#[test]
fn keeping_the_current_version_removes_only_the_sibling() {
    let (_dir, mut project, _node, path, sibling) = world_with_a_conflict();
    let c = project.conflicts().unwrap().pop().unwrap();
    let winner = fs::read(&path).unwrap();

    let outcome =
        project.resolve_conflict(&c.rel_path, Keep::Current, &c.current_hash).unwrap();

    assert_eq!(outcome, Resolved::Done);
    assert!(!sibling.exists(), "the rejected version should be gone");
    assert_eq!(fs::read(&path).unwrap(), winner, "the kept version was rewritten for no reason");
    assert!(project.conflicts().unwrap().is_empty());
}

#[test]
fn keeping_the_parked_version_writes_it_over_the_node_and_then_removes_it() {
    let (_dir, mut project, node, path, sibling) = world_with_a_conflict();
    let c = project.conflicts().unwrap().pop().unwrap();

    let outcome = project.resolve_conflict(&c.rel_path, Keep::Parked, &c.current_hash).unwrap();

    assert_eq!(outcome, Resolved::Done);
    assert!(!sibling.exists());
    let on_disk = fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("jake's only copy of this paragraph"), "{on_disk}");
    // The index has to agree with the disk immediately, or the editor keeps
    // showing the version the user just rejected until the next reconcile.
    assert!(project.get_node(node.id).unwrap().notes_raw.contains("jake's only copy"));
    assert!(project.conflicts().unwrap().is_empty());
}

#[test]
fn resolving_into_a_third_writer_refuses_and_deletes_nothing() {
    // The three-way case, and the one that silently loses data if it is wrong.
    //
    // Jake opens the card and reads a diff of his version against Nadia's.
    // While he is reading, Priya saves a third version over the node file. The
    // question Jake answers — "yours or Nadia's?" — is no longer the question
    // on disk, so whichever button he presses, applying it would discard text
    // he was never shown. Both choices must refuse, and refuse without touching
    // either file.
    let (_dir, mut project, _node, path, sibling) = world_with_a_conflict();
    let c = project.conflicts().unwrap().pop().unwrap();
    let parked = fs::read(&sibling).unwrap();

    nadia_writes(&path, "priya three, while jake was reading the diff");
    let priyas = fs::read(&path).unwrap();

    for keep in [Keep::Current, Keep::Parked] {
        let outcome = project.resolve_conflict(&c.rel_path, keep, &c.current_hash).unwrap();
        assert_eq!(outcome, Resolved::Stale, "{keep:?} was applied to a version nobody read");
        assert_eq!(fs::read(&sibling).unwrap(), parked, "{keep:?} disturbed the parked version");
        assert_eq!(fs::read(&path).unwrap(), priyas, "{keep:?} clobbered the third writer");
    }

    // And the card comes back with the new state, so the decision can be made
    // again against what is actually there.
    let again = project.conflicts().unwrap();
    assert_eq!(again.len(), 1);
    assert!(again[0].current.contains("priya three"));
    assert_ne!(again[0].current_hash, c.current_hash);

    // Answered against the fresh diff, it lands.
    let fresh = &again[0];
    assert_eq!(
        project.resolve_conflict(&fresh.rel_path, Keep::Parked, &fresh.current_hash).unwrap(),
        Resolved::Done,
    );
    assert!(fs::read_to_string(&path).unwrap().contains("jake's only copy of this paragraph"));
}

#[test]
fn resolving_one_of_two_conflicts_leaves_the_other_alone() {
    // Resolution is per sibling, not per node. Answering one card must not
    // sweep up a version the user has not looked at yet.
    let (_dir, mut project, node, path, first) = world_with_a_conflict();

    let mut jakes = project.get_node(node.id).unwrap();
    nadia_writes(&path, "priya three");
    jakes.notes_raw = "the second lost paragraph".into();
    let SaveOutcome::Conflict { conflict_path: second } = project.save_node(jakes).unwrap() else {
        panic!("expected a second conflict")
    };
    let second = wobu_store::paths::from_rel_string(project.root(), &second);
    let untouched = fs::read(&second).unwrap();

    let first_rel = rel_of(&project, &first);
    let hash = project
        .conflicts()
        .unwrap()
        .into_iter()
        .find(|c| c.rel_path == first_rel)
        .unwrap()
        .current_hash;
    assert_eq!(
        project.resolve_conflict(&first_rel, Keep::Current, &hash).unwrap(),
        Resolved::Done,
    );

    assert!(!first.exists());
    assert_eq!(fs::read(&second).unwrap(), untouched, "the other sibling was swept up");
    assert_eq!(project.conflicts().unwrap().len(), 1);
}

#[test]
fn a_path_that_is_not_a_conflict_sibling_is_refused() {
    // `resolve_conflict` deletes a file and takes its path over a bridge, so
    // this is the guard that stops a wrong argument deleting a node. Without
    // it, "keep theirs" on a mistyped path is a silent `rm`.
    let (_dir, mut project, node, path, _sibling) = world_with_a_conflict();
    let node_rel = project.index().rel_path_of(node.id).unwrap().unwrap();
    let bytes = fs::read(&path).unwrap();

    assert!(project.resolve_conflict(&node_rel, Keep::Current, "").is_err());
    assert!(project.resolve_conflict("project.json", Keep::Current, "").is_err());
    assert!(
        project
            .resolve_conflict("nodes/character/nothing.conflict-jake-20260731T142211Z.md",
                Keep::Current, "")
            .is_err(),
        "a sibling that does not exist is not a resolution",
    );

    assert_eq!(fs::read(&path).unwrap(), bytes, "the node file was deleted or rewritten");
    assert!(project.root().join("project.json").is_file());
}

#[test]
fn resolving_while_the_share_is_away_is_refused_rather_than_half_done() {
    // Checked before anything is read or written, because the failure mode
    // otherwise is a write that lands on the empty mountpoint and a delete that
    // does not — the winner invisible to everyone and the loser gone.
    let (dir, mut project, _node, _path, sibling) = world_with_a_conflict();
    let c = project.conflicts().unwrap().pop().unwrap();

    // What an unmount leaves behind: the mountpoint is still a directory.
    let stashed = dir.path().join("stash");
    fs::rename(project.root(), &stashed).unwrap();
    fs::create_dir_all(project.root()).unwrap();

    assert!(project.resolve_conflict(&c.rel_path, Keep::Parked, &c.current_hash).is_err());
    assert!(project.resolve_conflict(&c.rel_path, Keep::Current, &c.current_hash).is_err());

    fs::remove_dir_all(project.root()).unwrap();
    fs::rename(&stashed, project.root()).unwrap();
    assert!(sibling.is_file(), "the sibling did not survive the share going away");
}

/* ── who a sibling says wrote it (#76) ────────────────────────────────────── */

#[test]
fn a_sibling_is_stamped_with_this_installations_peer_alias_and_not_with_a_login() {
    // The whole of #76 as it reaches a folder. The name used to come from
    // `$USER` with a fallback to the literal string `user`, which made two
    // collaborators on default installs into the same person and let anybody be
    // anybody by exporting a variable. It now comes from an alias for this
    // machine's ed25519 key, which nothing on another machine can produce.
    let (_dir, project, _node, _path, sibling) = world_with_a_conflict();
    let name = sibling.file_name().unwrap().to_string_lossy().into_owned();

    let alias = wobu_store::peer::alias();
    let parsed = wobu_store::conflict::parse(&name).expect("a sibling the app just wrote");
    assert_eq!(parsed.peer.as_deref(), Some(alias), "{name}");
    assert!(parsed.saved_at.is_some(), "{name}");

    // And the card agrees with the filename, which is what makes "keep mine"
    // the right words rather than a guess.
    let listed = project.conflicts().unwrap();
    let card = listed.iter().find(|c| c.rel_path.ends_with(&name)).expect("the sibling is listed");
    assert_eq!(card.user.as_deref(), Some(alias));
    assert!(card.mine, "we wrote it, so the card must offer to keep ours");

    // The environment is no longer an input. Asserted rather than argued
    // because the failure would be silent: a build that still read `$USER`
    // would produce a name that parses, lists and looks entirely correct.
    assert!(!name.contains(".conflict-user-"), "the `$USER` fallback is back: {name}");
    if let Ok(login) = std::env::var("USER")
        && login != alias
    {
        assert!(!name.contains(&format!(".conflict-{login}-")), "a login named the sibling: {name}");
    }
}
