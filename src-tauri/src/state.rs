//! The mutable process state: the open project, the watcher keeping it honest
//! about what is on disk, the reconnect loop for when that disk stops being
//! there — and, deliberately beside all of it rather than inside it, the job
//! queue.
//!
//! ## Why the queue is not in the slot
//!
//! [`Jobs`] is managed separately from [`AppState`] and holds no reference to
//! it. Two reasons, and the second is the one that matters. A job outlives the
//! project it was started for — a paid call whose project has since closed is
//! still in flight and still has to be able to report what it cost. And a queue
//! that could reach the `Mutex` below would be the likeliest place in this
//! codebase to end up holding it across an await, which is exactly what the
//! next section says must never happen. It cannot, because there is no path
//! from a job to it.
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};
use wobu_core::Id;
use wobu_jobs::{Config, Event, Failure, JobId, Notify, Queue, QueueSnapshot, events};
use wobu_store::{
    Cancel, Error as StoreError, Peer, Presence, PresenceHandle, Project, Watcher, paths,
};

use crate::diag;
use crate::error::{CommandResult, WobuError};
use crate::redact;

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
    /// Whoever needs to know which project is open — in practice `sync.rs`, and
    /// nothing else. See [`Handover`].
    ///
    /// A `Weak` rather than an `Arc`, and that is not tidiness: the sync manager
    /// holds an [`AppState`] clone so it can reach the open project, so an
    /// `Arc` here would be a cycle that never drops and a `SyncEndpoint` that
    /// outlives the process's willingness to shut it down. `None` after the
    /// manager is gone means "nobody is syncing", which is exactly right.
    handover: Arc<Mutex<Option<Weak<dyn Handover>>>>,
}

/// Told, synchronously, which project the window is about to hold.
///
/// The whole of the #82 race, in two calls. `Project` owns a `rusqlite`
/// connection to an index keyed by project ULID in local app data, and sync
/// opens projects nobody is looking at — so a project that is *both* open in the
/// window and held by the sync manager would be two `Project` values writing to
/// one index and one folder, each maintaining its own idea of what is on disk.
/// The folder would still be canonical and the index would still rebuild, but
/// `world:changed` would stop being true and a fast-forward could be decided
/// against a row the other handle had already moved.
///
/// So there is exactly one holder at a time, and this is the handover. It is
/// deliberately **not** a channel: an event queued for a background task leaves
/// a window in which both halves believe they own the folder, and that window is
/// the bug. Both methods block until sync has let go, which is at most one
/// synchronous store step — a `reconcile`, an `apply` batch — because the rule
/// in this module's header holds on the sync side too and no network I/O ever
/// runs with a replica's lock held.
///
/// Implemented by `crate::sync::SyncManager` and by nothing else. It is a trait
/// so that this module does not have to know what a `SyncEndpoint` is; a project
/// opens on a machine that will never sync, and `state.rs` should keep compiling
/// if the sync module is torn out.
pub trait Handover: Send + Sync + 'static {
    /// This project is about to become the open one. Give the folder back.
    ///
    /// Called with the project mutex *not* held and before the slot is filled,
    /// so an implementation may block and may not call back into [`AppState`].
    fn opening(&self, project: Id, root: &Path);
    /// The open project is about to be dropped. Sync may take it again.
    fn closing(&self, project: Id);
}

impl AppState {
    /// Run `f` against the open project, or fail with `no_project_open`.
    pub fn with<T>(&self, f: impl FnOnce(&mut Project) -> CommandResult<T>) -> CommandResult<T> {
        let mut guard = self.slot.lock();
        let open = guard.as_mut().ok_or_else(WobuError::no_project_open)?;
        f(&mut open.project)
    }

