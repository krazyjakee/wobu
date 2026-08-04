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

#[cfg(test)]
mod integration;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Notify;
use wobu_core::{Id, SCHEMA_VERSION};
use wobu_store::ProjectMeta;
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
    accepting: Arc<Mutex<Option<Arc<Notify>>>>,
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
    if cancel.unwrap_or(false) {
        sync.cancel_accept();
        return Ok(None);
    }
    let manager = sync.manager()?;
    let token = token.ok_or_else(|| WobuError::new(Code::Invalid, "Paste a Wobu share link."))?;
    let ticket: Ticket = token.parse().map_err(WobuError::from)?;
    let alias = ticket.alias();
    let project = ticket.project();

    if manager.accept(&ticket) == Disposition::Join {
        diag::info(format!("sync: joined {project} with {alias}"));
        let root = manager.root_of(project).map(|path| path.to_string_lossy().into_owned());
        return Ok(Some(Accepted { project, alias, joined: true, root }));
    }

    let Some(destination) = destination else {
        return Ok(Some(Accepted { project, alias, joined: false, root: None }));
    };
    // The lease is RAII: every return path, including scaffold validation,
    // releases the operation slot. A Cancel during this synchronous step is a
    // stored Notify permit when the network wait starts.
    let accept = sync.begin_accept()?;
    let scaffold = create_clone_scaffold(Path::new(&destination), project)?;
    let root = scaffold.root.clone();
    manager.accept_clone(&ticket, &root);

    let downloaded = tokio::select! {
        result = manager.run_ticket(project, &ticket) => result,
        () = accept.cancelled() => Err(WobuError::new(Code::Cancelled, "Accepting the shared project was cancelled.")),
    };
    match downloaded {
        Ok(()) => {
            scaffold.complete();
            manager.start_poller(project);
            Ok(Some(Accepted {
                project,
                alias,
                joined: false,
                root: Some(root.to_string_lossy().into_owned()),
            }))
        }
        Err(error) => {
            cleanup_clone(&manager, project, &root);
            Err(error)
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloneMarker {
    project: Id,
    nonce: Id,
}

struct CloneScaffold {
    root: PathBuf,
    marker: PathBuf,
}

impl CloneScaffold {
    fn complete(self) {
        if let Err(error) = std::fs::remove_file(&self.marker) {
            diag::error(format!(
                "sync: could not remove completed clone marker {}: {error}",
                self.marker.display()
            ));
        }
    }
}

fn create_clone_scaffold(parent: &Path, project: Id) -> CommandResult<CloneScaffold> {
    if !parent.is_dir() {
        return Err(WobuError::new(
            Code::Invalid,
            "Choose an existing destination folder for the shared project.",
        ));
    }
    let short = project.to_string().chars().take(8).collect::<String>().to_lowercase();
    let root = parent.join(format!("shared-{short}.wobu"));
    let created_root = match std::fs::create_dir(&root) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => return Err(wobu_store::Error::io(&root, error).into()),
    };
    let metadata =
        std::fs::symlink_metadata(&root).map_err(|error| wobu_store::Error::io(&root, error))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(WobuError::new(
            Code::Invalid,
            "The clone destination is not a regular folder.",
        ));
    }
    let marker = root.join(".wobu/accepting.json");
    let created = (|| -> wobu_store::Result<()> {
        let path = root.join("project.json");
        if !created_root {
            // Validate ownership before creating even one child. An unrelated
            // collision, including one with hostile child symlinks, remains
            // byte-for-byte untouched when refused.
            let marker_bytes =
                std::fs::read(&marker).map_err(|error| wobu_store::Error::io(&marker, error))?;
            let clone_marker: CloneMarker = serde_json::from_slice(&marker_bytes)?;
            if clone_marker.project != project {
                return Err(wobu_store::Error::AlreadyExists(root.clone()));
            }
            let existing: ProjectMeta = serde_json::from_slice(
                &std::fs::read(&path).map_err(|error| wobu_store::Error::io(&path, error))?,
            )?;
            if existing.id != project {
                return Err(wobu_store::Error::AlreadyExists(root.clone()));
            }
        }
        for rel in [
            "nodes",
            "assets/originals",
            "assets/thumbs",
            "generations",
            ".wobu/tmp",
            ".wobu/sessions",
        ] {
            wobu_store::paths::ensure_dir(&root.join(rel))?;
        }
        let meta = ProjectMeta {
            id: project,
            name: format!("Shared project {short}"),
            schema_version: SCHEMA_VERSION,
            created_at: chrono::Utc::now(),
            providers: serde_json::Map::new(),
            // Match the store's default for a newly created project. The
            // canonical metadata is not part of the node-sync protocol yet.
            spend_ceiling_usd_micros: Some(10_000_000),
        };
        if created_root {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|error| wobu_store::Error::io(&path, error))?;
            file.write_all(&serde_json::to_vec_pretty(&meta)?)
                .map_err(|error| wobu_store::Error::io(&path, error))?;
            file.sync_all().map_err(|error| wobu_store::Error::io(&path, error))?;
            let clone_marker = CloneMarker { project, nonce: wobu_core::new_id() };
            std::fs::write(&marker, serde_json::to_vec_pretty(&clone_marker)?)
                .map_err(|error| wobu_store::Error::io(&marker, error))?;
        }
        Ok(())
    })();
    if let Err(error) = created {
        // Never recursively delete here. A person, another process, or a
        // completed atomic sync write may already have placed recoverable data
        // in this path. Only a verified marker may resume it.
        return Err(error.into());
    }
    Ok(CloneScaffold { root, marker })
}

