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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use parking_lot::Mutex;
use wobu_core::Id;
use wobu_store::Project;
use wobu_sync::{
    Config, Disposition, Identity, Projects, Reach, Session, Sessions, SyncEndpoint, Ticket,
};

use crate::diag;
use crate::error::{Code, CommandResult, WobuError};
use crate::state::{AppState, Handover};
use crate::sync::round;
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
        }
    }

    pub fn project(&self) -> Id {
        self.project
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

    /// Whether the window is currently holding this project.
    pub fn is_open(&self) -> bool {
        matches!(*self.held.lock(), Held::Open)
    }

    #[cfg(test)]
    pub(super) fn release_for_test(&self) {
        *self.held.lock() = Held::Detached(None);
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
        let manager = Arc::new(SyncManager {
            state,
            wake,
            identity: setup.identity.clone(),
            endpoint: OnceLock::new(),
            shares: Mutex::new(setup.shares),
            replicas: Mutex::new(BTreeMap::new()),
            runtime: Mutex::new(BTreeMap::new()),
            index_dir: setup.index_dir,
            poll: setup.poll,
            stopping: AtomicBool::new(false),
            pollers: Mutex::new(Vec::new()),
        });

        let accepts: Arc<Accepts> = Arc::new(Accepts(Arc::downgrade(&manager)));
        let config =
            Config { identity: Some(setup.identity), reach: setup.reach, ..Config::default() };
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

    /// Catch-up snapshots for a webview that mounted after an event fired.
    pub fn project_statuses(&self) -> Vec<ProjectSyncStatus> {
        let shares = self.shares();
        let runtime = self.runtime.lock();
        shares
            .into_iter()
            .map(|share| {
                let mut snapshot = runtime
                    .get(&share.project)
                    .map_or_else(
                        || ProjectSyncStatus {
                            project: share.project,
                            state: SyncPhase::Idle,
                            peers: Vec::new(),
                        },
                        |status| status.snapshot(share.project),
                    );

                Self::add_known_peers(&mut snapshot, share.peers);
                snapshot
            })
            .collect()
    }

    /// Announce the catch-up shape after the manager is installed in
    /// `SyncState`. In particular this emits `idle`: a webview may have queried
    /// while endpoint binding was still in flight, and silence would leave that
    /// truthful-but-temporary "not running" answer on screen forever.
    pub fn announce(&self) {
        for status in self.project_statuses() {
            self.wake.sync_state(status);
        }
    }

    fn announce_project(&self, project: Id) {
        if let Some(status) = self.project_statuses().into_iter().find(|s| s.project == project) {
            self.wake.sync_state(status);
        }
    }

    fn add_known_peers(snapshot: &mut ProjectSyncStatus, tickets: Vec<Ticket>) {
        // A joined project knows its outbound peers before the first dial. Put
        // them in status as disconnected so "offline" can name who was tried
        // rather than looking like no setup.
        for ticket in tickets {
            let endpoint_id = ticket.peer().to_string();
            if !snapshot.peers.iter().any(|peer| peer.endpoint_id == endpoint_id) {
                snapshot.peers.push(SyncPeerStatus {
                    endpoint_id,
                    alias: ticket.alias(),
                    connected: false,
                    last_converged_at: None,
                });
            }
        }
        snapshot.peers.sort_by(|a, b| a.alias.cmp(&b.alias));
    }

    fn with_known_peers(&self, mut snapshot: ProjectSyncStatus) -> ProjectSyncStatus {
        if let Some(share) = self.shares.lock().get(snapshot.project).cloned() {
            Self::add_known_peers(&mut snapshot, share.peers);
        }
        snapshot
    }

    fn set_phase(&self, project: Id, state: SyncPhase) {
        let snapshot = {
            let mut runtime = self.runtime.lock();
            let status = runtime.entry(project).or_default();
            if status.state == state {
                return;
            }
            status.state = state;
            status.snapshot(project)
        };
        self.wake.sync_state(self.with_known_peers(snapshot));
    }

    fn set_peer(
        &self,
        project: Id,
        endpoint_id: String,
        alias: String,
        connected: bool,
        converged: bool,
        state: SyncPhase,
    ) {
        let snapshot = {
            let mut runtime = self.runtime.lock();
            let status = runtime.entry(project).or_default();
            status.state = state;
            let peer = status.peers.entry(endpoint_id.clone()).or_insert(SyncPeerStatus {
                endpoint_id,
                alias: alias.clone(),
                connected,
                last_converged_at: None,
            });
            peer.alias = alias;
            peer.connected = connected;
            if converged {
                peer.last_converged_at = Some(
                    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                );
            }
            status.snapshot(project)
        };
        let snapshot = self.with_known_peers(snapshot);
        self.wake.sync_peer(snapshot.clone());
        self.wake.sync_state(snapshot);
    }

    /// Somebody pressed "Share".
    ///
    /// Returns the ticket to hand out. **Await [`SyncEndpoint::online`] first**
    /// under [`Reach::Internet`] or the address in it has no relay and the
    /// string works on this machine's LAN and nowhere else — which fails at the
    /// far end as a dial timeout and reads to a user as "they are offline". The
    /// caller does that awaiting, because it is the caller that knows whether
    /// somebody is standing in front of a progress spinner.
    pub fn share(self: &Arc<SyncManager>, project: Id, root: &Path) -> Ticket {
        let grant = {
            let mut shares = self.shares.lock();
            let grant = shares.share(project, root).grant;
            report(shares.save());
            grant
        };
        self.register(project, root, self.state.open_id() == Some(project));
        self.announce_project(project);
        if self.poll {
            self.spawn_poller(project);
        }
        self.endpoint().ticket(project, grant)
    }

    /// Somebody pasted a ticket.
    ///
    /// [`Disposition::Clone`] is returned rather than acted on: cloning means
    /// creating a folder somewhere the user picks, and a background task must
    /// not choose where somebody's world lives. The caller surfaces it.
    ///
    /// [`Disposition::Join`] is the case this method actually does something in,
    /// and it is the one the issue names: one project ULID is one world however
    /// many folders it is sitting in, so a ticket for a project already here
    /// starts syncing *this* replica rather than making a second one.
    pub fn accept(self: &Arc<SyncManager>, ticket: &Ticket) -> Disposition {
        let project = ticket.project();
        if ticket.disposition(&self.present()) == Disposition::Clone {
            return Disposition::Clone;
        }
        let Some(root) = self.root_of(project).or_else(|| self.open_root(project)) else {
            // `present` said yes a line ago, so this is unreachable in practice.
            // Reporting `Clone` rather than panicking is the honest answer: this
            // machine cannot name a folder for the project, which is what
            // `Clone` means.
            return Disposition::Clone;
        };

        let mut shares = self.shares.lock();
        shares.invite(project, &root, ticket.clone());
        report(shares.save());
        drop(shares);

        self.register(project, &root, self.state.open_id() == Some(project));
        self.announce_project(project);
        if self.poll {
            self.spawn_poller(project);
        }
        Disposition::Join
    }

    /// Stop syncing a project, and forget everything agreed with everybody about
    /// it.
    ///
    /// `forget_peer` for each peer as well as dropping the share, and the reason
    /// is `wobu-store`'s: a base is a claim that a specific machine holds
    /// specific bytes and the next sync fast-forwards on it without asking. A
    /// base left behind for a collaborator who has been un-shared is a licence to
    /// overwrite, sitting in a database, waiting for somebody to be re-added.
    /// Forgetting too much costs a re-compare; forgetting too little costs
    /// somebody's writing.
    pub fn unshare(&self, project: Id) -> CommandResult<()> {
        let peers: Vec<String> = {
            let mut shares = self.shares.lock();
            let peers = shares
                .get(project)
                .map(|s| s.peers.iter().map(|t| t.peer().to_string()).collect())
                .unwrap_or_default();
            shares.forget(project);
            report(shares.save());
            peers
        };

        // Taken out of the map before the loop, so the registry lock is not held
        // across a store step. The lock order everywhere else is replicas → held
        // → project slot, and this is the one place it would have been tempting
        // to hold all three.
        let replica = self.replicas.lock().remove(&project);
        if let Some(replica) = replica {
            for peer in &peers {
                // Reported and not propagated: a share that has been removed
                // from the list is not shared any more whether or not the index
                // could be tidied, and failing the command would leave the two
                // halves disagreeing.
                if let Err(e) = replica.with(|p| Ok(p.forget_peer(peer)?)) {
                    diag::error(format!(
                        "sync: could not forget peer for {project}: {}",
                        e.message
                    ));
                }
            }
        }
        self.runtime.lock().remove(&project);
        Ok(())
    }

    /// Whether this machine holds a project and the dialler presented the grant
    /// this installation minted for it. Both failures deliberately collapse to
    /// one bool before the transport constructs its refusal.
    fn admits(&self, project: &Id, grant: Option<&wobu_sync::Grant>) -> bool {
        if !self.replicas.lock().contains_key(project) {
            return false;
        }
        self.shares
            .lock()
            .get(*project)
            .is_some_and(|share| grant.is_some_and(|grant| grant == &share.grant))
    }

    /// Every world on this machine, shared or merely open. See [`Present`].
    ///
    /// Deliberately a wider set than [`Self::admits`], and the gap between them is
    /// the point. "Do I have this world anywhere" is the question a pasted ticket
    /// asks, and the open project counts — a friend sending a ticket for the
    /// world already on screen is joining it, not cloning a second copy of it
    /// next to itself. "May a stranger who dialled me sync this" is a different
    /// question and its answer is [`Self::admits`], which is shares only, because
    /// merely opening a folder is not consent to serve it to anybody who can
    /// guess its ULID off a `project.json` on a NAS.
    fn present(&self) -> Present {
        let mut ids: std::collections::BTreeSet<Id> =
            self.replicas.lock().keys().copied().collect();
        ids.extend(self.state.open_id());
        Present(ids)
    }

    fn root_of(&self, project: Id) -> Option<PathBuf> {
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

    /// One explicit outbound round, with no timer. The integration harness uses
    /// this to make every network step deterministic and observable.
    #[cfg(test)]
    pub(super) async fn run_once(self: &Arc<SyncManager>, project: Id) -> bool {
        self.dial_round(project).await
    }

    #[cfg(test)]
    pub(super) async fn run_ticket(self: &Arc<SyncManager>, project: Id, ticket: &Ticket) -> bool {
        let Some(replica) = self.replica(project) else { return false };
        self.set_phase(project, SyncPhase::Connecting);
        let Ok(session) = self.endpoint().connect_ticket(ticket).await else {
            self.set_phase(project, SyncPhase::Offline);
            return false;
        };
        let gate = replica.round.lock().await;
        let endpoint_id = ticket.peer().to_string();
        let alias = ticket.alias();
        self.set_peer(
            project,
            endpoint_id.clone(),
            alias.clone(),
            true,
            false,
            SyncPhase::Syncing,
        );
        let outcome = round::run(self, &replica, &session).await;
        drop(gate);
        session.close();
        let converged = outcome.as_ref().is_ok_and(|outcome| outcome.converged());
        self.set_peer(project, endpoint_id, alias, false, converged, SyncPhase::Idle);
        outcome.is_ok_and(|outcome| outcome.did_something())
    }

    /// One outbound round against every peer a share names.
    ///
    /// Returns whether anything actually moved, which is what [`BACKOFF`] walks.
    async fn dial_round(self: &Arc<SyncManager>, project: Id) -> bool {
        let Some(replica) = self.replica(project) else { return false };
        let Some(share) = self.shares.lock().get(project).cloned() else { return false };
        if share.peers.is_empty() {
            return false;
        }

        let mut worked = false;
        let mut answered = false;
        for ticket in &share.peers {
            if self.stopping() {
                return worked;
            }
            self.set_phase(project, SyncPhase::Connecting);
            let session = match self.endpoint().connect_ticket(ticket).await {
                Ok(session) => session,
                Err(e) => {
                    // A peer that is not online is the ordinary state of a
                    // peer-to-peer share with no seed node, so this is a debug
                    // line and not an error: raising a toast every time a
                    // collaborator's laptop is shut would make the app unusable.
                    diag::record(
                        diag::Level::Debug,
                        format!("sync: {} is not answering: {e}", ticket.alias()),
                    );
                    continue;
                }
            };
            answered = true;

            let gate = replica.round.lock().await;
            let endpoint_id = ticket.peer().to_string();
            let alias = ticket.alias();
            self.set_peer(
                project,
                endpoint_id.clone(),
                alias.clone(),
                true,
                false,
                SyncPhase::Syncing,
            );
            let outcome = round::run(self, &replica, &session).await;
            drop(gate);
            session.close();

            let converged = outcome.as_ref().is_ok_and(|outcome| outcome.converged());
            self.set_peer(project, endpoint_id, alias, false, converged, SyncPhase::Idle);

            match outcome {
                Ok(outcome) => worked |= outcome.did_something(),
                Err(e) => diag::error(format!("sync: round with a peer failed: {}", e.message)),
            }
        }
        self.set_phase(
            project,
            if answered { SyncPhase::Idle } else { SyncPhase::Offline },
        );
        worked
    }

    /// The outbound half: dial this share's peers, back off when nothing
    /// happens.
    ///
    /// One task per share rather than one task with a schedule, because the
    /// backoff is per share — a world in step with a collaborator who is asleep
    /// must not slow down the one they are both editing right now.
    fn spawn_poller(self: &Arc<SyncManager>, project: Id) {
        let manager = Arc::downgrade(self);
        let handle = tauri::async_runtime::spawn(async move {
            loop {
                let wait = {
                    let Some(manager) = manager.upgrade() else { return };
                    if manager.stopping() {
                        return;
                    }
                    let Some(replica) = manager.replica(project) else { return };
                    let step = replica.idle.load(Ordering::Relaxed) as usize;
                    Duration::from_secs(BACKOFF[step.min(BACKOFF.len() - 1)])
                    // `manager` dropped here, deliberately: holding a strong
                    // reference across the sleep would keep the endpoint alive
                    // through a shutdown for up to two minutes.
                };
                tokio::time::sleep(wait).await;

                let Some(manager) = manager.upgrade() else { return };
                if manager.stopping() {
                    return;
                }
                let worked = manager.dial_round(project).await;
                if let Some(replica) = manager.replica(project) {
                    if worked {
                        replica.idle.store(0, Ordering::Relaxed);
                    } else {
                        replica.idle.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        self.pollers.lock().push(handle);
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
    }
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
}

/// The accept side, holding a [`Weak`] so that the router does not keep the
/// manager — and therefore the router — alive. See the module documentation.
struct Accepts(Weak<SyncManager>);

impl Projects for Accepts {
    /// Cheap, synchronous, and a bool. The signature is the security boundary:
    /// there is nowhere to put a list, so the accept path cannot disclose one.
    fn admits(&self, project: &Id, grant: Option<&wobu_sync::Grant>) -> bool {
        self.0.upgrade().is_some_and(|m| !m.stopping() && m.admits(project, grant))
    }
}

#[async_trait]
impl Sessions for Accepts {
    /// This future *is* the session's lifetime — iroh drops the connection as
    /// soon as it returns — so the round runs inside it rather than being sent
    /// somewhere. That is also why it is bounded: an accept handler that never
    /// returns is an accept loop that cannot be wound down, which is the
    /// shutdown hang [`SyncManager::shutdown`] is written against.
    async fn opened(&self, session: Session) {
        let Some(manager) = self.0.upgrade() else { return };
        let Some(replica) = manager.replica(session.project()) else {
            session.close();
            return;
        };
        // Refused rather than queued. See the module documentation: making a
        // peer wait would stall it inside a manifest exchange whose idle timeout
        // would then fire, and "busy, try again" is both truer and faster.
        let Ok(gate) = replica.round.try_lock() else {
            session.close();
            return;
        };

        let project = session.project();
        let endpoint_id = session.peer().to_string();
        let alias = wobu_core::peer::alias(session.peer().as_bytes());
        manager.set_peer(
            project,
            endpoint_id.clone(),
            alias.clone(),
            true,
            false,
            SyncPhase::Syncing,
        );

        let outcome =
            tokio::time::timeout(SESSION_BUDGET, round::run(&manager, &replica, &session));
        let converged = match outcome.await {
            Ok(Ok(outcome)) => outcome.converged(),
            Ok(Err(e)) => {
                diag::error(format!("sync: inbound round failed: {}", e.message));
                false
            }
            Err(_elapsed) => {
                diag::error("sync: inbound round ran out of time");
                false
            }
        };
        drop(gate);
        session.close();
        manager.set_peer(project, endpoint_id, alias, false, converged, SyncPhase::Idle);
    }
}

/// The project ULIDs this machine has a folder for, as a value.
///
/// Only [`Ticket::disposition`] consults this, and it is **not** the accept
/// path's answer — see [`SyncManager::present`] for the two questions and why
/// they are different sets.
///
/// A value rather than a borrow for two reasons. [`Projects`] is `'static` — it
/// is stored behind an `Arc<dyn _>` on the accept path, so it has to be — and
/// the manager's own implementation lives on [`Accepts`] behind a `Weak` that a
/// `&self` method cannot produce. And a snapshot cannot see the registry mutate
/// underneath it, which matters because "which worlds do I have" is precisely
/// the question nothing should be able to answer twice differently within one
/// decision.
struct Present(std::collections::BTreeSet<Id>);

impl Projects for Present {
    fn admits(&self, project: &Id, _grant: Option<&wobu_sync::Grant>) -> bool {
        self.0.contains(project)
    }
}

/// A `wobu-sync` failure, as something the webview can read.
///
/// Everything the transport can raise is either "the other machine is not
/// answering" or "the link broke", and both are `io.failed`, which is retryable
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
                Code::Io,
                "The other machine no longer has this project. \
                 A share cannot be taken back, so this means they removed it.",
            ),
            _ => {
                WobuError::new(Code::Io, "Could not reach the other machine.").with_detail(message)
            }
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
