//! Who else has this world open.
//!
//! Each session writes `.wobu/sessions/<session-ulid>.json`, refreshes it every
//! twenty seconds, and deletes it on a clean close. Everyone reads everyone
//! else's, and the answer becomes a name in the title bar and a quiet dot beside
//! a node somebody is typing into.
//!
//! This is **advisory and only advisory**. Nothing in this module blocks,
//! delays or refuses a write, and nothing may be made to: hard locks over a
//! share strand files whenever a laptop sleeps or a VPN drops, and the recovery
//! UX is worse than the problem it solves. See `docs/07-file-shares.md`.
//!
//! ## Liveness is decided by mtime, never by the timestamp inside the file
//!
//! The obvious reaper parses `heartbeatAt` out of the JSON and subtracts it from
//! `Utc::now()`. That compares two machines' wall clocks, and desktops on a LAN
//! routinely sit a minute or more apart. At sixty seconds of tolerance that is
//! not a rounding error: a laptop running two minutes slow writes heartbeats
//! that look two minutes stale the instant they land, so every other machine
//! reaps it continuously while the person is sitting there working. Skew the
//! other way is quieter and worse — a session that never expires, and a peer who
//! left an hour ago still shown as present.
//!
//! So staleness here is always `now − mtime`, both halves observed through our
//! own `stat`. `heartbeatAt` is written for a human reading the folder and for
//! nothing else.
//!
//! Where that still leaks, honestly: on SMB and NFS the **server** stamps mtime,
//! so what is really compared is the server's clock against ours rather than
//! ours against itself. A machine badly out of step with its file server sees
//! every session — its own included — as uniformly too old or too young. That is
//! a strictly smaller failure than the per-peer one above, and one NTP fixes,
//! but nothing here detects it and it is not airtight.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wobu_core::Id;

/// How often we rewrite our own session file.
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(20);

/// How long a session may go unrefreshed before whoever wrote it is treated as
/// gone.
///
/// Three missed beats rather than one. A single beat lost to a share that
/// stalled for a few seconds is entirely ordinary, and reaping on it would make
/// collaborators flicker in and out of the UI while they are sitting right
/// there.
pub const STALE_AFTER: Duration = Duration::from_secs(60);

/// Granularity of the beat's sleep, so a close is not waiting out the remainder
/// of a twenty-second nap before the thread notices and exits.
const CLOSE_TICK: Duration = Duration::from_millis(250);

/// `.wobu/sessions/<session-ulid>.json`.
///
/// Keys are camelCase on disk to agree with `project.json`, the only other JSON
/// file in a project folder — two files a person may open in the same editor
/// should not disagree about their spelling.
///
/// `heartbeatAt` is here so that a human — or a support request — can read the
/// folder and see when someone was last in it. It is never what decides whether
/// this session is alive; see the module comment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub user: String,
    pub host: String,
    pub opened_at: DateTime<Utc>,
    pub heartbeat_at: DateTime<Utc>,
    /// Nodes this session has open, as the frontend last reported them.
    #[serde(default)]
    pub editing: Vec<Id>,
}

/// Somebody else with this project open, as the UI sees them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub session_id: Id,
    pub user: String,
    /// Best effort, and `unknown` on platforms that do not hand it to us. See
    /// `current_host`.
    pub host: String,
    /// How long ago their file was last refreshed, by **our** clock.
    ///
    /// Sent instead of the timestamps in the file. A duration the frontend
    /// worked out from a timestamp another machine wrote would be wrong by
    /// exactly the skew this module exists to route around, and nothing on that
    /// side of the bridge could tell.
    pub seen_secs_ago: u64,
    /// Node ids they have open. Advisory: the editor shows a passive banner and
    /// no save anywhere consults this.
    pub editing: Vec<Id>,
}

/// An open session, owned by whoever opened the project.
///
/// Dropping it stops the beat and removes the session file, which is why the
/// app holds it beside the watcher in the one slot that a close empties.
pub struct Presence {
    handle: PresenceHandle,
}