fn cleanup_clone(manager: &SyncManager, project: Id, root: &Path) {
    if let Err(error) = manager.unshare(project) {
        diag::error(format!(
            "sync: could not discard cancelled clone registration: {}",
            error.message
        ));
    }
    // Keep the marker and every downloaded file. Cancellation can land after
    // an atomic node write; recursively deleting the directory would turn
    // Cancel into data loss. Selecting the same parent later resumes only after
    // marker and project-id validation.
    diag::info(format!("sync: kept resumable partial clone at {}", root.display()));
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

    #[test]
    fn a_scaffold_failure_does_not_poison_the_next_accept() {
        let state = SyncState::default();
        let missing = scratch("missing-accept-parent").join("not-there");
        let active = state.begin_accept().expect("an accept starts");
        assert!(create_clone_scaffold(&missing, new_id()).is_err());
        drop(active);

        assert!(state.begin_accept().is_ok(), "the accept slot stayed occupied");
    }

    #[tokio::test]
    async fn cancel_before_the_accept_waits_is_not_lost() {
        let state = SyncState::default();
        let cancel = state.begin_accept().expect("an accept starts");
        state.cancel_accept();
        tokio::time::timeout(Duration::from_millis(50), cancel.cancelled())
            .await
            .expect("the stored cancellation permit was lost");
    }

    #[test]
    fn an_unmarked_clone_collision_is_not_modified() {
        let parent = scratch("clone-collision");
        let project = new_id();
        let short = project.to_string().chars().take(8).collect::<String>().to_lowercase();
        let collision = parent.join(format!("shared-{short}.wobu"));
        std::fs::create_dir(&collision).unwrap();
        let sentinel = collision.join("belongs-to-someone-else.txt");
        std::fs::write(&sentinel, b"untouched").unwrap();

        assert!(create_clone_scaffold(&parent, project).is_err());
        let entries: Vec<_> = std::fs::read_dir(&collision)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![sentinel.file_name().unwrap()]);
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"untouched");

        let _ = std::fs::remove_dir_all(parent);
    }

    #[test]
    fn a_verified_partial_clone_can_resume_without_deleting_downloaded_files() {
        let parent = scratch("resume-clone");
        let project = new_id();
        let first = create_clone_scaffold(&parent, project).expect("initial scaffold");
        let recovered = first.root.join("nodes/recovered.md");
        std::fs::write(&recovered, "recoverable").unwrap();

        let resumed = create_clone_scaffold(&parent, project).expect("verified resume");
        assert_eq!(resumed.root, first.root);
        assert_eq!(std::fs::read_to_string(recovered).unwrap(), "recoverable");

        let _ = std::fs::remove_dir_all(parent);
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
                index_dir: Some(dir.join("index")),
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
        // Real QUIC over loopback still loses connections when the machine is
        // busy, and the product calls that failure retryable precisely because
        // a client's answer to it is to dial again. What this test is about is
        // the round's termination handshake, not the link's durability, so it
        // does the same. Anything the product does not call retryable, and any
        // wrong answer, still fails at once.
        const ATTEMPTS: u32 = 5;
        let mut attempt = 0;
        let (session, exchange, fetched, pushed) = loop {
            attempt += 1;
            match one_round(&peer, &ticket, node_id).await {
                Ok(round) => break round,
                Err(error) if error.retryable && attempt < ATTEMPTS => {}
                Err(error) => panic!("a round completes (attempt {attempt}): {error:?}"),
            }
        };

        assert!(exchange.is_whole());
        // A fresh project is not empty — `Project::create` seeds it — so this
        // asks whether the node reached the manifest rather than counting.
        assert!(
            exchange.nodes.iter().any(|(id, _)| *id == node_id),
            "the app did not announce its node: {:?}",
            exchange.nodes
        );
        let announced = exchange.nodes.len();

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

    /// One dial, manifest swap and body round against the app, driven by hand
    /// as the peer.
    ///
    /// A function only so that `?` can hand a transport failure back to the
    /// caller's retry. Every dial gets a fresh session, because a connection
    /// that was lost is not one to ask a second question on.
    async fn one_round(
        peer: &SyncEndpoint,
        ticket: &Ticket,
        node_id: Id,
    ) -> CommandResult<(
        wobu_sync::Session,
        wobu_sync::manifest::Exchange,
        Vec<wobu_store::Outgoing>,
        Vec<Id>,
    )> {
        let session = peer.connect_ticket(ticket).await.map_err(WobuError::from)?;

        // The peer's half of the manifest exchange: it holds nothing. Under the
        // rule `wobu-sync` states twice, that is "never had it" and not
        // "deleted", so the app's side plans to send rather than to remove.
        let exchange =
            wobu_sync::manifest::exchange(&session, &[], &[], wobu_sync::manifest::IDLE_TIMEOUT)
                .await
                .map_err(WobuError::from)?;

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

        let (fetched, pushed) = tokio::try_join!(asking, answering)?;
        Ok((session, exchange, fetched, pushed))
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
