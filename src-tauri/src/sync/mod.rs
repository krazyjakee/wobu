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
//! ## Startup order, and the one thing that has to happen before a project opens
//!
//! `wobu_store::peer::install` refuses a second value rather than replacing the
//! first, deliberately — a folder where one conflict sibling says
//! `amber-heron-4f1a` and the next says something else, in one session, is a
//! folder nobody can read back. So the alias has to be in place before any
//! project opens, and binding an endpoint takes a network round trip. One is
//! allowed to be slow; the other is not allowed to be late.
//!
//! [`SyncState::start`] used to satisfy that by loading the identity on the main
//! thread before returning from `setup`, and the cost was stated as a prompt in
//! front of the user. The real cost was worse. Reading the keychain is not
//! merely slow on Linux — against a locked collection it can never return at
//! all (`wobu_sync::identity` sets out why), and every millisecond of it was
//! spent on the thread that has not yet created the window. The result was not a
//! slow start or a prompt; it was a process with no UI, no diagnostic past
//! `wobu <version> starting`, and nothing for the user to click. Reported as a
//! crash on startup, which from the outside is exactly what it was.
//!
//! So the load happens on a blocking thread now and the window comes up without
//! it. What replaces the ordering is [`SyncState::wait_until_named`]: the two
//! commands that can adopt a project folder await it first, so nothing can write
//! a conflict sibling before this installation knows its own name. The
//! invariant is unchanged — it is enforced at the point that needs it rather
//! than by freezing everything until it holds. Bounded, too, because
//! `Identity::load` is: the gate opens within `STORE_DEADLINE` of launch
//! whatever the credential store does.
//!
//! What is *not* done is opening the window first and naming the peer later
//! without a gate. An app that is briefly nobody, writing conflict files under a
//! name it will not use again, is the bug #76 exists to fix.
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
//! `src/lib/api/call.ts`; a sync error is either "could not reach the other
//! machine" (`sync.unreachable`, retryable, which is true — they may come back)
//! or "that is not a share link" (`node.invalid`). Keeping transport failures
//! out of `io.failed` matters because that code's UI guidance is specifically
//! about local disk space and project-folder permissions.

pub mod bodies;

pub mod manager;

pub mod round;

pub mod shares;

/// Turning an accepted ticket into a project folder on this machine.
mod clone;

#[cfg(test)]
mod integration;

#[cfg(test)]
pub mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{Notify, Semaphore};
use wobu_core::{Id, SCHEMA_VERSION};
use wobu_store::ProjectMeta;
use wobu_sync::{Disposition, Identity, Origin, Reach, Ticket};

use self::clone::accept_ticket;
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
    accepting: Arc<Mutex<Option<Arc<Notify>>>>,
    named: Arc<Named>,
}

/// Whether this installation knows its own name yet.
///
/// A semaphore that is closed rather than a flag beside a `Notify`, and the
/// difference is a race: a flag has to be checked and then awaited, and a
/// settle landing between those two is a wake nobody receives. `acquire` on a
/// closed semaphore fails immediately and for ever, which is the exact shape of
/// "settled once, never again" with no window to fall into.
struct Named(Semaphore);

impl Default for Named {
    fn default() -> Named {
        // Zero permits, and none are ever added. Closing is the whole signal.
        Named(Semaphore::new(0))
    }
}

impl Named {
    fn settle(&self) {
        self.0.close();
    }

    async fn wait(&self) {
        // Always `Err(AcquireError)` — there is no permit to acquire and the
        // close is what ends the wait.
        let _ = self.0.acquire().await;
    }
}

struct AcceptLease {
    slot: Arc<Mutex<Option<Arc<Notify>>>>,
    cancel: Arc<Notify>,
}

impl AcceptLease {
    fn cancelled(&self) -> tokio::sync::futures::Notified<'_> {
        self.cancel.notified()
    }
}

impl Drop for AcceptLease {
    fn drop(&mut self) {
        let mut active = self.slot.lock();
        if active.as_ref().is_some_and(|current| Arc::ptr_eq(current, &self.cancel)) {
            *active = None;
        }
    }
}

impl SyncState {
    /// Load the identity, name this installation, and start binding.
    ///
    /// Returns immediately; none of the three has happened yet. See the module
    /// documentation for why that is safe — [`Self::wait_until_named`] is what
    /// carries the ordering the synchronous version used to carry by blocking.
    pub fn start(&self, app: &AppHandle, state: AppState) {
        let slot = Arc::clone(&self.manager);
        let named = Arc::clone(&self.named);
        let wake: Arc<dyn Wake> = Arc::new(Window { app: app.clone(), state: state.handle() });
        tauri::async_runtime::spawn(async move {
            // `spawn_blocking` and not an ordinary await point: `Identity::load`
            // talks to the OS credential store on the calling thread, and a
            // locked Linux keyring can hold it for the full `STORE_DEADLINE`.
            // On a runtime worker that would stall every other task the app has,
            // which at startup is most of them.
            let identity = tauri::async_runtime::spawn_blocking(Identity::load)
                .await
                // The pool thread died, which is not a reason to have no name.
                // The ephemeral identity is the same answer a refusing store
                // would have produced, and the line below already says so.
                .unwrap_or_else(|_| Identity::ephemeral());
            let alias = identity.alias();
            match identity.origin() {
                Origin::Ephemeral => diag::error(format!(
                    "sync: the credential store would not answer; syncing as {alias} for this run only"
                )),
                origin => diag::info(format!("sync: this installation is {alias} ({origin:?})")),
            }
            // #76's wiring, and the whole reason a project open waits on this
            // task. A `false` means somebody already installed one — two calls
            // in one process is either a bug or the app being started twice
            // inside it, and both are worth a line.
            if !wobu_store::peer::install(&alias) {
                diag::error(
                    "sync: an alias was already installed; conflict siblings may be misnamed",
                );
            }
            // Before the bind rather than after it, and that is the whole point
            // of the split: naming this installation is the part a project open
            // has to wait for, and binding an endpoint is the part it must not.
            named.settle();

            let setup = Setup {
                identity,
                reach: Reach::Internet,
                shares: Shares::load(),
                poll: true,
                index_dir: None,
            };
            match SyncManager::start(state, wake, setup).await {
                Ok(manager) => {
                    diag::info(format!(
                        "sync: listening as {} for {} project(s)",
                        manager.identity().alias(),
                        manager.shares().len()
                    ));
                    *slot.lock() = Some(Arc::clone(&manager));
                    manager.announce();
                }
                // Not fatal, and not a dialog. Everything local still works; what
                // is lost is syncing, and an app that refused to start because a
                // UDP socket would not bind would be worse than one that says so
                // in its log and carries on.
                Err(e) => diag::error(format!("sync: could not start: {}", e.message)),
            }
        });
    }

