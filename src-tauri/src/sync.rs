//! Peer-to-peer sync, wired into the app.
//!
//! Four issues landed the parts and none of them were connected to anything:
//! #76 put a peer identity in the keychain that nothing loaded, #77 minted
//! tickets nothing persisted, #79 exchanged manifests nothing fed into a
//! `Project`, and #80 landed a three-way apply nothing called. This module is
//! where they meet, and #82 is the issue that says so. Until it existed, the
//! Tauri shell did not depend on `wobu-sync` at all — which is why a conflict
//! sibling was named from `wobu-store`'s unattributed fallback, a name that is
//! per *process* rather than per installation, so a person's conflict files
//! stopped accumulating under one name every time the app restarted.
//!
//! ## What is where
//!
//! - [`manager`] — the `SyncManager`: one endpoint, one replica per shared
//!   project, and the rule about who is holding a project's folder at any
//!   moment. That rule is the one genuine architectural change in the milestone
//!   and its argument lives there.
//! - [`round`] — what two peers do once they have agreed which project they are
//!   about.
//! - [`bodies`] — the one piece of wire that had nowhere else to live, and why.
//! - [`shares`] — the persisted list, and where a credential is kept.
//!
//! ## Why the commands are not in `commands.rs`
//!
//! Every other command in the app is. These are not, and it is worth a sentence
//! rather than looking like an accident: `commands.rs` is two thousand lines of
//! synchronous, project-scoped glue where every function opens with
//! `state.with(...)`. Sync is the opposite shape — asynchronous, not
//! project-scoped, and holding the project mutex is the one thing it must never
//! do across a network step. Putting these there would put the file's one
//! obvious pattern next to five functions that must not follow it.
//!
//! ## Startup order, and the one thing that has to happen synchronously
//!
//! [`SyncState::start`] loads the identity and installs the alias *before* it
//! returns, and only then spawns the bind. That order is load bearing:
//! `wobu_store::peer::install` refuses a second value rather than replacing the
//! first, deliberately — a folder where one conflict sibling says
//! `amber-heron-4f1a` and the next says something else, in one session, is a
//! folder nobody can read back. So the alias has to be in place before any
//! project opens, and binding an endpoint takes a network round trip. One is
//! allowed to be slow; the other is not allowed to be late.
//!
//! Reading the keychain on the main thread at startup is the cost, and on Linux
//! a locked login keyring can put a prompt in front of the user there. That is
//! the trade [`wobu_sync::Identity`] already documents — it is infallible and
//! degrades to an ephemeral identity — and the alternative is worse: an app that
//! is briefly nobody, writing conflict files under a name it will not use again.
//!
//! ## Sync failures do not raise anything
//!
//! A peer that is not answering is the ordinary state of a share with no seed
//! node: both machines have to be online at once, and most of the time they are
//! not. So a failed dial is a debug line, a failed round is an error line in the
//! diagnostics log, and neither is a toast. The only sync failures a person sees
//! are the ones they caused by pressing a button.
//!
//! This is also why nothing here has its own [`Code`](crate::error::Code).
//! `error.rs` owns that taxonomy and every string in it appears in
//! `src/lib/api.ts`; a sync error is either "could not reach the other machine"
//! (`io.failed`, retryable, which is true — they may come back) or "that is not
//! a share link" (`node.invalid`). Adding `sync.*` codes is a change to the
//! frontend contract and belongs with #83's status UI, which is the first thing
//! that will actually branch on one.

pub mod bodies;
pub mod manager;
pub mod round;
pub mod shares;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use wobu_core::Id;
use wobu_sync::{Disposition, Identity, Origin, Reach, Ticket};

use crate::diag;
use crate::error::{Code, CommandResult, WobuError};
use crate::state::{self, AppState};
use manager::{Setup, SyncManager, Wake};
use shares::Shares;

