//! The thing that owns a `SyncEndpoint`, and the thing that owns a project
//! nobody is looking at.
//!
//! ## Why this exists at all
//!
//! [`SyncEndpoint`] holds iroh's `Router`, which is `#[must_use]` and **aborts
//! its accept loop when dropped**. A `SyncEndpoint` that goes out of scope takes
//! every inbound connection with it, at once and without telling the peers, and
//! nothing raises an error — the app simply stops being reachable. So something
//! has to hold it for as long as sync is meant to work, and that something has
//! to outlive any one project. `wobu-sync`'s own module documentation names this
//! file as the owner it implies.
//!
//! ## Beside the open-project slot, not inside it
//!
//! `AppState` holds `Mutex<Option<Open>>`: exactly one project, and only while
//! somebody is looking at it. "Sync in the background" means syncing worlds
//! nobody has open, so sync state cannot live in that slot — a share would stop
//! converging the moment its window closed, which is the opposite of what a
//! share is for.
//!
//! So the registry here is driven by [`Shares`], which persists, and each entry
//! is a [`Replica`]: a project ULID, a folder, and a rule about who is currently
//! holding it.
//!
//! ## One holder at a time, and the handover is synchronous
//!
//! This is the only genuinely new invariant in #82 and it is worth stating
//! plainly. `Project` owns a `rusqlite::Connection` to an index in local app
//! data keyed by project ULID. Two `Project` values for one ULID is two writers
//! to one SQLite file and two independent caches of what is on disk. The folder
//! is still canonical and the index still rebuilds, so nothing is *lost* — but
//! `world:changed` stops being true, and a fast-forward can be decided against a
//! row the other handle already moved. That is a corruption of meaning rather
//! than of bytes, which is the kind nobody notices for a week.
//!
//! [`Held`] is therefore an exclusive two-state thing, and [`crate::state::Handover`]
//! is how the window claims and releases it. Both directions block, and they
//! block on a lock that is only ever held across a *synchronous store step* —
//! never across network I/O. `state.rs`'s rule ("every helper takes the lock,
//! does one thing, and gives it back") is the same rule, and sync is precisely
//! the workload that breaks it if anybody gets lazy: the round in
//! [`super::round`] is written as a sequence of short locked steps with the
//! network strictly between them, and it is not an optimisation, it is the only
//! reason a background sync does not freeze the editor.
//!
//! There is a deliberate gap in the middle of an open: between sync letting go
//! and the window filling its slot, *nobody* holds the folder, and a round in
//! flight finds nothing and gives up on that pass. Briefly nobody is the correct
//! answer to "who owns this"; two people is not.
//!
//! ## Opening a project is slow, so it does not happen under the lock
//!
//! `Project::open` rescans the folder, which on a share is seconds. Doing that
//! with [`Replica::held`] locked would mean the user's "Open project" click
//! waiting behind a background scan of a *different* world. [`Replica::with`]
//! therefore takes the lock, finds nothing, gives it back, opens outside it, and
//! re-takes it to install — and drops what it opened if the window claimed the
//! project in the meantime.
//!
//! That leaves one racer: two rounds against the same replica both opening. The
//! round gate below is what makes it impossible rather than merely unlikely.
//!
//! ## One round at a time per project
//!
//! [`Replica::round`] is a `tokio::sync::Mutex` held for a whole round. An
//! inbound session that cannot take it is closed immediately rather than queued,
//! and the peer retries — because a second concurrent round against one replica
//! does no work the first is not already doing, and making a peer *wait* would
//! stall it inside a manifest exchange whose idle timeout would then fire and be
//! reported as "the peer went quiet". Refusing fast and reconnecting is a better
//! lie than that, and it is not even a lie.
//!
//! ## The accept side holds a `Weak`
//!
//! [`SyncEndpoint::bind`] takes `Arc<dyn Projects>` and `Arc<dyn Sessions>`, and
//! the natural implementation of both is this manager — which also holds the
//! endpoint. That is a reference cycle, and a cycle here is not a leak of a few
//! bytes: it is a `Router` that can never be dropped and therefore an accept
//! loop that outlives every intention to stop it. [`Accepts`] holds a
//! [`std::sync::Weak`] instead, and a manager that is gone refuses everything —
//! which is the right behaviour and is checked rather than assumed.

