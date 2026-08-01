//! Two people editing one world at once.
//!
//! M2 exists because multi-user file corruption is the one class of bug that
//! destroys trust permanently: it is silent, it is discovered days later, and
//! by then the good copy is gone. The unit tests in `atomic.rs` cover
//! `guarded_write` in isolation; these drive the whole path a real save takes —
//! `Project::save_node` → index stamp → guarded write → conflict sibling →
//! index bookkeeping — over a real folder on a real filesystem.
//!
//! **How the second writer is simulated.** Nadia is a different machine, so she
//! appears to us the only way another machine ever can: as bytes changing under
//! us between the moment we loaded a file and the moment we save it. Writing
//! her version directly to disk is not a shortcut around the API — it *is* what
//! a share delivers. Driving her through a second `Project` would be less
//! faithful, not more, because both instances would share one local SQLite
//! index and so could never disagree about the state of the folder, which is
//! the entire thing under test.
//!
//! Every assertion here is a variation on one question: after the dust settles,
//! is anybody's text gone?

use std::fs;
use std::path::{Path, PathBuf};

use wobu_core::{Node, NodeKind};
use wobu_store::{Project, SaveOutcome};

/// A project with one character in it, and the path to that character's file.
fn world() -> (tempfile::TempDir, Project, Node, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let node = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    let path = node_path(&project, &node);
    assert!(path.is_file(), "the create should have written a file");
    (dir, project, node, path)
}

fn node_path(project: &Project, node: &Node) -> PathBuf {
    let rel = project.index().rel_path_of(node.id).unwrap().expect("indexed");
    wobu_store::paths::from_rel_string(project.root(), &rel)
}

/// Files sitting next to `path` that `guarded_write` parked there.
fn conflict_siblings(path: &Path) -> Vec<PathBuf> {
    let dir = path.parent().unwrap();
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().contains(".conflict-"))
        .collect();
    out.sort();
    out
}

/// Nadia, on another machine, saves her version of the same file.
fn nadia_writes(path: &Path, body: &str) {
    let original = fs::read_to_string(path).unwrap();
    // Keep the frontmatter so the result is still a parseable node; only the
    // prose differs. A conflict over an unreadable file would be a different
    // test.
    let (frontmatter, _) = original.split_once("\n---\n").unwrap();
    fs::write(path, format!("{frontmatter}\n---\n\n## Notes\n\n{body}\n")).unwrap();
}

#[test]
fn the_second_writer_never_overwrites_the_first() {
    let (_dir, mut project, node, path) = world();

    // Jake loads the node. Nadia saves hers while he is typing.
    let mut jakes = project.get_node(node.id).unwrap();
    nadia_writes(&path, "nadia got here first");

    jakes.notes_raw = "jake's paragraph".into();
    let outcome = project.save_node(jakes).unwrap();

    let SaveOutcome::Conflict { conflict_path } = outcome else {
        panic!("expected a conflict, got a clean save — Nadia's text would be gone")
    };

    let on_disk = fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("nadia got here first"), "the winner's text must survive");
    assert!(!on_disk.contains("jake's paragraph"), "the loser must not have clobbered it");

    let siblings = conflict_siblings(&path);
    assert_eq!(siblings.len(), 1, "exactly one sibling, got {siblings:?}");
    assert!(
        fs::read_to_string(&siblings[0]).unwrap().contains("jake's paragraph"),
        "the loser's text has to be somewhere, or the conflict destroyed it"
    );

    // The reported path is project-relative and points at the sibling that
    // actually exists — the UI uses it to offer "open both".
    let reported = wobu_store::paths::from_rel_string(project.root(), &conflict_path);
    assert!(reported.is_file(), "{conflict_path} does not exist");
    assert_eq!(reported, siblings[0]);
}

#[test]
fn saving_identical_bytes_is_not_a_conflict() {
    // Two people independently reaching the same text lose nothing, so a
    // conflict card would be pure noise — and noise here is expensive, because
    // it teaches people to dismiss the card that one day matters.
    let (_dir, mut project, node, path) = world();

    // Jake has the node open. Note the ordering: `get_node` reads from disk, so
    // he has to load *before* Nadia saves or he simply inherits her copy and
    // there is nothing to disagree about.
    let mut jakes = project.get_node(node.id).unwrap();

    // Nadia makes the same edit and saves first. Her file carries *her*
    // `updated_at`, so the bytes are not identical to what Jake will render —
    // which is the whole difficulty. Every save re-stamps the timestamp, so
    // byte equality between two independent writers is unreachable in practice,
    // and a byte-level check alone would park a conflict sibling whose only
    // difference from the winner is a clock reading.
    let mut nadias = project.get_node(node.id).unwrap();
    nadias.notes_raw = "we agree".into();
    nadias.updated_at += chrono::Duration::seconds(5);
    fs::write(&path, wobu_store::markdown::to_markdown(&nadias).unwrap()).unwrap();

    jakes.notes_raw = "we agree".into();
    let outcome = project.save_node(jakes).unwrap();
    assert!(
        matches!(outcome, SaveOutcome::Saved(_)),
        "agreeing with the other writer must not be a conflict"
    );
    assert!(conflict_siblings(&path).is_empty(), "no sibling for an agreement");
    assert!(fs::read_to_string(&path).unwrap().contains("we agree"));
}