/// How long a share dialog will wait for a relay before minting the ticket
/// anyway.
///
/// [`wobu_sync::SyncEndpoint::ticket`] is emphatic about the ordering: an
/// address collected before a relay has been picked contains direct sockets and
/// nothing else, so the string works on this machine's LAN and nowhere on earth
/// beyond it — and it fails at the far end as a dial timeout, which reads to a
/// user as "the other person is offline".
///
/// Waiting for ever is not the answer either, because a machine with no route to
/// n0's relays will never resolve and the user would be left with a spinner and
/// no string. So: wait, then mint regardless, and report
/// [`Shared::relayed`] so the dialog can say plainly that this link only works
/// on the local network. A control that quietly hands out a broken link is worse
/// than one that says what it is handing out.
const ONLINE_WAIT: Duration = Duration::from_secs(10);

/// The sync manager, once it exists.
///
/// Managed state beside [`AppState`] rather than inside it, for the reason #82
/// opens with: that slot holds exactly one project and only while it is open,
/// and syncing worlds nobody is looking at is the entire feature.
///
/// `Option` because binding is asynchronous and the window is up before it
/// finishes — every command here answers "sync is not running yet" for the first
/// second or so of a session, which is a true statement and a better one than
/// blocking startup on a relay handshake.
///
/// Named `SyncState` and not `Sync` on purpose: a type called `Sync` in scope
/// shadows the marker trait of the same name, and the first symptom is a
/// `T: Send + Sync` bound elsewhere in the file failing to mean what it says.
#[derive(Default)]
pub struct SyncState {
    manager: Arc<Mutex<Option<Arc<SyncManager>>>>,
}

impl SyncState {
    /// Load the identity, name this installation, and start binding.
    ///
    /// See the module documentation for why the first two are synchronous and
    /// the third is not.
    pub fn start(&self, app: &AppHandle, state: AppState) {
        let identity = Identity::load();
        let alias = identity.alias();
        match identity.origin() {
            Origin::Ephemeral => diag::error(format!(
                "sync: the credential store would not answer; syncing as {alias} for this run only"
            )),
            origin => diag::info(format!("sync: this installation is {alias} ({origin:?})")),
        }
        // #76's wiring, and the whole reason the alias is loaded before anything
        // else. A `false` means somebody already installed one — two calls in
        // one process is either a bug or the app being started twice inside it,
        // and both are worth a line.
        if !wobu_store::peer::install(&alias) {
            diag::error("sync: an alias was already installed; conflict siblings may be misnamed");
        }

        let slot = Arc::clone(&self.manager);
        let wake: Arc<dyn Wake> = Arc::new(Window { app: app.clone(), state: state.handle() });
        tauri::async_runtime::spawn(async move {
            let setup =
                Setup { identity, reach: Reach::Internet, shares: Shares::load(), poll: true };
            match SyncManager::start(state, wake, setup).await {
                Ok(manager) => {
                    diag::info(format!(
                        "sync: listening as {} for {} project(s)",
                        manager.identity().alias(),
                        manager.shares().len()
                    ));
                    *slot.lock() = Some(manager);
                }
                // Not fatal, and not a dialog. Everything local still works; what
                // is lost is syncing, and an app that refused to start because a
                // UDP socket would not bind would be worse than one that says so
                // in its log and carries on.
                Err(e) => diag::error(format!("sync: could not start: {}", e.message)),
            }
        });
    }

    fn get(&self) -> Option<Arc<SyncManager>> {
        self.manager.lock().clone()
    }

    /// The manager, or a readable failure.
    fn manager(&self) -> CommandResult<Arc<SyncManager>> {
        self.get().ok_or_else(|| {
            WobuError::new(Code::Io, "Sync is still starting up. Try again in a moment.")
        })
    }

    /// Wind sync down, from the synchronous world.
    ///
    /// Called from Tauri's `RunEvent::Exit`, which runs on the main thread with
    /// no locks held — and both halves of that matter. The main thread is where
    /// a hang is visible as a window that will not close; "no locks held" is
    /// what stops the shutdown waiting on a round that is waiting on the project
    /// mutex. See [`SyncManager::shutdown`] for the rest of the argument.
    ///
    /// The manager is taken out of the slot first, so a second exit event does
    /// nothing rather than shutting down an endpoint that is already closed.
    pub fn stop(&self) {
        let Some(manager) = self.manager.lock().take() else { return };
        tauri::async_runtime::block_on(async move { manager.shutdown().await });
    }
}