mod membership;
mod poller;
mod status;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use parking_lot::Mutex;
use wobu_core::Id;
use wobu_store::Project;
use wobu_sync::{Blobs, Config, Identity, Reach, SyncEndpoint};

use self::membership::Accepts;
use crate::diag;
use crate::error::{Code, CommandResult, WobuError};
use crate::state::{AppState, Handover};
use crate::sync::shares::{Share, Shares};
use crate::sync::{ProjectSyncStatus, SyncPeerStatus, SyncPhase};

/// How long a whole inbound session gets before it is abandoned.
///
/// Not a bound on how much can be transferred — every step inside already has
/// its own idle deadline, so a large project over a slow link keeps going as
/// long as it keeps arriving. This is the backstop for the case those cannot
/// see: a peer that stays busy for ever, alternating between saying something
/// and waiting, which no per-step deadline can distinguish from useful work.
const SESSION_BUDGET: Duration = Duration::from_secs(300);

/// How long [`SyncManager::shutdown`] will wait before giving up and dropping
/// the endpoint.
///
/// A quit that hangs is worse than a peer that is cut off mid-transfer. See
/// [`SyncManager::shutdown`] for the argument and for why this is a backstop
/// rather than the mechanism.
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);

/// Seconds between outbound polls, by consecutive rounds that did nothing.
///
/// Starts eager, because the first poll after a share is accepted is the one a
/// person is watching, and caps at two minutes, which is the interval at which a
/// collaborator who came online is noticed without either machine spending its
/// evening dialling.
///
/// The cap is also #80's "back off on a permanent conflict". Two peers that both
/// hold a version the other refuses will exchange manifests, agree there is
/// nothing to do, write nothing, and do it again — for ever, at whatever
/// interval this list ends on. A round that changed nothing counts as a round
/// that did nothing, so a stuck pair walks down this list to the bottom and
/// stays there rather than polling hot.
const BACKOFF: &[u64] = &[3, 5, 10, 20, 45, 90, 120];

/// Where `world:changed` goes.
///
/// A trait rather than an `AppHandle`, for the same reason `wobu_jobs::Notify`
/// is one: an `AppHandle` cannot be constructed outside a running Tauri app, so
/// a manager that named one directly would be a manager with no test. The
/// implementation in [`super`] is three lines and is where the "only if this
/// project is the open one" decision lives — a refetch for a world nobody is
/// looking at is a refetch of nothing.
pub trait Wake: Send + Sync + 'static {
    fn world_changed(&self, project: Id);

    /// A default keeps non-window harnesses concerned only with file
    /// invalidation small. The production window overrides both status hooks.
    fn sync_state(&self, _status: ProjectSyncStatus) {}
    fn sync_peer(&self, _status: ProjectSyncStatus) {}
}

#[derive(Default)]
struct RuntimeProject {
    state: SyncPhase,
    peers: BTreeMap<String, SyncPeerStatus>,
}

impl RuntimeProject {
    fn snapshot(&self, project: Id) -> ProjectSyncStatus {
        ProjectSyncStatus {
            project,
            state: self.state,
            peers: self.peers.values().cloned().collect(),
            // Filled in by the two callers that know the folder, because this
            // struct is the *runtime* half of the status and whether a clone
            // finished arriving is a fact about a directory.
            arriving: false,
        }
    }
}