    /// Like [`with`](Self::with), but only if the open project is the one
    /// named — otherwise `no_project_open`.
    ///
    /// For sync, and the check is the point rather than a formality. A
    /// background round decided which project it was about when the connection
    /// opened; by the time it has bytes to write, the user may have closed that
    /// world and opened another. Reaching for "the open project" at that moment
    /// would fold one project's incoming nodes into a different one's folder,
    /// and every hash involved would agree that it was fine.
    pub fn with_project<T>(
        &self,
        project: Id,
        f: impl FnOnce(&mut Project) -> CommandResult<T>,
    ) -> CommandResult<T> {
        let mut guard = self.slot.lock();
        let open = guard.as_mut().filter(|o| o.project.id() == project);
        let open = open.ok_or_else(WobuError::no_project_open)?;
        f(&mut open.project)
    }

    /// Like [`with`](Self::with), but for callers that are fine with there
    /// being nothing open.
    pub fn peek<T>(&self, f: impl FnOnce(Option<&Project>) -> T) -> T {
        let guard = self.slot.lock();
        f(guard.as_ref().map(|o| &o.project))
    }

    /// Which project is open, if any. One lock, one field, no borrow — the
    /// shape a background task can use without holding anything.
    pub fn open_id(&self) -> Option<Id> {
        self.slot.lock().as_ref().map(|o| o.project.id())
    }

    /// Register the sync manager's interest in open/close. Idempotent by
    /// replacement; there is only ever one.
    pub fn observe(&self, handover: Weak<dyn Handover>) {
        *self.handover.lock() = Some(handover);
    }

    /// The observer, if there still is one. Taken out from under the lock,
    /// because calling it holds the *replica's* lock and a sync step inside that
    /// lock reaches back for this one — the two-lock cycle this method exists to
    /// keep impossible.
    fn handover(&self) -> Option<Arc<dyn Handover>> {
        self.handover.lock().as_ref().and_then(Weak::upgrade)
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
        let id = project.id();

        self.close();
        // Between here and the slot being filled, sync has already let go and
        // the window does not hold it yet, so a round mid-flight finds nothing
        // and gives up on this pass. That is the correct answer to "who owns the
        // folder right now" — briefly, nobody — and it is a far better one than
        // two handles both saying "me".
        if let Some(handover) = self.handover() {
            handover.opening(id, &root);
        }
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

        // Read the id and give the lock straight back before telling sync, so
        // that a round which is inside `with_project` at this instant can finish
        // and release it. The handover then blocks on the replica's lock, which
        // is the *other* order — hence the two statements rather than one.
        let closing = self.slot.lock().as_ref().map(|o| o.project.id());
        if let (Some(id), Some(handover)) = (closing, self.handover()) {
            handover.closing(id);
        }

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
    /// callback, the reconnect thread and the sync manager — none of which can
    /// borrow from a Tauri `State<'_, _>`.
    pub fn handle(&self) -> AppState {
        AppState {
            slot: Arc::clone(&self.slot),
            generation: Arc::clone(&self.generation),
            opening: Arc::clone(&self.opening),
            handover: Arc::clone(&self.handover),
        }
    }
}

enum Outcome {
    Reconciled(bool),
    WentOffline,
}

/// The job queue, and the bridge that turns what it says into Tauri events.
///
/// Managed state of its own — see the module header for why it is not part of
/// [`AppState`]. Everything interesting is in `wobu-jobs`; what lives here is
/// the adapter, and it is deliberately the only thing the shell contributes to
/// the queue.
pub struct Jobs(Queue);

impl Jobs {
    /// Started from `setup`, which is the first moment an `AppHandle` exists.
    ///
    /// Not a `Default`, and that is why: the queue's whole purpose is to emit,
    /// and managed state is registered before there is anything to emit
    /// through.
    pub fn start(app: &AppHandle) -> Jobs {
        // Tauri's runtime, named explicitly rather than taken from
        // `Handle::current` — `setup` does not run inside it, and a queue that
        // started a runtime of its own would be a second thread pool competing
        // with the one every async command already uses.
        let runtime = tauri::async_runtime::handle().inner().clone();
        Jobs(Queue::with_runtime(Config::default(), Bridge { app: app.clone() }, runtime))
    }

    /// For the commands that submit work — `enhance_start` today, the image
    /// backends (#40) next.
    pub fn queue(&self) -> &Queue {
        &self.0
    }