#[test]
fn agreeing_on_the_words_still_notices_a_real_difference() {
    // The guard above must not become "close enough is the same". A genuine
    // difference alongside the agreement is still a conflict.
    let (_dir, mut project, node, path) = world();

    let mut jakes = project.get_node(node.id).unwrap();

    let mut nadias = project.get_node(node.id).unwrap();
    nadias.notes_raw = "we agree".into();
    nadias.summary = "but not about this".into();
    nadias.updated_at += chrono::Duration::seconds(5);
    fs::write(&path, wobu_store::markdown::to_markdown(&nadias).unwrap()).unwrap();

    jakes.notes_raw = "we agree".into();

    assert!(
        matches!(project.save_node(jakes).unwrap(), SaveOutcome::Conflict { .. }),
        "a differing summary is a real disagreement"
    );
}

#[test]
fn a_move_that_loses_the_race_leaves_the_file_alone() {
    // A move is a save of the same file with a different parent, so it takes
    // the same guarded path — and must fail the same way rather than dragging
    // the file out from under whoever won.
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let parent = project.create_node(NodeKind::Setting, "The Ember Coast", None).unwrap();
    let child = project.create_node(NodeKind::Setting, "Cinder Bay", None).unwrap();
    let path = node_path(&project, &child);

    // Jake's index still holds the stamp from before Nadia's edit.
    nadia_writes(&path, "nadia rewrote this while jake dragged it");

    let err = project.move_node(child.id, Some(parent.id)).unwrap_err();
    let on_disk = fs::read_to_string(&path).unwrap();

    assert!(on_disk.contains("nadia rewrote this"), "a losing move must not clobber: {err}");
    assert_eq!(conflict_siblings(&path).len(), 1, "the move's version is kept beside it");
}

#[test]
fn resolving_a_conflict_can_itself_lose_a_race() {
    // The nasty one. Two conflicts in a row is where a naive implementation
    // starts overwriting, because it treats "I already handled a conflict" as
    // permission to force the next write.
    let (_dir, mut project, node, path) = world();

    let mut jakes = project.get_node(node.id).unwrap();
    nadia_writes(&path, "nadia one");
    jakes.notes_raw = "jake one".into();
    assert!(matches!(project.save_node(jakes).unwrap(), SaveOutcome::Conflict { .. }));

    // Jake decides his version wins and saves again — but a third writer has
    // moved the file again in the meantime.
    let mut resolved = project.get_node(node.id).unwrap();
    nadia_writes(&path, "priya three");
    resolved.notes_raw = "jake resolved".into();

    let outcome = project.save_node(resolved).unwrap();
    assert!(matches!(outcome, SaveOutcome::Conflict { .. }), "the second race must also lose");

    let on_disk = fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("priya three"), "the newest winner survives");

    let siblings = conflict_siblings(&path);
    assert_eq!(siblings.len(), 2, "both losing versions are kept, got {siblings:?}");
    let bodies: Vec<String> = siblings.iter().map(|p| fs::read_to_string(p).unwrap()).collect();
    assert!(bodies.iter().any(|b| b.contains("jake one")), "first loser lost their text");
    assert!(bodies.iter().any(|b| b.contains("jake resolved")), "second loser lost their text");
}

#[test]
fn a_rescan_after_a_losing_save_reports_the_winner() {
    // The index is what the UI reads. If a conflict leaves it believing our
    // version landed, the app shows text that is not on disk — the user only
    // finds out when they reopen the project, by which time they have built on
    // a version nobody else has.
    let (_dir, mut project, node, path) = world();

    let mut jakes = project.get_node(node.id).unwrap();
    nadia_writes(&path, "nadia wins this one");
    jakes.notes_raw = "jake's lost paragraph".into();
    assert!(matches!(project.save_node(jakes).unwrap(), SaveOutcome::Conflict { .. }));

    project.rescan().unwrap();
    let reread = project.get_node(node.id).unwrap();
    assert!(
        reread.notes_raw.contains("nadia wins this one"),
        "the index must agree with the disk, found: {:?}",
        reread.notes_raw
    );
}

#[test]
fn a_save_that_wins_survives_the_next_rescan() {
    // The other direction, so the test above cannot pass by always reporting
    // whatever is on disk regardless of who wrote it.
    let (_dir, mut project, node, path) = world();

    let mut jakes = project.get_node(node.id).unwrap();
    jakes.notes_raw = "jake's kept paragraph".into();
    assert!(matches!(project.save_node(jakes).unwrap(), SaveOutcome::Saved(_)));

    project.rescan().unwrap();
    assert!(project.get_node(node.id).unwrap().notes_raw.contains("jake's kept paragraph"));
    assert!(conflict_siblings(&path).is_empty());
}