    /// Wait until this installation has a peer alias installed.
    ///
    /// Every command that can adopt a project folder calls this first, and that
    /// is the ordering the module documentation describes: `peer::install`
    /// latches, so a conflict sibling written before it lands would be stamped
    /// with `wobu-store`'s per-process fallback name and could never be
    /// attributed to this person again.
    ///
    /// Not a hang risk even where the credential store is one. `Identity::load`
    /// is bounded, so [`Self::start`]'s task always reaches `settle` — the
    /// ceiling on this wait is `STORE_DEADLINE` after launch, and only for a
    /// user who reaches a project folder inside it.
    ///
    /// Returns immediately once settled, for ever after.
    pub async fn wait_until_named(&self) {
        self.named.wait().await;
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

    fn begin_accept(&self) -> CommandResult<AcceptLease> {
        let mut accepting = self.accepting.lock();
        if accepting.is_some() {
            return Err(WobuError::new(
                Code::AlreadyExists,
                "Another shared project is already being accepted.",
            ));
        }
        let cancel = Arc::new(Notify::new());
        *accepting = Some(Arc::clone(&cancel));
        drop(accepting);
        Ok(AcceptLease { slot: Arc::clone(&self.accepting), cancel })
    }

    fn cancel_accept(&self) {
        if let Some(cancel) = self.accepting.lock().as_ref() {
            // `notify_one` stores a permit if the accept task has not reached
            // its select yet, closing the immediate Cancel race.
            cancel.notify_one();
        }
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

    fn sync_state(&self, status: ProjectSyncStatus) {
        if self.state.open_id() == Some(status.project) {
            let _ = self.app.emit(SYNC_STATE, status);
        }
    }

    fn sync_peer(&self, status: ProjectSyncStatus) {
        if self.state.open_id() == Some(status.project) {
            let _ = self.app.emit(SYNC_PEER, status);
        }
    }
}

/* ── what the webview sees ────────────────────────────────────────────────── */

pub const SYNC_STATE: &str = "sync:state";

pub const SYNC_PEER: &str = "sync:peer";

/// What the endpoint is doing for one project right now.
///
/// `Offline` is intentionally narrow: a poll just tried every known peer and
/// none answered. It does not claim that this computer has no network route,
/// which the endpoint cannot know.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncPhase {
    #[default]
    Idle,
    Connecting,
    Syncing,
    Offline,
}

/// One peer the manager has actually learned about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPeerStatus {
    /// The TLS identity. Aliases are for display and are not unique.
    pub endpoint_id: String,
    pub alias: String,
    /// A live sync round, not a promise that the peer will remain reachable.
    pub connected: bool,
    /// RFC 3339, written only after a complete round with no parked or refused
    /// node. `None` is more truthful than calling a partial exchange synced.
    pub last_converged_at: Option<String>,
}

/// The event payload and the catch-up value use exactly the same shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSyncStatus {
    pub project: Id,
    pub state: SyncPhase,
    pub peers: Vec<SyncPeerStatus>,
    /// Whether this replica is a clone whose first round never finished.
    ///
    /// Separate from [`SyncPhase`] because it is not a phase: it outlives the
    /// connection, outlives the app, and is true of a folder rather than of a
    /// moment. A world that is half here reads as `idle · offline` otherwise —
    /// which is the truth about the socket and a lie about the world.
    pub arriving: bool,
}

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
    /// Runtime state is separate from `shares`: a creator learns an inbound
    /// peer's identity from its authenticated session, not from a ticket it
    /// could have listed before that peer ever connected.
    pub projects: Vec<ProjectSyncStatus>,
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
    /// The local folder to open after a join or completed clone. Absent on the
    /// first, destination-less probe for a project this machine does not hold.
    pub root: Option<String>,
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
            projects: Vec::new(),
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
        projects: manager.project_statuses(),
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
///
/// A destination-less call is a cheap, non-mutating probe for the launcher. If
/// the project is already present it joins that replica immediately. Otherwise
/// the launcher asks where to put a clone and calls again with that parent
/// directory. `cancel` deliberately travels through this same registered
/// command so a second invocation can stop the first while it is awaiting the
/// peer.
#[tauri::command]
pub async fn sync_accept(
    token: Option<String>,
    destination: Option<String>,
    cancel: Option<bool>,
    sync: State<'_, SyncState>,
) -> CommandResult<Option<Accepted>> {
    accept_ticket(&sync, token, destination, cancel).await
}

/// Stop syncing a project, and forget everything ever agreed about it.
#[tauri::command]
pub async fn sync_unshare(project: Id, sync: State<'_, SyncState>) -> CommandResult<()> {
    sync.manager()?.unshare(project)
}
