//! The one piece of mutable process state: the open project, the watcher
//! keeping it honest about what is on disk, and the reconnect loop for when
//! that disk stops being there.
//!
//! ## Why a `Mutex` and not an `RwLock`
//!
//! It would be nice to let `node_list` and `node_get` run concurrently — they
//! only need `&Project`. They cannot. `Project` owns an `Index`, which owns a
//! `rusqlite::Connection`, which is `Send` but deliberately not `Sync`. An
//! `RwLock<Project>` is only `Sync` when `Project: Send + Sync`, so it will not
//! even compile as Tauri managed state; a `Mutex<Project>` needs `Send` alone.
//!
//! That is not as costly as it looks. Reads go to the local SQLite index, not
//! the filesystem, so the critical section is microseconds. The write path is
//! the one to watch, and it is bounded by a single guarded `write` + `rename`
//! of one Markdown file. What must *not* happen is holding the lock across
//! anything slow — an LLM call, a network stat, a dialog — and the rule that
//! keeps that true is that every helper below takes the lock, does one thing,
//! and gives it back.
//!
//! ## Offline
//!
//! A project on a share can stop being reachable at any moment. The index
//! lives in local app data, so when that happens the whole world is still
//! *readable* — the app stays usable and the UI keeps rendering from it. What
//! changes is that writes are refused (in `wobu-store`, on the write path) and
//! a reconnect loop starts. Nothing is closed and nothing is discarded,
//! because the share coming back is the expected outcome rather than the
//! surprising one.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};
use wobu_core::Id;
use wobu_store::{
    Cancel, Error as StoreError, Peer, Presence, PresenceHandle, Project, Watcher, paths,
};

use crate::error::{CommandResult, WobuError};

/// The project folder turned out to differ from what we last indexed.
/// `src/lib/queries.ts` listens for this and invalidates the world.
pub const WORLD_CHANGED: &str = "world:changed";
/// The folder stopped being reachable. The UI raises a banner and keeps
/// rendering from the cache it already has — it must *not* refetch, because
/// the whole point is that nothing has been lost.
pub const SHARE_OFFLINE: &str = "share:offline";
/// It came back, and the index has been reconciled against it.
pub const SHARE_ONLINE: &str = "share:online";

/// Reconnect backoff, in seconds. Starts eager because a laptop waking from
/// sleep is usually back within a second or two, and caps low enough that a
/// share restored over lunch is noticed within half a minute.
const BACKOFF: &[u64] = &[1, 2, 4, 8, 15, 30];
/// Poll granularity, so closing a project is noticed inside a quarter second
/// rather than at the end of the current backoff step.
const TICK: Duration = Duration::from_millis(250);

pub struct Open {
    pub project: Project,
    /// Dropped on close, which stops the watch thread. `None` only in the
    /// window between opening a project and the watcher starting, and if the
    /// platform refused to give us one at all.
    pub watcher: Option<Watcher>,
    /// Our entry in `.wobu/sessions`. Dropped on close, which stops the
    /// twenty-second heartbeat and takes the session file with it — so a
    /// collaborator stops seeing us the moment we leave rather than a minute
    /// later.
    pub presence: Presence,
    /// Whether the folder is currently unreachable. `wobu-store` refuses
    /// writes on its own; this exists so the banner is raised once rather than
    /// on every event, and so a webview that reloaded while disconnected can
    /// ask for the current state.
    pub offline: bool,
}

#[derive(Default)]
pub struct AppState {
    slot: Arc<Mutex<Option<Open>>>,
    /// Bumped on every install and close. Background threads capture the value
    /// they were spawned under and stop as soon as it moves — which is what
    /// stops a reconnect loop belonging to a closed project from reconciling
    /// its successor.
    generation: Arc<AtomicU64>,
    /// The cancel token for a scan currently running, if any.
    ///
    /// Separate from `slot` on purpose: an open is by definition not installed
    /// yet, and parking it in the same mutex would mean holding the lock that
    /// every command needs for the whole duration of a scan that can take
    /// minutes on a NAS.
    opening: Arc<Mutex<Option<Cancel>>>,
}