#[test]
fn staging_is_inside_the_project_and_not_the_os_temp_dir() {
    // The rename that makes a write atomic is only atomic within one
    // filesystem. Staging in the OS temp dir would silently degrade it into a
    // copy — which can be interrupted halfway, which is the exact failure this
    // whole module exists to prevent. Nothing in the type system stops someone
    // "tidying" this later, so it is pinned here.
    let (_dir, mut project, node, path) = world();

    let mut n = project.get_node(node.id).unwrap();
    n.notes_raw = "anything".into();
    project.save_node(n).unwrap();

    let tmp = project.root().join(".wobu").join("tmp");
    assert!(tmp.is_dir(), "staging directory should exist after a write");
    assert!(tmp.starts_with(project.root()), "staging escaped the project folder");
    // Note there is deliberately no `!tmp.starts_with(env::temp_dir())` check:
    // this test's own project lives under the OS temp dir, so that assertion
    // would be testing the harness rather than the code, and would pass or fail
    // for reasons having nothing to do with staging.

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // The property that actually matters, rather than a proxy for it: same
        // device means `rename` is atomic. Across devices it silently becomes a
        // copy, which can be interrupted halfway.
        let tmp_dev = fs::metadata(&tmp).unwrap().dev();
        let target_dev = fs::metadata(path.parent().unwrap()).unwrap().dev();
        assert_eq!(tmp_dev, target_dev, "staging is on a different filesystem than the target");
    }
    let _ = path;
}

#[test]
fn staging_is_empty_once_the_write_lands() {
    let (_dir, mut project, node, _path) = world();
    let mut n = project.get_node(node.id).unwrap();
    n.notes_raw = "anything".into();
    project.save_node(n).unwrap();

    let left: Vec<_> = fs::read_dir(project.root().join(".wobu/tmp")).unwrap().flatten().collect();
    assert!(left.is_empty(), "a completed write left staging behind: {left:?}");
}

/// A process killed between staging and renaming.
///
/// The target was never touched — that is the guarantee — but a full copy of a
/// node file is left in `.wobu/tmp`. Nothing ever reads it, they accumulate,
/// and on a synced share they replicate to everyone.
mod interrupted_writes {
    use super::*;

    /// Backdate a file, since the sweep decides by age and no test is going to
    /// wait an hour. `File::set_times` keeps this dependency-free.
    fn age_file(path: &Path, age: std::time::Duration) {
        let when = std::time::SystemTime::now() - age;
        let f = fs::File::options().write(true).open(path).unwrap();
        f.set_times(fs::FileTimes::new().set_modified(when)).unwrap();
    }

    fn stranded_part(project: &Project, age: std::time::Duration) -> PathBuf {
        let tmp = project.root().join(".wobu").join("tmp");
        fs::create_dir_all(&tmp).unwrap();
        let part = tmp.join(format!("{}.part", wobu_core::new_id()));
        fs::write(&part, "half a node file").unwrap();
        age_file(&part, age);
        part
    }

    #[test]
    fn the_target_is_untouched_and_the_orphan_is_swept_on_next_open() {
        let (dir, project, node, path) = world();
        let before = fs::read_to_string(&path).unwrap();
        let root = project.root().to_path_buf();

        let old = stranded_part(&project, std::time::Duration::from_secs(48 * 60 * 60));
        drop(project);

        let reopened = Project::open(&root).unwrap();

        assert!(!old.exists(), "an abandoned staging file survived a reopen");
        assert_eq!(fs::read_to_string(&path).unwrap(), before, "the target was modified");
        assert!(reopened.get_node(node.id).is_ok(), "the node should still be readable");
        drop(dir);
    }

    #[test]
    fn a_staging_file_that_might_still_be_in_flight_is_left_alone() {
        // On a share the `.part` may belong to another Wobu that is mid-write.
        // Deleting it makes their rename fail — so this sweep destroys the very
        // save it exists to protect if it is too eager.
        let (dir, project, _node, _path) = world();
        let root = project.root().to_path_buf();

        let fresh = stranded_part(&project, std::time::Duration::from_secs(5));
        drop(project);

        Project::open(&root).unwrap();
        assert!(fresh.exists(), "swept a staging file that could still be in use");
        drop(dir);
    }

    #[test]
    fn the_sweep_only_touches_part_files() {
        let (dir, project, _node, _path) = world();
        let root = project.root().to_path_buf();

        let tmp = root.join(".wobu").join("tmp");
        fs::create_dir_all(&tmp).unwrap();
        let bystander = tmp.join("something-else.txt");
        fs::write(&bystander, "not ours").unwrap();
        age_file(&bystander, std::time::Duration::from_secs(48 * 60 * 60));
        drop(project);

        Project::open(&root).unwrap();
        assert!(bystander.exists(), "the sweep deleted a file it does not own");
        drop(dir);
    }
}