/// `world:changed`, but only for the world somebody is looking at.
///
/// A background round against a closed project changes files nobody has on
/// screen, and the window's next `Project::open` rescans the folder anyway — so
/// emitting for one would be an invalidation with nothing to invalidate. The
/// check is by project id rather than "is anything open", because the round
/// decided which world it was about when the connection opened and the user may
/// have moved on since.
struct Window {
    app: AppHandle,
    state: AppState,
}

impl Wake for Window {
    fn world_changed(&self, project: Id) {
        if self.state.open_id() == Some(project) {
            // Emission failures are dropped deliberately: the window may be on
            // its way out, and a sync that cannot announce itself is not a sync
            // that should stop working.
            let _ = self.app.emit(state::WORLD_CHANGED, ());
        }
    }
}

/* ── what the webview sees ────────────────────────────────────────────────── */

/// This installation, as a person reads it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// Whether the endpoint has finished binding.
    pub running: bool,
    /// `amber-heron-4f1a`. **Display only** — twenty-eight bits is a name and
    /// not a key, and anything deciding anything compares `endpointId`.
    pub alias: String,
    /// The full 64-hex public key, which is also this peer's TLS certificate.
    pub endpoint_id: String,
    /// Whether the identity survives a restart. `false` means the credential
    /// store would not answer and collaborators will see a different peer next
    /// time, which is worth saying out loud rather than discovering.
    pub persistent: bool,
    pub shares: Vec<SharedProject>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedProject {
    pub project: Id,
    pub root: String,
    /// How many peers this machine knows how to dial about this project.
    pub peers: usize,
    /// Whether this is the project currently on screen.
    pub open: bool,
}

/// A minted ticket, and the one thing a share dialog has to check before it lets
/// the string be copied.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shared {
    pub project: Id,
    /// The pasteable string. **This is a credential** — see
    /// [`wobu_sync::ticket`] — and it must not be written into the project
    /// folder, logged, or included in a diagnostics bundle.
    pub token: String,
    /// `false` means this link names direct socket addresses only, so it works
    /// on this machine's network and nowhere else. The dialog has to say so; a
    /// link that fails as a timeout at the far end reads to the recipient as
    /// "they are offline".
    pub relayed: bool,
    /// This peer's short name, so the person being sent the string can be told
    /// which machine it names without reading sixty-four hex characters.
    pub alias: String,
}

/// What accepting a ticket did, or what it needs.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Accepted {
    pub project: Id,
    /// The peer's short name, for showing who was just added.
    pub alias: String,
    /// `true` when this machine already held the project and is now syncing the
    /// replica that is here.
    ///
    /// `false` means there is nothing here to join and the world has to be
    /// cloned into a folder the user picks — which this command deliberately
    /// does not do, because a background task must not choose where somebody's
    /// world lives. Cloning is #85's.
    pub joined: bool,
}

/* ── commands ─────────────────────────────────────────────────────────────── */

/// Who this installation is, and what it is syncing.
#[tauri::command]
pub async fn sync_status(sync: State<'_, SyncState>) -> CommandResult<SyncStatus> {
    let Some(manager) = sync.get() else {
        return Ok(SyncStatus {
            running: false,
            alias: String::new(),
            endpoint_id: String::new(),
            persistent: false,
            shares: Vec::new(),
        });
    };
    Ok(SyncStatus {
        running: true,
        alias: manager.identity().alias(),
        endpoint_id: manager.identity().id().to_string(),
        persistent: manager.identity().origin() != Origin::Ephemeral,
        shares: manager
            .shares()
            .into_iter()
            .map(|share| SharedProject {
                // Asked of the replica rather than compared against
                // `AppState::open_id`, because the replica's answer is the one
                // that decides where a round's writes go — and a status that
                // could disagree with it would be a status that lies about
                // exactly the race #82 exists to close.
                open: manager.replica(share.project).is_some_and(|r| r.is_open()),
                project: share.project,
                root: share.root.to_string_lossy().into_owned(),
                peers: share.peers.len(),
            })
            .collect(),
    })
}