/// A non-owning view of a session.
///
/// Exists so a caller holding a lock can let go of it before touching the
/// folder: [`PresenceHandle::peers`] is a directory listing plus a read per
/// peer over whatever the project is mounted on, and the app's project mutex is
/// needed by every other command. Dropping a handle does not end the session.
#[derive(Clone)]
pub struct PresenceHandle {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
}

struct Shared {
    root: PathBuf,
    path: PathBuf,
    session_id: Id,
    user: String,
    host: String,
    opened_at: DateTime<Utc>,
    /// Doubles as the write lock for our own session file, which is what lets
    /// a close delete that file without a beat putting it straight back.
    editing: Mutex<Vec<Id>>,
}

impl Presence {
    /// Announce ourselves and start beating.
    ///
    /// Infallible on purpose. A project whose session file cannot be written —
    /// a read-only share, a folder owned by someone else — is still a project
    /// that opens, and presence is a courtesy rather than a precondition.
    pub fn start(root: &Path) -> Presence {
        Presence::start_every(root, HEARTBEAT_EVERY)
    }

    /// The interval injected, so the beat can be tested in under a second
    /// rather than over a minute.
    fn start_every(root: &Path, every: Duration) -> Presence {
        let session_id = wobu_core::new_id();
        let shared = Arc::new(Shared {
            root: root.to_path_buf(),
            path: sessions_dir(root).join(format!("{session_id}.json")),
            session_id,
            // The same name a conflict sibling in this folder is stamped with —
            // see [`crate::peer`]. Shared so that a collaborator in the title bar
            // and the writer of the file beside them cannot be labelled
            // differently, which would leave a user unable to tell that the
            // person who is here is the person whose paragraph they are looking
            // at. The field is still `user` on disk and on the wire because the
            // session file is a thing a human may open in an editor.
            user: crate::peer::alias().to_owned(),
            host: current_host(),
            opened_at: Utc::now(),
            editing: Mutex::new(Vec::new()),
        });
        let handle = PresenceHandle { shared, stop: Arc::new(AtomicBool::new(false)) };

        // Before the thread rather than after it, so the next person to open
        // this folder sees us immediately instead of finding it apparently
        // empty for the first twenty seconds of the session.
        handle.beat();

        let thread = handle.clone();
        std::thread::spawn(move || {
            while !thread.stop.load(Ordering::Relaxed) {
                let step = CLOSE_TICK.min(every);
                let mut waited = Duration::ZERO;
                while waited < every {
                    if thread.stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(step);
                    waited += step;
                }
                thread.beat();
            }
        });

        Presence { handle }
    }

    /// A view that can outlive a lock but not the session. See
    /// [`PresenceHandle`].
    pub fn handle(&self) -> PresenceHandle {
        self.handle.clone()
    }

    pub fn session_id(&self) -> Id {
        self.handle.shared.session_id
    }

    pub fn peers(&self) -> Vec<Peer> {
        self.handle.peers()
    }

    pub fn set_editing(&self, editing: Vec<Id>) {
        self.handle.set_editing(editing);
    }
}

impl PresenceHandle {
    /// Everyone else in this folder, with anything that stopped beating reaped
    /// on the way past.
    pub fn peers(&self) -> Vec<Peer> {
        peers_excluding(&self.shared.root, Some(self.shared.session_id))
    }

    /// Record which nodes this session has open.
    ///
    /// The whole list rather than an add/remove pair. The frontend already
    /// knows exactly which nodes are open, whereas a delta protocol drifts the
    /// first time one closes during a disconnection and then stays wrong for
    /// the rest of the session.
    pub fn set_editing(&self, editing: Vec<Id>) {
        *lock(&self.shared.editing) = editing;
        // Written now rather than at the next beat: up to twenty seconds of
        // "Nadia is editing" pointing at a node she has already left is the
        // kind of wrongness that teaches people to stop believing the dot.
        self.beat();
    }