/// Who is holding a project's folder and index right now.
///
/// Two states and no third. In particular there is no "both" and no "nobody,
/// permanently": see the module documentation.
#[allow(
    clippy::large_enum_variant,
    reason = "exactly one of these exists per shared project, behind a Mutex and \
              never moved through a channel, so boxing the Project would buy an \
              allocation and a pointer chase to save a few hundred bytes once"
)]
enum Held {
    /// The window has it. Reach it through [`AppState`], so that the index the
    /// UI is reading from and the index sync is writing to are the same one.
    Open,
    /// Sync has it, or may take it. `None` until a round first needs it, because
    /// opening rescans and a share this machine has not touched today should not
    /// cost a scan at startup.
    Detached(Option<Project>),
}

/// One shared project, and the discipline for reaching it.
pub struct Replica {
    project: Id,
    root: PathBuf,
    /// A caller-chosen local index, used by the two-peer integration harness so
    /// two replicas of one project do not meet in the app's ULID-keyed index.
    /// `None` is the app path and remains the production default.
    index_path: Option<PathBuf>,
    state: AppState,
    held: Mutex<Held>,
    /// Held for a whole round. See the module documentation.
    round: tokio::sync::Mutex<()>,
    /// How many consecutive outbound rounds have found nothing to do, which is
    /// what [`BACKOFF`] is indexed by.
    idle: AtomicU64,
    /// Interrupts a backed-off outbound poll when this replica gained bytes the
    /// other peers do not have yet — from an inbound round, or from the user
    /// editing the folder this machine is holding.
    changed: tokio::sync::Notify,
}

impl Replica {
    fn new(
        project: Id,
        root: PathBuf,
        index_path: Option<PathBuf>,
        state: AppState,
        open: bool,
    ) -> Replica {
        Replica {
            project,
            root,
            index_path,
            state,
            held: Mutex::new(if open { Held::Open } else { Held::Detached(None) }),
            round: tokio::sync::Mutex::new(()),
            idle: AtomicU64::new(0),
            changed: tokio::sync::Notify::new(),
        }
    }

    fn expedite(&self) {
        self.idle.store(0, Ordering::Relaxed);
        self.changed.notify_one();
    }