impl AppState {
    /// Run `f` against the open project, or fail with `no_project_open`.
    pub fn with<T>(&self, f: impl FnOnce(&mut Project) -> CommandResult<T>) -> CommandResult<T> {
        let mut guard = self.slot.lock();
        let open = guard.as_mut().ok_or_else(WobuError::no_project_open)?;
        f(&mut open.project)
    }

    /// Like [`with`](Self::with), but for callers that are fine with there
    /// being nothing open.
    pub fn peek<T>(&self, f: impl FnOnce(Option<&Project>) -> T) -> T {
        let guard = self.slot.lock();
        f(guard.as_ref().map(|o| &o.project))
    }

    pub fn is_offline(&self) -> bool {
        self.slot.lock().as_ref().is_some_and(|o| o.offline)
    }

    /// Who else has the open project's folder open.
    ///
    /// An empty list rather than an error when nothing is open: presence is
    /// advisory, and a poll that arrives one tick after a close is ordinary
    /// rather than something to put on screen.
    pub fn peers(&self) -> Vec<Peer> {
        self.presence().map(|p| p.peers()).unwrap_or_default()
    }

    /// Tell everyone else which nodes this session has open.
    pub fn set_editing(&self, node_ids: Vec<Id>) {
        if let Some(presence) = self.presence() {
            presence.set_editing(node_ids);
        }
    }

    /// A handle taken out from under the lock, because everything done with one
    /// touches the project folder — a directory listing and a read per peer, or
    /// a write, over whatever the world happens to be mounted on. Holding the
    /// mutex every other command needs across an SMB round trip is exactly what
    /// this module's header says must not happen.
    ///
    /// A handle whose project has since closed is inert: its writes no-op, so
    /// nothing here can resurrect a session file that a close has removed.
    fn presence(&self) -> Option<PresenceHandle> {
        self.slot.lock().as_ref().map(|o| o.presence.handle())
    }

    /// Hand out a cancel token for a scan that is about to start.
    ///
    /// Replacing any previous one rather than reusing it: a token that has
    /// already been cancelled would abort the new scan instantly, so the user
    /// who cancelled one open and immediately picked another folder would find
    /// the second one refusing to open for no visible reason.
    pub fn begin_open(&self) -> Cancel {
        let cancel = Cancel::new();
        *self.opening.lock() = Some(cancel.clone());
        cancel
    }

    pub fn finish_open(&self) {
        *self.opening.lock() = None;
    }

    /// Stop the scan in flight, if there is one.
    pub fn cancel_open(&self) {
        if let Some(cancel) = self.opening.lock().as_ref() {
            cancel.cancel();
        }
    }

    /// Install a freshly opened project, replacing (and closing) whatever was
    /// open before, then start watching its folder.
    pub fn install(&self, app: &AppHandle, project: Project) {
        let root = project.root().to_path_buf();

        self.close();
        let generation = self.generation.load(Ordering::SeqCst);
        // After `close`, deliberately: reopening the same folder must not leave
        // the previous session's file beside the new one, which would show the
        // user to themselves as a second person in the world.
        let presence = Presence::start(&root);
        *self.slot.lock() = Some(Open { project, watcher: None, presence, offline: false });

        let watcher = self.start_watcher(app, &root, generation);
        if let Some(open) = self.slot.lock().as_mut() {
            open.watcher = watcher;
        }
    }

    /// Close the open project, if any. Idempotent.
    pub fn close(&self) {
        // Bumped first: a background thread waking between here and the `take`
        // below sees a stale generation and bows out rather than reconciling a
        // project on its way out.
        self.generation.fetch_add(1, Ordering::SeqCst);

        // Written as take-then-drop rather than `*self.slot.lock() = None` so
        // that the `Project` — and with it the SQLite index handle — is dropped
        // with the guard already released. The watch thread may be inside the
        // callback below at this instant; taking the value out under the lock
        // means we wait for that reconcile to finish, and dropping it outside
        // means the thread's next iteration finds `None` and no-ops rather than
        // queueing behind us.
        let taken = self.slot.lock().take();
        drop(taken);
    }