    /// Rewrite our session file, and tidy up anyone who has stopped writing
    /// theirs.
    ///
    /// Every failure below is swallowed. On a read-only folder each of these
    /// calls fails every time, and a user who deliberately opened a world they
    /// cannot write to must not be told about it once every twenty seconds.
    fn beat(&self) {
        let editing = lock(&self.shared.editing);
        // Read under the lock because a `Presence` being dropped takes this
        // same lock to delete the file. Without the check here, a beat that
        // arrived a microsecond later would put the file back and leave a ghost
        // in the folder for the next sixty seconds.
        if self.stop.load(Ordering::Relaxed) {
            return;
        }
        // A write into an unmounted share's leftover mountpoint succeeds — on
        // the local disk, invisibly, to be shadowed the moment the share
        // returns. For node text that loses an edit; here it only litters, but
        // it litters somewhere nobody will ever look.
        if !crate::paths::project_is_present(&self.shared.root) {
            return;
        }

        let dir = sessions_dir(&self.shared.root);
        let _ = fs::create_dir_all(&dir);
        reap(&dir, SystemTime::now());

        let session = Session {
            user: self.shared.user.clone(),
            host: self.shared.host.clone(),
            opened_at: self.shared.opened_at,
            heartbeat_at: Utc::now(),
            editing: editing.clone(),
        };
        let Ok(json) = serde_json::to_vec_pretty(&session) else { return };
        // Written in place rather than staged and renamed the way node text is.
        // A reader that catches this half-written gets JSON that does not parse
        // and skips one peer for one poll, which twenty seconds later fixes
        // itself — cheap enough that an `fsync` per session per beat over SMB
        // is not worth paying to avoid it.
        let _ = fs::write(&self.shared.path, json);
    }
}

impl Drop for Presence {
    /// A clean close takes the session file with it, so nobody waits a minute
    /// to stop seeing us. A crash does not, which is what `reap` is for.
    fn drop(&mut self) {
        self.handle.stop.store(true, Ordering::Relaxed);
        // Held so a beat already in flight finishes before the delete, and so
        // the next one — which reads `stop` under this same lock — cannot
        // recreate the file afterwards.
        let _guard = lock(&self.handle.shared.editing);
        let _ = fs::remove_file(&self.handle.shared.path);
    }
}

/// Everyone with this folder open, without needing a session of our own.
///
/// This is what an open reads before it has announced itself, and what a test
/// reads to see what a session actually wrote.
pub fn peers_in(root: &Path) -> Vec<Peer> {
    peers_excluding(root, None)
}

fn peers_excluding(root: &Path, own: Option<Id>) -> Vec<Peer> {
    let dir = sessions_dir(root);
    let now = SystemTime::now();
    reap(&dir, now);

    let Ok(entries) = fs::read_dir(&dir) else {
        // No sessions folder, or a share that has gone away. "Nobody" is a
        // perfectly good answer to "who else is here", and there is nothing
        // here worth turning into an error the user has to read.
        return Vec::new();
    };

    let mut peers = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(session_id) = session_id_of(&path) else { continue };
        if own == Some(session_id) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let age = age(&meta, now);
        // Reaped above; this catches one that went stale in between, and keeps
        // the listing honest even if the delete failed on a read-only folder.
        if age > STALE_AFTER {
            continue;
        }
        let Ok(raw) = fs::read(&path) else { continue };
        let Ok(session) = serde_json::from_slice::<Session>(&raw) else { continue };

        peers.push(Peer {
            session_id,
            user: session.user,
            host: session.host,
            seen_secs_ago: age.as_secs(),
            editing: session.editing,
        });
    }

    // Sorted because `read_dir` returns whatever order the filesystem feels
    // like, and a list of names that reshuffles itself on every poll reads as a
    // bug. Session ULIDs lead with a millisecond timestamp, so this comes out
    // roughly in the order people arrived.
    peers.sort_by_key(|a| a.session_id);
    peers
}