    pub fn project(&self) -> Id {
        self.project
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Do one thing with this project, whoever is holding it.
    ///
    /// The closure is **synchronous and must stay that way**. It runs with a
    /// lock held that the window's "open project" path blocks on, so an `await`
    /// in here would be the editor freezing while a background sync waits for a
    /// relay. That is not a style rule — it is the reason the signature takes a
    /// `FnOnce` rather than something returning a future, and a future-returning
    /// version of this method must not be added.
    pub fn with<T>(&self, f: impl FnOnce(&mut Project) -> CommandResult<T>) -> CommandResult<T> {
        // The fast path, and the only one the handover ever waits behind: one
        // lock, no I/O.
        {
            let mut held = self.held.lock();
            match &mut *held {
                Held::Open => return self.state.with_project(self.project, f),
                Held::Detached(Some(project)) => return f(project),
                Held::Detached(None) => {}
            }
        }

        // Outside the lock, because this rescans the folder. See the module
        // documentation; the round gate is what stops two of these racing.
        let opened = match &self.index_path {
            Some(index_path) => Project::open_at_index(&self.root, index_path)?,
            None => Project::open(&self.root)?,
        };

        let mut held = self.held.lock();
        match &mut *held {
            // The window claimed it while we were scanning. Drop what we opened,
            // unused, rather than installing a second handle on one index.
            Held::Open => self.state.with_project(self.project, f),
            Held::Detached(slot) => f(slot.get_or_insert(opened)),
        }
    }

    /// Bring the canonical folder into the index without making an open
    /// project's global mutex pay for a network walk.
    pub fn reconcile(&self) -> CommandResult<()> {
        {
            let mut held = self.held.lock();
            match &mut *held {
                Held::Open => {}
                Held::Detached(Some(project)) => {
                    project.reconcile()?;
                    return Ok(());
                }
                Held::Detached(None) => {
                    drop(held);
                    let opened = match &self.index_path {
                        Some(index_path) => Project::open_at_index(&self.root, index_path)?,
                        None => Project::open(&self.root)?,
                    };
                    let mut held = self.held.lock();
                    match &mut *held {
                        Held::Open => drop(opened),
                        Held::Detached(slot) => {
                            *slot = Some(opened);
                            return Ok(());
                        }
                    }
                }
            }
        }

        // The handover state can move after `held` is released. AppState
        // rechecks the project id and generation before applying, so a close or
        // replacement turns this into no_project_open rather than touching the
        // successor.
        self.state.reconcile_project_now(self.project)?;
        Ok(())
    }

    /// Whether the window is currently holding this project.
    pub fn is_open(&self) -> bool {
        matches!(*self.held.lock(), Held::Open)
    }

    #[cfg(test)]
    pub(super) fn release_for_test(&self) {
        *self.held.lock() = Held::Detached(None);
    }

    /// How many consecutive empty rounds the backoff currently stands at.
    #[cfg(test)]
    pub(in crate::sync) fn idle_steps(&self) -> u64 {
        self.idle.load(Ordering::Relaxed)
    }

    /// Pretend the poller has dialled this many times and found nothing, which
    /// is how a replica arrives at the two-minute end of [`BACKOFF`].
    #[cfg(test)]
    pub(in crate::sync) fn go_idle_for_test(&self, steps: u64) {
        self.idle.store(steps, Ordering::Relaxed);
    }

    /// Whether a wake is waiting for the poller, without blocking if it is not.
    ///
    /// `Notify::notify_one` leaves a permit when nobody is waiting, so this is
    /// also the assertion that an expedite raised *during* a round is not lost
    /// before the poller comes back around to its next sleep.
    #[cfg(test)]
    pub(in crate::sync) async fn took_wake_for_test(&self) -> bool {
        tokio::time::timeout(Duration::from_millis(250), self.changed.notified()).await.is_ok()
    }

    /// Hand the folder to the window. Blocks until sync has let go.
    fn hand_over(&self) {
        let mut held = self.held.lock();
        // Assigning drops the `Project` — and with it the SQLite connection —
        // while the lock is held, which is what makes "the window's handle is
        // the only one" true at the moment the window is told it may proceed.
        *held = Held::Open;
    }

    /// Take the folder back. Lazily: the next round opens it.
    fn take_back(&self) {
        let mut held = self.held.lock();
        if matches!(*held, Held::Open) {
            *held = Held::Detached(None);
        }
    }
}

/// The sync manager: one endpoint, one replica per shared project.
pub struct SyncManager {
    state: AppState,
    wake: Arc<dyn Wake>,
    identity: Identity,
    /// Filled immediately after `bind`, and never replaced.
    ///
    /// A `OnceLock` rather than a field, because binding needs `Arc<dyn
    /// Projects>` and `Arc<dyn Sessions>` — which are this manager — so the
    /// manager has to exist before the endpoint does. The alternative is a
    /// second struct holding everything except the endpoint, which is the same
    /// cycle with more names in it.
    endpoint: OnceLock<SyncEndpoint>,
    /// One machine-wide verified content store. A round scopes a cheap clone
    /// to its replica root before it reads or places any project path.
    blobs: Option<Blobs>,
    shares: Mutex<Shares>,
    replicas: Mutex<BTreeMap<Id, Arc<Replica>>>,
    /// Ephemeral connectivity plus the last completed round for each peer.
    /// This is deliberately memory-only: after restart, "never synced in this
    /// session" is an honest answer and a stale historical green light is not.
    runtime: Mutex<BTreeMap<Id, RuntimeProject>>,
    index_dir: Option<PathBuf>,
    poll: bool,
    /// Read on every poll tick and before every round step. Set once, never
    /// cleared: a manager that has been shut down stays shut down, because the
    /// endpoint it would need is closed.
    stopping: AtomicBool,
    /// Tauri's handle rather than tokio's, so that a poller can be started from
    /// a synchronous method without a runtime in scope — `share` and `accept`
    /// are called from commands and must not panic for want of one.
    pollers: Mutex<Vec<tauri::async_runtime::JoinHandle<()>>>,
}

/// How to start one.
pub struct Setup {
    pub identity: Identity,
    pub reach: Reach,
    pub shares: Shares,
    /// Whether to dial out at all.
    ///
    /// `false` in the tests that are about the accept side, where a poller
    /// dialling a ticket nobody minted is noise in the log and a task to shut
    /// down for no reason.
    pub poll: bool,
    /// Override local index placement. The app passes `None`; integration tests
    /// use one directory per simulated machine so replicas of one ULID remain
    /// genuinely independent.
    pub index_dir: Option<PathBuf>,
}

/// Told which project the window is about to hold. See [`Handover`].
impl Handover for SyncManager {
    fn opening(&self, project: Id, root: &Path) {
        // A project being opened is also a project this machine plainly holds,
        // so register it even if it is not shared — it costs one map entry and
        // it means a ticket accepted while it is open joins rather than clones.
        self.register(project, root, true);
        for (id, replica) in self.replicas.lock().iter() {
            if *id == project {
                replica.hand_over();
            } else {
                // Exactly one project can be open, so anything else claiming to
                // be is a stale mark — a close that never arrived because the
                // manager started after it. Sweeping here is what keeps
                // `Held::Open` from surviving its project.
                replica.take_back();
            }
        }
    }