/// Share the open project: mint the string to paste to somebody.
///
/// The open project rather than a project id argument, and that is a decision
/// about what a share *is*: sharing a world means handing over a folder on this
/// machine, and the only folder the app can be sure it knows the path of is the
/// one it currently has open. A project id argument would invite the launcher to
/// share a recents entry whose path has since gone stale.
#[tauri::command]
pub async fn sync_share(
    sync: State<'_, SyncState>,
    state: State<'_, AppState>,
) -> CommandResult<Shared> {
    let manager = sync.manager()?;
    let open: Option<(Id, PathBuf)> =
        state.peek(|project| project.map(|p| (p.id(), p.root().to_path_buf())));
    let (project, root) = open.ok_or_else(WobuError::no_project_open)?;

    // Bounded, and the result is deliberately ignored: see `ONLINE_WAIT`. A
    // machine that cannot reach a relay still gets a string, and `relayed` is
    // what tells the user what kind of string it is.
    let _ = tokio::time::timeout(ONLINE_WAIT, manager.endpoint().online()).await;

    let ticket = manager.share(project, &root);
    if !ticket.is_relayed() {
        diag::info("sync: minted a ticket with no relay in it; it will not work off this network");
    }
    Ok(Shared {
        project,
        token: ticket.to_string(),
        relayed: ticket.is_relayed(),
        alias: manager.identity().alias(),
    })
}

/// Accept a ticket somebody pasted.
#[tauri::command]
pub async fn sync_accept(token: String, sync: State<'_, SyncState>) -> CommandResult<Accepted> {
    let manager = sync.manager()?;
    let ticket: Ticket = token.parse().map_err(WobuError::from)?;
    let alias = ticket.alias();
    let project = ticket.project();

    let joined = manager.accept(&ticket) == Disposition::Join;
    if joined {
        diag::info(format!("sync: joined {project} with {alias}"));
    }
    Ok(Accepted { project, alias, joined })
}

/// Stop syncing a project, and forget everything ever agreed about it.
#[tauri::command]
pub async fn sync_unshare(project: Id, sync: State<'_, SyncState>) -> CommandResult<()> {
    sync.manager()?.unshare(project)
}

