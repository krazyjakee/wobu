//! Presence across a shared folder, from the outside.
//!
//! The unit tests in `presence.rs` cover one session's own lifecycle. What is
//! exercised here is the part that only exists because there is more than one
//! machine: files written by somebody else's Wobu, on somebody else's clock,
//! read back through ours.
//!
//! Peer session files are written as literal JSON rather than by serialising a
//! `Session`. A round trip would agree with itself no matter what another
//! build actually writes into the folder, and the two things these tests need
//! to vary — the timestamp *inside* the file and the mtime the filesystem
//! reports *for* it — have to be set independently to mean anything.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{Duration as Offset, Utc};
use wobu_core::NodeKind;
use wobu_store::presence::{Presence, STALE_AFTER, peers_in};
use wobu_store::{Project, SaveOutcome};

/// A folder that looks like a project to everything presence touches.
fn folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("project.json"), "{}").unwrap();
    fs::create_dir_all(dir.path().join(".wobu/sessions")).unwrap();
    dir
}

fn sessions(root: &Path) -> PathBuf {
    root.join(".wobu").join("sessions")
}

/// Another machine's session file: `heartbeat_at` is what it claims about
/// itself, `written_ago` is what our filesystem will say about it.
fn plant(root: &Path, user: &str, heartbeat_at: &str, written_ago: Duration) -> PathBuf {
    let id = wobu_core::new_id();
    let path = sessions(root).join(format!("{id}.json"));
    fs::write(
        &path,
        format!(
            r#"{{
  "user": "{user}",
  "host": "nadia-mbp",
  "openedAt": "2026-07-31T09:00:00Z",
  "heartbeatAt": "{heartbeat_at}",
  "editing": []
}}"#
        ),
    )
    .unwrap();

    let f = fs::OpenOptions::new().write(true).open(&path).unwrap();
    f.set_modified(SystemTime::now() - written_ago).unwrap();
    path
}

fn stamp(offset: Offset) -> String {
    (Utc::now() + offset).to_rfc3339()
}

#[test]
fn a_session_whose_own_clock_is_an_hour_out_is_still_judged_live_by_its_mtime() {
    // The regression this exists for, and the reason the reaper never reads
    // `heartbeatAt`. Two desktops on a LAN sit minutes or hours apart all the
    // time — a wrong timezone, a VM that resumed, a machine that has never had
    // NTP. Against a sixty-second window that is not a rounding error: a peer
    // an hour behind would be reaped continuously while they sit there typing,
    // and a peer an hour ahead would never expire at all.
    let dir = folder();
    let behind = plant(dir.path(), "nadia", &stamp(-Offset::hours(1)), Duration::from_secs(1));
    let ahead = plant(dir.path(), "sam", &stamp(Offset::hours(1)), Duration::from_secs(1));

    let peers = peers_in(dir.path());
    let mut users: Vec<&str> = peers.iter().map(|p| p.user.as_str()).collect();
    users.sort_unstable();
    assert_eq!(users, ["nadia", "sam"], "a skewed clock decided who was alive");

    assert!(behind.is_file(), "the peer an hour behind was reaped");
    assert!(ahead.is_file(), "the peer an hour ahead was reaped");
    // Both were written a second ago by our own filesystem, and that is the
    // only clock in the answer.
    assert!(peers.iter().all(|p| p.seen_secs_ago <= 2), "{peers:?}");
}

#[test]
fn a_session_that_stopped_beating_is_reaped_however_fresh_it_claims_to_be() {
    // The other half, so the test above cannot pass by never reaping anything.
    // A crashed peer whose last heartbeat happened to carry a timestamp from a
    // clock running fast would otherwise sit in the folder forever.
    let dir = folder();
    let dead =
        plant(dir.path(), "nadia", &stamp(Offset::zero()), STALE_AFTER + Duration::from_secs(30));

    assert!(peers_in(dir.path()).is_empty(), "a session that stopped beating stayed listed");
    assert!(!dead.exists(), "the stale file was left to accumulate");
}

#[test]
fn how_long_ago_a_peer_was_seen_comes_from_the_file_not_from_the_file_contents() {
    // `seenSecsAgo` is what #17 renders and what a stale-presence banner would
    // key off. Deriving it from `heartbeatAt` would put another machine's clock
    // error straight on screen, and nothing on the frontend could tell.
    let dir = folder();
    plant(dir.path(), "nadia", &stamp(Offset::zero()), Duration::from_secs(30));

    let peers = peers_in(dir.path());
    assert_eq!(peers.len(), 1);
    assert!(
        (29..=32).contains(&peers[0].seen_secs_ago),
        "expected roughly 30s, got {}",
        peers[0].seen_secs_ago,
    );
}

#[test]
fn two_sessions_in_one_folder_see_each_other_and_not_themselves() {
    let dir = folder();
    let nadia = Presence::start(dir.path());
    let sam = Presence::start(dir.path());

    let seen_by_nadia = nadia.peers();
    assert_eq!(seen_by_nadia.len(), 1, "{seen_by_nadia:?}");
    assert_eq!(seen_by_nadia[0].session_id, sam.session_id());

    let seen_by_sam = sam.peers();
    assert_eq!(seen_by_sam.len(), 1, "{seen_by_sam:?}");
    assert_eq!(seen_by_sam[0].session_id, nadia.session_id());

    assert_eq!(peers_in(dir.path()).len(), 2, "both are in the folder");
}