    pub fn cancel(&self, id: JobId) -> bool {
        self.0.cancel(id)
    }

    pub fn snapshot(&self) -> QueueSnapshot {
        self.0.snapshot()
    }
}

/// `wobu_jobs::Notify` over `app.emit`.
///
/// The queue calls this from whichever task is driving a job, holding none of
/// its own locks and none of this crate's — `Bridge` cannot reach [`AppState`],
/// so there is no way for an emit to end up behind the project mutex.
struct Bridge {
    app: AppHandle,
}

impl Notify for Bridge {
    fn notify(&self, event: Event) {
        // Emission failures are dropped deliberately: the window may be on its
        // way out, and a job that cannot announce itself is not a job that
        // should stop working.
        let _ = match event {
            Event::State(snapshot) => self.app.emit(events::JOB_STATE, snapshot),
            Event::Progress(progress) => self.app.emit(events::JOB_PROGRESS, progress),
            Event::Preview(preview) => self.app.emit(events::JOB_PREVIEW, preview),
            Event::Retry(mut retry) => {
                retry.failure = scrubbed(retry.failure);
                self.app.emit(events::JOB_RETRY, retry)
            }
            Event::Done(done) => self.app.emit(events::JOB_DONE, done),
            Event::Failed(mut failed) => {
                failed.failure = scrubbed(failed.failure);
                // Logged here, and after scrubbing. `WobuError::new` is the
                // equivalent choke point for command failures — one place that
                // catches every one of them for the log — and a job failure
                // never passes through it, so without this line the one class
                // of error the user is most likely to ask about would be the
                // one class missing from the log they send us.
                diag::error(&format!("job {}: {}", failed.id, failed.failure.message));
                self.app.emit(events::JOB_ERROR, failed)
            }
        };
    }
}

/// Every string on a job failure, through the same scrubber every other message
/// crossing this boundary goes through.
///
/// This is the only path to the webview that does not run through
/// [`WobuError::new`], and a provider's own words are exactly where a key would
/// turn up — an `Unavailable { detail }` carrying a URL with the key in its
/// query string is the realistic version. `redact::scrub` is reused rather than
/// reimplemented because it is idempotent and because a second scrubber is a
/// second thing to keep in step with the first.
fn scrubbed(mut failure: Failure) -> Failure {
    failure.message = redact::scrub(&failure.message);
    failure.detail = failure.detail.as_deref().map(redact::scrub);
    failure.cost_note = failure.cost_note.as_deref().map(redact::scrub);
    failure
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_string_on_a_job_failure_is_scrubbed_before_it_leaves_the_process() {
        // `job:error` is the one route to the webview that does not pass through
        // `WobuError::new`, and a provider that echoes the request back in its
        // error — they do — would otherwise put the user's key in an event
        // payload, in the log, and in whatever they paste into an issue.
        //
        // Every field is loaded with a key rather than just the message,
        // because the regression this is really guarding is a *new* string
        // field on `Failure` that nobody thinks to add above.
        let failure = Failure::new(
            "provider.unavailable",
            "GET https://api.example/v1/messages?api_key=sk-ant-abc123 failed",
        )
        .with_detail("x-api-key: sk-ant-abc123")
        .cost_note("billed under key sk-ant-abc123");

        let clean = scrubbed(failure);
        for field in [Some(clean.message), clean.detail, clean.cost_note] {
            let text = field.expect("the field was set");
            assert!(!text.contains("sk-ant-abc123"), "a key survived redaction in {text:?}");
            assert!(text.contains(redact::MASK), "nothing was masked in {text:?}");
        }
    }

    #[test]
    fn an_ordinary_failure_message_comes_through_unchanged() {
        // The other half: a scrubber that masked everything would be safe and
        // useless, and the message is the only thing telling the user what
        // happened.
        let failure = Failure::new("provider.rate_limited", "Anthropic is rate limiting this key.");
        let clean = scrubbed(failure);
        assert_eq!(clean.message, "Anthropic is rate limiting this key.");
        assert_eq!(clean.detail, None);
    }
}