/// Delete session files nobody is refreshing any more.
///
/// Stats only — nothing is opened. Without this a folder a team has used for a
/// year accumulates one file per crash, per closed laptop lid, per VPN drop,
/// and the listing that presence depends on gets slower every month.
fn reap(dir: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        // Only files we recognise as sessions. Anything else in here belongs to
        // somebody's tooling or a sync client, and deleting other people's
        // files out of a shared folder is not ours to do.
        if session_id_of(&path).is_none() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if age(&meta, now) > STALE_AFTER {
            let _ = fs::remove_file(&path);
        }
    }
}

/// How long ago a file was written, by our own clock against the mtime we
/// observe. The module comment explains why it is not the timestamp inside.
fn age(meta: &fs::Metadata, now: SystemTime) -> Duration {
    let Ok(modified) = meta.modified() else {
        // A filesystem that will not report mtime cannot be reaped safely, so
        // let the session linger: a ghost in the peer list is cosmetic, reaping
        // a collaborator who is sitting right there is not.
        return Duration::ZERO;
    };
    // `duration_since` errors on a file stamped in the future, which is skew in
    // the other direction. Reading that as brand new is the same cautious
    // choice `Project::sweep_staging` makes.
    now.duration_since(modified).unwrap_or(Duration::ZERO)
}

/// The session ULID a path names, or `None` if this is not one of our files.
fn session_id_of(path: &Path) -> Option<Id> {
    if path.extension().and_then(|e| e.to_str()) != Some("json") {
        return None;
    }
    path.file_stem()?.to_str()?.parse::<Id>().ok()
}

fn sessions_dir(root: &Path) -> PathBuf {
    root.join(".wobu").join("sessions")
}