    fn closing(&self, project: Id) {
        if let Some(replica) = self.replicas.lock().get(&project) {
            replica.take_back();
        }
    }

    fn changed_locally(&self, project: Id) {
        // Unshared projects have a replica too — `opening` registers one for
        // anything the window holds — but no peers, so `dial_round` returns on
        // the empty peer list and the expedite costs one loop iteration.
        if let Some(replica) = self.replicas.lock().get(&project) {
            replica.expedite();
        }
    }
}

/// A `wobu-sync` failure, as something the webview can read.
///
/// Everything the transport can raise is either "the other machine is not
/// answering" or "the link broke", and both are `sync.unreachable`, which is retryable
/// — a peer coming back is the expected outcome of a peer-to-peer share with no
/// seed node. `NotATicket` is the one exception and it is the user's paste, not
/// the network's fault, so it lands on `node.invalid` where the UI will show the
/// message rather than offering a "Try again" that would fail identically.
impl From<wobu_sync::Error> for WobuError {
    fn from(e: wobu_sync::Error) -> WobuError {
        let message = e.to_string();
        match e {
            wobu_sync::Error::NotATicket => WobuError::new(
                Code::Invalid,
                "That is not a Wobu share link. Copy the whole line, starting with `wobuproject`.",
            )
            .with_detail(message),
            wobu_sync::Error::ProjectNotHeld => WobuError::new(
                Code::SyncUnreachable,
                "The other machine no longer has this project. \
                 A share cannot be taken back, so this means they removed it.",
            ),
            wobu_sync::Error::BlobStore { .. } => {
                WobuError::new(Code::Io, "Wobu could not prepare local storage for synced images.")
                    .with_detail(message)
            }
            _ => WobuError::new(Code::SyncUnreachable, "Could not reach the other machine.")
                .with_detail(message),
        }
    }
}

/// A write to the shares file that failed, logged rather than propagated.
///
/// The share is in effect for this run either way, and failing the command the
/// user just ran — after the ticket has already been minted — would be telling
/// them the share did not happen when it did.
fn report(result: std::io::Result<()>) {
    if let Err(e) = result {
        diag::error(format!("sync: could not write the share list: {e}"));
    }
}

impl SyncManager {
    /// Bind an endpoint and start syncing every share.
    ///
    /// Async because binding is, and it is called from a task rather than from
    /// Tauri's `setup` for that reason. Nothing here creates a runtime and
    /// nothing may: iroh runs on the one Tauri already started, which is what
    /// makes the accept loop something the app can actually stop.
    pub async fn start(
        state: AppState,
        wake: Arc<dyn Wake>,
        setup: Setup,
    ) -> CommandResult<Arc<SyncManager>> {
        if let Some(index_dir) = &setup.index_dir {
            std::fs::create_dir_all(index_dir).map_err(|error| {
                WobuError::new(Code::Io, "Could not create the local sync index directory.")
                    .with_detail(error.to_string())
            })?;
        }
        // The content database is local cache, never project data. Tests place
        // it beside their private index directory; production places it in app
        // data. `service-root` only supplies the endpoint's serving clone with
        // a valid root — project rounds replace it with their own root before
        // any path is read or written.
        let blob_home = setup
            .index_dir
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(wobu_store::paths::app_data_dir);
        let service_root = blob_home.join("sync-blob-root");
        let blobs = match std::fs::create_dir_all(&service_root) {
            Err(error) => {
                diag::error(format!(
                    "sync: image transfer is unavailable; text sync will continue: {error}"
                ));
                None
            }
            Ok(()) => match Blobs::open(&service_root, blob_home.join("sync-blob-cache")).await {
                Ok(blobs) => Some(blobs),
                Err(error) => {
                    diag::error(format!(
                        "sync: image transfer is unavailable; text sync will continue: {error}"
                    ));
                    None
                }
            },
        };

        let manager = Arc::new(SyncManager {
            state,
            wake,
            identity: setup.identity.clone(),
            endpoint: OnceLock::new(),
            blobs: blobs.clone(),
            shares: Mutex::new(setup.shares),
            replicas: Mutex::new(BTreeMap::new()),
            runtime: Mutex::new(BTreeMap::new()),
            index_dir: setup.index_dir,
            poll: setup.poll,
            stopping: AtomicBool::new(false),
            pollers: Mutex::new(Vec::new()),
        });

        let accepts: Arc<Accepts> = Arc::new(Accepts(Arc::downgrade(&manager)));
        let config = Config {
            identity: Some(setup.identity),
            reach: setup.reach,
            blobs,
            ..Config::default()
        };
        let endpoint = SyncEndpoint::bind(config, accepts.clone(), accepts)
            .await
            .map_err(|e| WobuError::from(e).with_detail("binding the sync endpoint"))?;
        // Infallible: nothing else can reach this `OnceLock` before `start`
        // returns, because nothing else has the `Arc` yet.
        let _ = manager.endpoint.set(endpoint);

        // The window may already have a project open — sync starts a moment
        // after the app does, and a user who was quick, or a share that was
        // opened from a command line argument, both land here. Asking rather
        // than assuming is what stops the first round opening a second handle on
        // an index the editor is already writing to.
        let open = manager.state.open_id();
        let shares: Vec<Share> = manager.shares.lock().all().to_vec();
        for share in shares {
            manager.register(share.project, &share.root, open == Some(share.project));
            if setup.poll {
                manager.spawn_poller(share.project);
            }
        }

        manager.state.observe(Arc::downgrade(&manager) as Weak<dyn Handover>);
        Ok(manager)
    }

