//! What makes Wobu bearable on a NAS.
//!
//! Reading several hundred small Markdown files over SMB is genuinely slow —
//! each one is a round trip, and a folder that opens instantly on an SSD can
//! take a minute on a share. Everything here is about the two properties that
//! decide whether that is a one-off cost or a permanent tax.
//!
//! **These are not timing tests.** A stopwatch on a local SSD proves nothing
//! about SMB: the numbers would be microseconds either way, and the test would
//! pass just as happily if the code re-read every file on every poll. What is
//! measured instead is *whether the files are opened at all*, using the
//! operating system as the instrument — the node files are made unreadable, so
//! any code that tries to read one fails loudly. A test that cannot be
//! satisfied by being fast is a test that still means something on hardware
//! nobody ran it on.

use std::fs;
use std::path::{Path, PathBuf};

use wobu_core::NodeKind;
use wobu_store::{Cancel, Project, ScanProgress};

/// A project with `n` characters in it. Returns the *project* root, which is a
/// `.wobu` folder inside the temp dir rather than the temp dir itself.
fn world(n: usize) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    for i in 0..n {
        project.create_node(NodeKind::Character, &format!("Character {i:04}"), None).unwrap();
    }
    let root = project.root().to_path_buf();
    (dir, root)
}

/// Throw away the local index so the next open takes the full-scan path.
///
/// This is what "first open on this machine" means — the expensive case, and
/// the only one that reports progress or can be cancelled.
fn make_cold(root: &Path) {
    let id = {
        let project = Project::open(root).unwrap();
        project.id()
    };
    fs::remove_file(wobu_store::paths::index_path(&id)).unwrap();
}

/// Make every node file unopenable while leaving the directory listable, so
/// `stat` still works and `read` does not.
fn seal(root: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut sealed = None;
        for entry in walkdir::WalkDir::new(root.join("nodes")).into_iter().flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("md") {
                fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o000)).unwrap();
                sealed = Some(entry.path().to_path_buf());
            }
        }
        // Running as root ignores the permission bits entirely, and a test that
        // silently proves nothing is worse than one that is skipped out loud.
        match sealed {
            Some(p) => fs::read(&p).is_err(),
            None => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        false
    }
}

fn unseal(root: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in walkdir::WalkDir::new(root.join("nodes")).into_iter().flatten() {
            if entry.path().is_file() {
                let _ = fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o644));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = root;
}

#[test]
fn reconcile_does_not_reopen_files_whose_stamp_has_not_moved() {
    // The property the whole NAS story rests on: after the first open, the
    // workspace renders from the local index and the folder is only touched for
    // files that actually moved. If this regresses, every five-second poll
    // becomes several hundred SMB round trips and the app feels broken on
    // exactly the setup it was designed for.
    let (_dir, root) = world(40);
    let mut project = Project::open(&root).unwrap();

    if !seal(&root) {
        eprintln!("skipping: cannot make files unreadable (running as root?)");
        return;
    }

    // Every file is now unreadable. A reconcile that only stats will not notice.
    let changed = project.reconcile().expect("reconcile read a file it did not need to");
    assert!(!changed, "nothing moved, so nothing should have been reported as changed");

    // And the world is still fully there, entirely from the index.
    assert_eq!(project.list_nodes().unwrap().len(), 42, "42 = 40 characters + 2 singletons");
    assert!(project.corrupt_files().unwrap().is_empty(), "no file should have been read at all");

    unseal(&root);
}

#[test]
fn reconcile_does_read_the_one_file_that_moved() {
    // The other half, so the test above cannot pass by never reading anything.
    let (_dir, root) = world(20);
    let mut project = Project::open(&root).unwrap();

    let target = project
        .index()
        .rel_path_of(project.list_nodes().unwrap()[0].id)
        .unwrap()
        .expect("indexed");
    let path = wobu_store::paths::from_rel_string(&root, &target);
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("## Notes", "## Notes\n\nedited elsewhere")).unwrap();

    assert!(project.reconcile().unwrap(), "an edited file should be noticed");
}

#[test]
fn a_first_open_reports_progress_that_reaches_the_end() {
    // A stalled mount and a large world look identical from outside; the only
    // thing that tells them apart is a number that advances. So it has to
    // actually advance, and it has to finish.
    let (_dir, root) = world(30);
    make_cold(&root);

    let mut seen: Vec<ScanProgress> = Vec::new();
    let project = Project::open_with(&root, &Cancel::new(), &mut |p| seen.push(p)).unwrap();

    assert!(seen.len() > 2, "expected progress to be reported, saw {}", seen.len());
    let last = seen.last().unwrap();
    assert_eq!(last.done, last.total, "progress must reach the end, ended at {last:?}");
    assert_eq!(last.percent(), 100);
    // Monotonic, because a bar that goes backwards reads as a bug.
    assert!(seen.windows(2).all(|w| w[0].done <= w[1].done), "progress went backwards");
    assert_eq!(project.list_nodes().unwrap().len(), 32);
}

#[test]
fn a_cancelled_open_stops_and_leaves_the_index_untouched() {
    // The failure this guards against is a cancel that half-finishes: the scan
    // clears the index, gets stopped, and leaves a world that is entirely
    // intact on disk looking empty in the app — which the next open would then
    // rebuild, slowly, over the same slow share.
    let (dir, root) = world(60);
    let before = Project::open(&root).unwrap().list_nodes().unwrap().len();
    make_cold(&root);

    let cancel = Cancel::new();
    let mut seen = 0usize;
    let result = Project::open_with(&root, &cancel, &mut |_| {
        seen += 1;
        // Stop it a little way in, the way a user watching a stalled count would.
        if seen == 5 {
            cancel.cancel();
        }
    });

    assert!(matches!(result, Err(wobu_store::Error::Cancelled)), "expected a clean cancellation");

    // Reopening finds the world exactly as it was.
    let reopened = Project::open(&root).unwrap();
    assert_eq!(reopened.list_nodes().unwrap().len(), before, "the index lost nodes to a cancel");
    drop(dir);
}

#[test]
fn cancelling_before_it_starts_stops_immediately() {
    let (dir, root) = world(5);
    make_cold(&root);

    let cancel = Cancel::new();
    cancel.cancel();
    let mut progress_events = 0;
    let result = Project::open_with(&root, &cancel, &mut |_| progress_events += 1);

    assert!(matches!(result, Err(wobu_store::Error::Cancelled)));
    // The initial 0-of-n is fine; reading any file is not.
    assert!(progress_events <= 1, "kept scanning after being cancelled up front");
    drop(dir);
}