#[test]
fn a_session_that_closes_stops_being_a_peer_immediately() {
    // Without the delete on close, leaving a project would leave the person
    // shown as present for another minute — long enough that the next person
    // in decides the feature is lying.
    let dir = folder();
    let nadia = Presence::start(dir.path());
    let sam = Presence::start(dir.path());
    assert_eq!(nadia.peers().len(), 1);

    drop(sam);
    assert!(nadia.peers().is_empty(), "a cleanly closed session lingered");
}

#[test]
fn which_nodes_someone_has_open_crosses_the_folder() {
    let dir = folder();
    let nadia = Presence::start(dir.path());
    let sam = Presence::start(dir.path());

    let kael = wobu_core::new_id();
    sam.set_editing(vec![kael]);

    let peers = nadia.peers();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].editing, vec![kael], "the editing list did not reach the other side");

    // And moving on releases nothing, because nothing was held.
    sam.set_editing(Vec::new());
    assert!(nadia.peers()[0].editing.is_empty());
}

#[test]
fn presence_never_stands_between_anyone_and_a_save() {
    // The line the whole feature is on the right side of. Declaring a node as
    // being edited is a courtesy to the other person, not a claim on the file:
    // hard locks over a share strand files whenever a laptop sleeps, and the
    // recovery is worse than the collision. If this test ever needs a
    // `--force`, something has misread `docs/07-file-shares.md`.
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let kael = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();

    let nadia = Presence::start(project.root());
    nadia.set_editing(vec![kael.id]);

    let mut mine = project.get_node(kael.id).unwrap();
    mine.notes_raw = "saved anyway".into();
    let outcome = project.save_node(mine).unwrap();
    assert!(matches!(outcome, SaveOutcome::Saved(_)), "{outcome:?}");
    assert_eq!(project.get_node(kael.id).unwrap().notes_raw, "saved anyway");

    // Deleting it is not blocked either — the dot is information, not a veto.
    project.delete_node(kael.id).unwrap();
}

#[test]
fn opening_a_project_clears_out_what_a_crash_left_behind() {
    // Reaping has to happen without the UI asking. A team that never opens the
    // presence panel still accumulates one file per crash, per closed lid, per
    // dropped VPN, and the listing presence depends on gets slower every month.
    let dir = folder();
    let dead =
        plant(dir.path(), "nadia", &stamp(Offset::zero()), STALE_AFTER + Duration::from_secs(5));
    let live = plant(dir.path(), "sam", &stamp(Offset::zero()), Duration::from_secs(2));

    let _presence = Presence::start(dir.path());

    assert!(!dead.exists(), "opening the project left the dead session behind");
    assert!(live.is_file(), "opening the project reaped a live one");
}

#[test]
fn files_that_are_not_ours_are_left_alone() {
    // A project folder on a share collects other people's tooling, and this
    // reaper deletes things. It only ever deletes what it can name.
    let dir = folder();
    let notes = sessions(dir.path()).join("notes.json");
    let readme = sessions(dir.path()).join("README.txt");
    for path in [&notes, &readme] {
        fs::write(path, "not a session").unwrap();
        let f = fs::OpenOptions::new().write(true).open(path).unwrap();
        f.set_modified(SystemTime::now() - (STALE_AFTER + Duration::from_secs(60))).unwrap();
    }

    assert!(peers_in(dir.path()).is_empty());
    assert!(notes.is_file(), "deleted a file that was never a session");
    assert!(readme.is_file(), "deleted a file that was never a session");
}

/// The one name, in the two places it appears (#76).
///
/// Presence says who is *here* and a conflict sibling says who *wrote* the
/// paragraph you are reading. If those disagree, a user looking at "Nadia is
/// editing Kael Vantris" beside `kael-vantris.conflict-<somebody-else>-….md`
/// cannot tell whether they are the same person, and the whole point of naming
/// a peer is gone. They come from `wobu_store::peer::alias`, and this is the
/// assertion that keeps them coming from the same call.
#[test]
fn presence_and_a_conflict_sibling_call_this_installation_the_same_thing() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let node = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();

    // Somebody else's Wobu saves over the file while we are holding it.
    let rel = project.index().rel_path_of(node.id).unwrap().expect("indexed");
    let path = wobu_store::paths::from_rel_string(project.root(), &rel);
    let mut ours = project.get_node(node.id).unwrap();
    let theirs = fs::read_to_string(&path).unwrap();
    let (frontmatter, _) = theirs.split_once("\n---\n").unwrap();
    fs::write(&path, format!("{frontmatter}\n---\n\n## Notes\n\nnadia got here first\n")).unwrap();
    ours.notes_raw = "our only copy of this paragraph".into();
    let SaveOutcome::Conflict { conflict_path } = project.save_node(ours).unwrap() else {
        panic!("the fixture needs a conflict and did not get one")
    };

    let _presence = Presence::start(project.root());
    let announced = peers_in(project.root());
    assert_eq!(announced.len(), 1, "{announced:?}");

    let sibling = conflict_path.rsplit('/').next().unwrap();
    let wrote_it = wobu_store::conflict::parse(sibling).expect("a sibling").peer;
    assert_eq!(wrote_it.as_deref(), Some(announced[0].user.as_str()), "{sibling}");
    assert_eq!(wrote_it.as_deref(), Some(wobu_store::peer::alias()));
}