#[cfg(test)]
pub mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use wobu_core::new_id;
    use wobu_store::Project;
    use wobu_sync::{Grant, Projects, SyncEndpoint};

    use super::*;
    use crate::state::Handover;
    use bodies::Request;

    /// A private directory per test. `tempfile` is not a dependency of this
    /// crate and adding one to `[dev-dependencies]` for four tests is not worth
    /// the supply chain.
    pub fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wobu-{name}-{}", new_id()));
        std::fs::create_dir_all(&dir).expect("a temp directory");
        dir
    }

    /// Counts `world:changed` instead of emitting it, because an `AppHandle`
    /// cannot be constructed outside a running Tauri app — which is the whole
    /// reason [`Wake`] is a trait.
    #[derive(Default)]
    struct Counter(AtomicUsize);

    impl Wake for Counter {
        fn world_changed(&self, _project: Id) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// A manager on the loopback interface, with its own share file.
    ///
    /// `Reach::Loopback` is one socket on `127.0.0.1` with no relay and no
    /// address lookup, so nothing in this file can quietly start depending on
    /// n0's infrastructure. What it therefore cannot exercise is real and is not
    /// implied by any of this passing: NAT traversal, holepunching, relay
    /// selection, and a network where the relay is blocked. Those need two
    /// machines.
    async fn manager(state: &AppState, dir: &Path) -> Arc<SyncManager> {
        SyncManager::start(
            state.handle(),
            Arc::new(Counter::default()),
            Setup {
                identity: Identity::ephemeral(),
                reach: Reach::Loopback,
                shares: Shares::load_from(dir.join("shares.json")),
                // No dialling: these tests are about the manager, and a poller
                // reaching for a ticket nobody minted is a task to shut down for
                // no reason and noise in the log.
                poll: false,
            },
        )
        .await
        .expect("a loopback endpoint binds without a network")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_manager_binds_holds_the_router_and_gives_it_back_without_hanging() {
        // The trap the crate documentation names: `SyncEndpoint` holds iroh's
        // `Router`, which aborts its accept loop on drop — so the app has to
        // hold one and has to call `shutdown` rather than letting it fall out of
        // scope. This is the smallest statement of that lifecycle, and the
        // assertion is the elapsed time: a shutdown that hangs is the one
        // failure mode a green test would otherwise hide behind a CI timeout.
        let dir = scratch("sync-lifecycle");
        let state = AppState::default();
        let manager = manager(&state, &dir).await;

        assert!(!manager.stopping());
        assert_eq!(manager.identity().alias(), manager.endpoint().alias());

        let started = std::time::Instant::now();
        manager.shutdown().await;
        assert!(manager.stopping());
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());

        // Idempotent, because the exit path can be reached twice and a second
        // shutdown must not be a second wait on the deadline.
        let again = std::time::Instant::now();
        manager.shutdown().await;
        assert!(again.elapsed() < Duration::from_secs(1), "{:?}", again.elapsed());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_ticket_for_a_project_already_here_joins_rather_than_cloning() {
        // One project ULID is one world however many folders it is sitting in.
        // Cloning a project this machine already holds would leave two replicas
        // syncing against each other on one disk — which is not a share, it is a
        // bug with a progress bar. The check is against what the manager holds,
        // not against a path, because "already held" is a fact about the id.
        let dir = scratch("sync-join");
        let state = AppState::default();
        let manager = manager(&state, &dir).await;

        let mine = new_id();
        let theirs = new_id();
        manager.share(mine, &dir.join("Ashfall.wobu"));

        let peer =
            SyncEndpoint::bind(wobu_sync::Config::loopback(), Arc::new(Nothing), Arc::new(Nothing))
                .await
                .unwrap();

        let held = peer.ticket(mine, Grant::generate());
        let unknown = peer.ticket(theirs, Grant::generate());

        assert_eq!(manager.accept(&held), Disposition::Join);
        assert_eq!(manager.accept(&unknown), Disposition::Clone);

        // Joining recorded the peer to dial; cloning did not invent a share for
        // a world this machine has no folder for.
        let shares = manager.shares();
        assert_eq!(shares.len(), 1, "{shares:?}");
        assert_eq!(shares[0].project, mine);
        assert_eq!(shares[0].peers.len(), 1);
        assert_eq!(shares[0].peers[0].peer(), peer.id());

        // …and it survives a restart, because a share that had to be re-accepted
        // on every launch would not be a share.
        let reloaded = Shares::load_from(dir.join("shares.json"));
        assert_eq!(reloaded.all().len(), 1);
        assert_eq!(reloaded.get(mine).unwrap().peers.len(), 1);

        manager.shutdown().await;
        peer.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_shared_project_is_admitted_and_one_this_machine_never_saw_is_refused() {
        // The accept path, which is the security-relevant one. A peer that dials
        // with a guessed ULID must learn whether that guess was right and
        // nothing else — `Projects::admits` takes one project and optional grant
        // and returns one bool, so it has no way to form the other sentence.
        // This checks the real manager policy rather than a crate-only fake.
        //
        // A shut-down manager refuses everything, and that is not belt and
        // braces: `Router::shutdown` winds the accept loop down, but admitting a
        // connection already in flight would start a round against a project the
        // app is on its way out of.
        let dir = scratch("sync-admission");
        let state = AppState::default();
        let manager = manager(&state, &dir).await;

        let held = new_id();
        let ticket = manager.share(held, &dir.join("Ashfall.wobu"));

        let dialler =
            SyncEndpoint::bind(wobu_sync::Config::loopback(), Arc::new(Nothing), Arc::new(Nothing))
                .await
                .unwrap();

        let admitted = dialler.connect_ticket(&ticket).await;
        assert!(admitted.is_ok(), "a shared project was refused: {admitted:?}");
        // Closed straight away: the round on the far side has nothing to talk
        // to, and this test is about who gets in rather than what they do.
        admitted.unwrap().close();

        let unknown = wobu_sync::Ticket::new(new_id(), ticket.addr().clone(), ticket.grant());
        let refused = dialler.connect_ticket(&unknown).await;
        assert!(matches!(refused, Err(wobu_sync::Error::ProjectNotHeld)), "{refused:?}");

        let forged = wobu_sync::Ticket::new(held, ticket.addr().clone(), Grant::generate());
        let refused = dialler.connect_ticket(&forged).await;
        assert!(matches!(refused, Err(wobu_sync::Error::ProjectNotHeld)), "{refused:?}");

        let refused = dialler.connect(ticket.addr().clone(), held).await;
        assert!(matches!(refused, Err(wobu_sync::Error::ProjectNotHeld)), "{refused:?}");

        manager.shutdown().await;
        // Bounded rather than awaited to its natural end: a dial at a closed
        // endpoint sits in iroh's own connect timeout, which is half a minute,
        // and "did not get in within three seconds" is the whole of what this
        // asserts. A timeout here is a pass — it is the peer not getting in.
        let after =
            tokio::time::timeout(Duration::from_secs(3), dialler.connect_ticket(&ticket)).await;
        assert!(!matches!(after, Ok(Ok(_))), "a shut-down manager admitted somebody: {after:?}");

        dialler.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn opening_a_project_takes_the_folder_off_sync_and_closing_gives_it_back() {
        // The #82 race, stated as the only thing that actually has to be true:
        // there is exactly one holder of a project at a time. Two `Project`
        // values for one ULID would be two writers to one SQLite index and two
        // caches of what is on disk — not a corruption of bytes, which would be
        // noticed, but of meaning, which would not.
        let dir = scratch("sync-handover");
        let state = AppState::default();
        let manager = manager(&state, &dir).await;

        let project = Project::create(&dir, "Ashfall").expect("a project in a temp directory");
        let id = project.id();
        let root = project.root().to_path_buf();
        // Dropped before sync is allowed near it, and that is the test obeying
        // its own invariant: this handle and the one the replica opens below
        // would be two connections to one index, which is exactly what the
        // handover exists to prevent.
        drop(project);
        manager.share(id, &root);

        let replica = manager.replica(id).expect("sharing registered a replica");
        assert!(!replica.is_open(), "sync should hold a project nobody opened");

        // Sync takes it, which opens its own handle.
        replica.with(|p| Ok(p.manifest()?)).expect("sync can read a project it holds");

        manager.opening(id, &root);
        assert!(replica.is_open(), "the window did not take the folder");

        manager.closing(id);
        assert!(!replica.is_open(), "sync did not get the folder back");

        // A different project being opened must sweep a stale `Open` mark, or a
        // replica would keep routing through a slot that holds somebody else.
        manager.opening(id, &root);
        manager.opening(new_id(), &dir);
        assert!(!replica.is_open(), "a stale `Open` survived a different project opening");

        manager.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_round_serves_what_a_peer_asks_for_and_pushes_what_it_is_behind_on() {
        // The one test that runs a whole round against real QUIC, and the only
        // thing standing between this milestone and a protocol bug that appears
        // on somebody else's machine.
        //
        // Two full managers cannot be stood up here, and the reason is worth
        // recording: the SQLite index is keyed by project ULID under
        // `app_data_dir()`, so two replicas of one project in one process share
        // one index — the exact thing the handover exists to prevent. So one
        // side is the app, and the other is a peer driven by hand from this
        // test: it dials, swaps manifests, asks, answers, and says when it is
        // done. If the round's termination handshake were wrong in either
        // direction, this hangs.
        let dir = scratch("sync-round");
        let state = AppState::default();
        let manager = manager(&state, &dir).await;

        let node = {
            let mut project = Project::create(&dir, "Ashfall").expect("a project");
            let node = project
                .create_node(wobu_core::NodeKind::Character, "Kael Vantris", None)
                .expect("a node");
            (project.id(), project.root().to_path_buf(), node.id)
            // …and the handle goes out of scope here, so the replica below is
            // the only one holding this index.
        };
        let (project, root, node_id) = node;
        let ticket = manager.share(project, &root);

        let peer =
            SyncEndpoint::bind(wobu_sync::Config::loopback(), Arc::new(Nothing), Arc::new(Nothing))
                .await
                .unwrap();
        let session = peer.connect_ticket(&ticket).await.expect("admitted");

        // The peer's half of the manifest exchange: it holds nothing. Under the
        // rule `wobu-sync` states twice, that is "never had it" and not
        // "deleted", so the app's side plans to send rather than to remove.
        let exchange =
            wobu_sync::manifest::exchange(&session, &[], &[], wobu_sync::manifest::IDLE_TIMEOUT)
                .await
                .expect("both sides swap manifests");
        assert!(exchange.is_whole());
        // A fresh project is not empty — `Project::create` seeds it — so this
        // asks whether the node reached the manifest rather than counting.
        assert!(
            exchange.nodes.iter().any(|(id, _)| *id == node_id),
            "the app did not announce its node: {:?}",
            exchange.nodes
        );
        let announced = exchange.nodes.len();

        let connection = session.connection();
        // Both halves at once, exactly as `round::run` does it — a peer that
        // asked everything before answering anything would deadlock against an
        // app doing the same, and this is what proves it does not.
        let asking = async {
            let bodies = bodies::want(connection, &[node_id]).await?;
            bodies::done(connection).await?;
            CommandResult::Ok(bodies)
        };
        let answering = async {
            let mut pushed: Vec<Id> = Vec::new();
            loop {
                let (mut send, request) = bodies::accept(connection).await?;
                match request {
                    // The app has nothing to fetch — this peer announced an
                    // empty manifest — but it is entitled to ask, and an
                    // unanswered request is a round that times out.
                    Request::Want(_) => bodies::bodies(&mut send, &[]).await?,
                    Request::Give(nodes) => {
                        pushed.extend(nodes.iter().map(|n| n.node_id));
                        let ids: Vec<Id> = nodes.iter().map(|n| n.node_id).collect();
                        bodies::agreed(&mut send, &ids).await?;
                    }
                    Request::Done => {
                        bodies::finished(&mut send).await?;
                        return CommandResult::Ok(pushed);
                    }
                }
            }
        };

        let (fetched, pushed) = tokio::try_join!(asking, answering).expect("a round completes");

        assert_eq!(fetched.len(), 1, "the app did not serve exactly what was asked for");
        assert_eq!(fetched[0].node_id, node_id);
        assert!(fetched[0].text.contains("Kael Vantris"), "{}", fetched[0].text);
        assert_eq!(fetched[0].slug, "kael-vantris");

        // Everything the peer announced nothing for. An absence is "never had
        // it", so the whole project is behind, and the app offers all of it —
        // that is the same rule that makes an empty manifest safe.
        assert!(pushed.contains(&node_id), "the app did not push a node the peer lacked");
        assert_eq!(pushed.len(), announced, "{pushed:?}");

        session.close();
        manager.shutdown().await;
        peer.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An endpoint that holds nothing and does nothing with what it accepts.
    struct Nothing;

    impl Projects for Nothing {
        fn admits(&self, _project: &Id, _grant: Option<&wobu_sync::Grant>) -> bool {
            false
        }
    }

    #[async_trait::async_trait]
    impl wobu_sync::Sessions for Nothing {
        async fn opened(&self, session: wobu_sync::Session) {
            session.close();
        }
    }
}