    /// This peer's name to everybody else, and its TLS certificate.
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn endpoint(&self) -> &SyncEndpoint {
        self.endpoint.get().expect("`start` fills this before it returns")
    }

    /// The shared content store, scoped to this replica's project folder.
    pub fn blobs_for(&self, replica: &Replica) -> CommandResult<Option<Blobs>> {
        self.blobs
            .as_ref()
            .map(|blobs| blobs.with_root(&replica.root).map_err(WobuError::from))
            .transpose()
    }

    /// Where a round says the folder moved.
    pub fn wake(&self) -> &dyn Wake {
        self.wake.as_ref()
    }

    pub fn stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    pub fn replica(&self, project: Id) -> Option<Arc<Replica>> {
        self.replicas.lock().get(&project).cloned()
    }

    pub fn shares(&self) -> Vec<Share> {
        self.shares.lock().all().to_vec()
    }

    pub(super) fn root_of(&self, project: Id) -> Option<PathBuf> {
        self.replicas.lock().get(&project).map(|r| r.root.clone())
    }

    /// The open project's folder, if the open project is this one.
    fn open_root(&self, project: Id) -> Option<PathBuf> {
        self.state.peek(|open| open.filter(|p| p.id() == project).map(|p| p.root().to_path_buf()))
    }