    fn start_watcher(&self, app: &AppHandle, root: &Path, generation: u64) -> Option<Watcher> {
        let this = self.handle();
        let app = app.clone();
        let watched = root.to_path_buf();

        let result = Watcher::start(root, move || {
            this.on_folder_event(&app, &watched, generation)
        });

        match result {
            Ok(w) => Some(w),
            // Not fatal: without a watcher the app still reads and writes, the
            // view is just no longer live to edits made outside it.
            Err(e) => {
                eprintln!("wobu: could not watch {}: {e}", root.display());
                None
            }
        }
    }

    /// The watcher fired: either the folder changed, or it went away.
    ///
    /// Returns whether anything actually moved. On a share that answer decides
    /// the next poll interval — the poller has no other way to tell whether
    /// somebody else is working in this world right now.
    fn on_folder_event(&self, app: &AppHandle, root: &Path, generation: u64) -> bool {
        if self.generation.load(Ordering::SeqCst) != generation {
            return false;
        }

        // Reconcile under the lock; emit outside it.
        let outcome = {
            let mut guard = self.slot.lock();
            let Some(open) = guard.as_mut() else { return false };
            if open.offline {
                // The reconnect loop owns recovery from here. A stray event
                // for a dead mountpoint must not race it.
                return false;
            }
            match open.project.reconcile() {
                // `reconcile` reports whether anything actually moved, and the
                // frontend is only woken when it did — otherwise every save the
                // app itself makes bounces straight back as an invalidation
                // and a refetch.
                Ok(changed) => Outcome::Reconciled(changed),
                Err(StoreError::Disconnected) => {
                    open.offline = true;
                    Outcome::WentOffline
                }
                Err(e) => {
                    eprintln!("wobu: reconcile failed: {e}");
                    Outcome::Reconciled(false)
                }
            }
        };

        match outcome {
            Outcome::Reconciled(true) => {
                let _ = app.emit(WORLD_CHANGED, ());
                true
            }
            Outcome::Reconciled(false) => false,
            Outcome::WentOffline => {
                let _ = app.emit(SHARE_OFFLINE, ());
                self.spawn_reconnect(app, root, generation);
                // A share that just vanished is the opposite of idle: stay on
                // the fast poll so coming back is noticed quickly.
                true
            }
        }
    }

    /// Wait for the folder to come back, then reconcile and say so.
    ///
    /// A thread rather than a timer because the wait is unbounded and the work
    /// either side of it is trivial. It exits on remount, or as soon as the
    /// generation moves — so closing the project, or opening another, stops it.
    fn spawn_reconnect(&self, app: &AppHandle, root: &Path, generation: u64) {
        let this = self.handle();
        let app = app.clone();
        let root: PathBuf = root.to_path_buf();
        let last = *BACKOFF.last().expect("BACKOFF is not empty");

        std::thread::spawn(move || {
            for step in BACKOFF.iter().copied().chain(std::iter::repeat(last)) {
                let ticks = step * 1000 / TICK.as_millis() as u64;
                for _ in 0..ticks {
                    if this.generation.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    std::thread::sleep(TICK);
                }
                if this.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                if !paths::project_is_present(&root) {
                    continue;
                }

                let recovered = {
                    let mut guard = this.slot.lock();
                    let Some(open) = guard.as_mut() else { return };
                    match open.project.reconcile() {
                        Ok(_) => {
                            open.offline = false;
                            true
                        }
                        // It went away again between the probe and the lock.
                        Err(_) => false,
                    }
                };
                if recovered {
                    let _ = app.emit(SHARE_ONLINE, ());
                    let _ = app.emit(WORLD_CHANGED, ());
                    return;
                }
            }
        });
    }

    /// A detached clone sharing the same slot and generation, for the watcher
    /// callback and the reconnect thread — neither of which can borrow from a
    /// Tauri `State<'_, _>`.
    fn handle(&self) -> AppState {
        AppState {
            slot: Arc::clone(&self.slot),
            generation: Arc::clone(&self.generation),
            opening: Arc::clone(&self.opening),
        }
    }
}

enum Outcome {
    Reconciled(bool),
    WentOffline,
}
