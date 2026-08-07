use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use wobu_core::new_id;
use wobu_store::Project;
use wobu_sync::{Grant, Projects, SyncEndpoint};

use super::clone::{accept_ticket, create_clone_scaffold};
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
    let entries: Vec<_> =
        std::fs::read_dir(&collision).unwrap().map(|entry| entry.unwrap().file_name()).collect();
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
    let after = tokio::time::timeout(Duration::from_secs(3), dialler.connect_ticket(&ticket)).await;
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

/// #149, stated as a number.
///
/// Accepting a ticket killed the Windows build outright — exception
/// `0xc00000fd`, `STATUS_STACK_OVERFLOW`, faulting in `wobu.exe` — with no
/// panic hook line and no wind-down, because a stack overflow is not a
/// panic and nothing in the process survives to write one.
///
/// The cause was the size of this future. An `async fn` is sized for its
/// largest branch whichever branch runs, so the clone transfer inside
/// `clone_into` — `run_ticket`'s state machine is ~130 KB on its own — was
/// built on the stack even by the destination-less probe that never touches
/// it. Tauri constructs a command's future on the main thread, and Windows
/// gives a main thread 1 MiB against Linux's 8 MiB, which is the whole
/// reason this reproduced on one platform and not the other.
///
/// The budget is far below what would actually overflow. That is the point:
/// this asserts the transfer is *boxed*, not that it currently fits, so
/// inlining it again fails here rather than on somebody's machine.
#[test]
fn the_accept_future_fits_a_windows_stack() {
    const BUDGET: usize = 4 * 1024;

    let sync = SyncState::default();
    // Never polled — constructing it is what the size is about.
    let future = accept_ticket(&sync, None, None, None);
    let size = std::mem::size_of_val(&future);

    assert!(
        size <= BUDGET,
        "the accept future is {size} bytes, over the {BUDGET}-byte budget; \
         something large is inlined into it again — box it, as `clone_into` is"
    );
}