/// A poisoned lock here means a thread panicked mid-beat. What is behind it is
/// a list of node ids with no invariant a panic could have broken, and refusing
/// to write another heartbeat over it would take presence down for the rest of
/// the session over nothing.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The machine name, as far as it can be established without a crate for it.
///
/// `COMPUTERNAME` is always set on Windows and `HOSTNAME` by most Unix shells,
/// and `/etc/hostname` covers the case that actually bites — a Linux desktop
/// launching Wobu from a menu entry, where no shell ever ran to export
/// anything.
///
/// What is left, honestly: on macOS none of the three exist, so this reads
/// `unknown`, and so will anything else started without an environment. That is
/// survivable because `host` is a subtitle — `user` is the field that says who
/// is in the folder, and it comes from the same place every other user-facing
/// name in this crate does. A `hostname` crate would close the gap and is not
/// worth a dependency for a subtitle.
fn current_host() -> String {
    if let Ok(host) = std::env::var("HOSTNAME").or_else(|_| std::env::var("COMPUTERNAME"))
        && !host.trim().is_empty()
    {
        return host.trim().to_string();
    }
    if let Ok(text) = fs::read_to_string("/etc/hostname")
        && let Some(line) = text.lines().next()
        && !line.trim().is_empty()
    {
        return line.trim().to_string();
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder that passes the "is the project still there" probe, which is
    /// what the beat checks before it writes anything.
    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("project.json"), "{}").unwrap();
        dir
    }

    fn session_files(root: &Path) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(sessions_dir(root)) else { return Vec::new() };
        entries.flatten().map(|e| e.path()).collect()
    }

    #[test]
    fn opening_a_project_announces_the_session_straight_away() {
        // Not on the first beat twenty seconds later: someone who opens a world
        // and is asked "is anyone else in here" in the same breath has to be
        // visible by then.
        let dir = folder();
        let presence = Presence::start(dir.path());
        let files = session_files(dir.path());
        assert_eq!(files.len(), 1, "{files:?}");
        assert!(files[0].ends_with(format!("{}.json", presence.session_id())));
    }

    #[test]
    fn the_session_file_carries_the_documented_fields() {
        let dir = folder();
        let presence = Presence::start(dir.path());
        presence.set_editing(vec![wobu_core::new_id()]);

        let raw = fs::read_to_string(&session_files(dir.path())[0]).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for key in ["user", "host", "openedAt", "heartbeatAt", "editing"] {
            assert!(json.get(key).is_some(), "`{key}` is missing: {raw}");
        }
    }

    #[test]
    fn the_heartbeat_keeps_the_file_fresh() {
        // Scaled down so this runs in a moment; what is being tested is that
        // the timer fires at all, not the wall-clock interval.
        let dir = folder();
        let _presence = Presence::start_every(dir.path(), Duration::from_millis(80));
        let path = session_files(dir.path())[0].clone();
        let first = fs::metadata(&path).unwrap().modified().unwrap();

        std::thread::sleep(Duration::from_millis(400));
        let later = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(later > first, "the session file was never refreshed");
    }

    #[test]
    fn a_clean_close_removes_the_session_file() {
        let dir = folder();
        {
            let _presence = Presence::start(dir.path());
            assert_eq!(session_files(dir.path()).len(), 1);
        }
        assert!(session_files(dir.path()).is_empty(), "we are still advertised as present");
    }

    #[test]
    fn closing_stops_the_beat_rather_than_leaving_a_thread_writing() {
        // The failure this guards: a heartbeat thread that outlives its project
        // and recreates the session file it was just deleted from. The ghost
        // then takes a full minute to reap, and every open of the day leaves
        // another one behind it.
        let dir = folder();
        {
            let _presence = Presence::start_every(dir.path(), Duration::from_millis(50));
            std::thread::sleep(Duration::from_millis(120));
        }
        std::thread::sleep(Duration::from_millis(400));
        assert!(session_files(dir.path()).is_empty(), "a closed session came back");
    }

    #[test]
    fn a_read_only_folder_degrades_silently_instead_of_failing() {
        // Opening a world you cannot write to is a supported thing to do, and
        // presence is a courtesy on top of it. None of this may raise.
        let dir = folder();
        let wobu = dir.path().join(".wobu");
        fs::create_dir_all(&wobu).unwrap();
        if !make_read_only(&wobu) {
            eprintln!("skipping: cannot make a directory read-only (running as root?)");
            return;
        }

        let presence = Presence::start(dir.path());
        presence.set_editing(vec![wobu_core::new_id()]);
        assert!(presence.peers().is_empty());
        drop(presence);

        restore(&wobu);
    }

    #[cfg(unix)]
    fn make_read_only(dir: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
        // Running as root ignores the bits entirely, and a test that silently
        // proves nothing is worse than one that is skipped out loud.
        fs::create_dir(dir.join("probe")).is_err()
    }

    #[cfg(not(unix))]
    fn make_read_only(_dir: &Path) -> bool {
        false
    }

    #[cfg(unix)]
    fn restore(dir: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o755));
    }

    #[cfg(not(unix))]
    fn restore(_dir: &Path) {}

    #[test]
    fn nothing_is_written_into_a_folder_that_has_gone_away() {
        // An unmounted share leaves its mountpoint behind as an empty
        // directory, so this write would succeed on the local disk under it —
        // creating a `.wobu/sessions` that the real folder shadows the moment
        // it comes back.
        let dir = tempfile::tempdir().unwrap();
        let presence = Presence::start(dir.path());
        assert!(session_files(dir.path()).is_empty(), "wrote into a folder with no project.json");
        drop(presence);
    }

    #[test]
    fn a_host_name_is_always_produced() {
        // The field is decoration, but an empty string in the UI reads as a
        // rendering bug rather than as "we do not know".
        assert!(!current_host().is_empty());
    }

    #[test]
    fn only_our_own_files_are_treated_as_sessions() {
        // The reaper deletes what this recognises, and a shared folder collects
        // other people's tooling.
        let id = wobu_core::new_id();
        assert_eq!(session_id_of(Path::new(&format!("/s/{id}.json"))), Some(id));
        assert_eq!(session_id_of(Path::new("/s/notes.json")), None);
        assert_eq!(session_id_of(Path::new(&format!("/s/{id}.txt"))), None);
    }
}