    /// Add a replica, or leave the one that is there.
    ///
    /// Replacing an existing one would drop a `Project` some round is inside,
    /// and would reset the handover state to whatever this caller guessed. A
    /// share that is already registered is already correct.
    fn register(&self, project: Id, root: &Path, open: bool) {
        let index_path = self.index_dir.as_ref().map(|dir| dir.join(format!("{project}.sqlite")));
        self.replicas.lock().entry(project).or_insert_with(|| {
            Arc::new(Replica::new(
                project,
                root.to_path_buf(),
                index_path,
                self.state.handle(),
                open,
            ))
        });
    }

    /// Stop accepting, stop dialling, and close the endpoint.
    ///
    /// ## Why this cannot hang
    ///
    /// Four things, and the last one is the one that makes it a guarantee rather
    /// than an argument:
    ///
    /// 1. **The flag first.** Every poller checks [`Self::stopping`] on waking
    ///    and every round step checks it between steps, so nothing new starts.
    /// 2. **The pollers are aborted, not awaited.** A poller can be asleep for
    ///    two minutes or inside a dial that iroh is still retrying, and waiting
    ///    for either would be waiting for a timeout somebody else chose. An
    ///    aborted round leaves nothing half-written: every write inside one is a
    ///    single `guarded_write` + rename, and the base it would have moved only
    ///    moves on acknowledgement, so the worst an abort costs is a node
    ///    re-offered on the next round.
    /// 3. **`SyncEndpoint::shutdown`, not a drop.** Dropping the `Router` aborts
    ///    the accept loop silently and severs every inbound connection; shutdown
    ///    winds the loop down and closes the endpoint. iroh does abort the
    ///    in-flight accept futures once the handler's own shutdown returns, so a
    ///    session mid-transfer is still cut — but the peers are told, which is
    ///    the difference between a clean quit and a hung socket at the far end.
    /// 4. **A deadline around all of it.** [`SHUTDOWN_BUDGET`] is a backstop, not
    ///    the mechanism: if iroh's shutdown ever blocks on something this file
    ///    cannot see, the app quits five seconds later instead of never. A quit
    ///    that hangs is the worst outcome available here, worse than a peer that
    ///    is cut off, so the timeout is not negotiable even though nothing is
    ///    expected to reach it.
    ///
    /// The trap this is written against: `Sessions::opened` *is* the connection's
    /// lifetime, so an accept handler that blocks for ever is an accept loop that
    /// cannot be wound down. Every round inside one is bounded by
    /// [`SESSION_BUDGET`] and every step inside that by its own idle deadline, so
    /// there is no reachable state in which a handler outlives the budget.
    ///
    /// Idempotent, and safe to call from the main thread with no locks held —
    /// which is the other half of the trap. A round takes the project mutex, so
    /// shutting down *while holding* it would deadlock: the round would wait for
    /// the lock and the shutdown would wait for the round. Nothing here takes it.
    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        for handle in self.pollers.lock().drain(..) {
            handle.abort();
        }
        match tokio::time::timeout(SHUTDOWN_BUDGET, self.endpoint().shutdown()).await {
            Ok(Ok(())) => diag::info("sync: endpoint closed"),
            Ok(Err(e)) => diag::error(format!("sync: endpoint shutdown failed: {e}")),
            Err(_elapsed) => diag::error("sync: endpoint shutdown timed out; dropping it"),
        }
        if let Some(blobs) = &self.blobs {
            match tokio::time::timeout(SHUTDOWN_BUDGET, blobs.shutdown()).await {
                Ok(Ok(())) => diag::info("sync: blob store closed"),
                Ok(Err(e)) => diag::error(format!("sync: blob store shutdown failed: {e}")),
                Err(_elapsed) => diag::error("sync: blob store shutdown timed out; dropping it"),
            }
        }
    }
}
