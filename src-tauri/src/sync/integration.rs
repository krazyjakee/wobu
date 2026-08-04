//! #85: whole sync rounds between independent project folders and indexes over
//! real loopback QUIC endpoints.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use wobu_core::{Id, NodeKind, new_id};
use wobu_store::{Project, SaveOutcome};
use wobu_sync::{
    Config, Disposition, Grant, Identity, Projects, Reach, Session, Sessions, SyncEndpoint, Ticket,
};

use super::manager::{Setup, SyncManager, Wake};
use super::shares::Shares;
use crate::state::AppState;

struct Quiet;

impl Wake for Quiet {
    fn world_changed(&self, _project: Id) {}
}

struct Peer {
    manager: Option<Arc<SyncManager>>,
    home: PathBuf,
    root: PathBuf,
    ticket: Ticket,
}

impl Peer {
    async fn start(home: PathBuf, root: PathBuf, project: Id) -> Peer {
        let mut shares = Shares::load_from(home.join("shares.json"));
        let grant = shares.share(project, &root).grant;
        shares.save().unwrap();
        let manager = SyncManager::start(
            AppState::default(),
            Arc::new(Quiet),
            Setup {
                identity: Identity::ephemeral(),
                reach: Reach::Loopback,
                shares,
                poll: false,
                index_dir: Some(home.join("index")),
            },
        )
        .await
        .expect("a test peer binds");
        let ticket = manager.endpoint().ticket(project, grant);
        Peer { manager: Some(manager), home, root, ticket }
    }

    fn manager(&self) -> &Arc<SyncManager> {
        self.manager.as_ref().unwrap()
    }

    fn invite(&self, ticket: &Ticket) {
        assert_eq!(self.manager().accept(ticket), Disposition::Join);
    }

    async fn sync(&self, project: Id) {
        let _ = self.manager().run_once(project).await;
    }

    /// A loopback QUIC connection can still be dropped part-way through a
    /// round, and the error says as much about itself — `retryable: true`. The
    /// product's answer to that is to run the round again, so a helper that
    /// gave up on the first drop would be asserting something stricter than
    /// Wobu promises. A round is idempotent, so going again is safe; what the
    /// test still insists on is that the peers converge.
    async fn sync_with(&self, project: Id, other: &Peer) {
        const ATTEMPTS: u32 = 5;
        for attempt in 1..=ATTEMPTS {
            match self.manager().run_ticket(project, &other.ticket).await {
                Ok(()) => return,
                Err(error) if error.retryable && attempt < ATTEMPTS => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Err(error) => {
                    panic!("a sync round still failed after {attempt} attempts: {error:?}")
                }
            }
        }
    }

    fn edit(&self, project: Id, node: Id, notes: &str) {
        self.manager()
            .replica(project)
            .unwrap()
            .with(|project| {
                let mut node = project.get_node(node)?;
                node.notes_raw = notes.into();
                assert!(matches!(project.save_node(node)?, SaveOutcome::Saved(_)));
                Ok(())
            })
            .unwrap();
    }

    fn notes(&self, project: Id, node: Id) -> String {
        self.manager()
            .replica(project)
            .unwrap()
            .with(|project| Ok(project.get_node(node)?.notes_raw))
            .unwrap()
    }

    fn manifest(&self, project: Id) -> Vec<(Id, String)> {
        self.manager().replica(project).unwrap().with(|project| Ok(project.manifest()?)).unwrap()
    }

    fn drop_sync_table(&self, project: Id) {
        let replica = self.manager().replica(project).unwrap();
        replica.release_for_test();
        let path = self.home.join("index").join(format!("{project}.sqlite"));
        let db = rusqlite::Connection::open(path).unwrap();
        db.execute_batch("DROP TABLE sync_state").unwrap();
    }

    async fn stop(&mut self) {
        if let Some(manager) = self.manager.take() {
            manager.shutdown().await;
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        // Dropping the last manager handle aborts the router even if an assertion
        // panicked before the async shutdown. The private tree is then safe to
        // remove on Unix and Windows alike.
        self.manager.take();
        let _ = fs::remove_dir_all(&self.home);
    }
}

struct Pair {
    home: PathBuf,
    project: Id,
    node: Id,
    global_index: PathBuf,
    a: Peer,
    b: Peer,
}

impl Pair {
    async fn new(empty_b: bool) -> Pair {
        let base = scratch("pair");
        let a_home = base.join("a-machine");
        let b_home = base.join("b-machine");
        fs::create_dir_all(&a_home).unwrap();
        fs::create_dir_all(&b_home).unwrap();

        let mut source = Project::create(&a_home, "Ashfall").unwrap();
        let node = source.create_node(NodeKind::Character, "Kael Vantris", None).unwrap().id;
        let project = source.id();
        let global_index = source.index_path();
        let a_root = source.root().to_path_buf();
        drop(source);

        let b_root = b_home.join(a_root.file_name().unwrap());
        if empty_b {
            empty_clone(&a_root, &b_root);
        } else {
            copy_tree(&a_root, &b_root);
        }

        let a = Peer::start(a_home, a_root, project).await;
        let b = Peer::start(b_home, b_root, project).await;
        a.invite(&b.ticket);
        b.invite(&a.ticket);

        let pair = Pair { home: base, project, node, global_index, a, b };
        if !empty_b {
            pair.a.sync(pair.project).await;
        }
        pair
    }

    async fn stop(mut self) {
        self.a.stop().await;
        self.b.stop().await;
        cleanup_index(&self.global_index);
    }
}

impl Drop for Pair {
    fn drop(&mut self) {
        self.a.manager.take();
        self.b.manager.take();
        cleanup_index(&self.global_index);
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("wobu-sync-{name}-{}", new_id()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn cleanup_index(path: &Path) {
    let _ = fs::remove_file(path);
    let path = path.to_string_lossy();
    let _ = fs::remove_file(format!("{path}-wal"));
    let _ = fs::remove_file(format!("{path}-shm"));
}

fn run_async(future: impl std::future::Future<Output = ()> + Send + 'static) {
    std::thread::Builder::new()
        .name("wobu-sync-integration".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
            runtime.block_on(future);
        })
        .unwrap()
        .join()
        .unwrap();
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn empty_clone(from: &Path, to: &Path) {
    for rel in ["nodes", "assets/originals", "assets/thumbs", "generations", ".wobu/tmp"] {
        fs::create_dir_all(to.join(rel)).unwrap();
    }
    fs::copy(from.join("project.json"), to.join("project.json")).unwrap();
}

fn node_path(root: &Path, id: Id, index: &Path) -> PathBuf {
    let project = Project::open_at_index(root, index).unwrap();
    let rel = project.index().rel_path_of(id).unwrap().unwrap();
    wobu_store::paths::from_rel_string(root, &rel)
}

fn conflicts(path: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.file_name().unwrap().to_string_lossy().contains(".conflict-"))
        .collect();
    paths.sort();
    paths
}

#[test]
fn clean_fast_forwards_cross_the_real_loop_in_both_directions() {
    run_async(async {
        let pair = Pair::new(false).await;
        pair.a.edit(pair.project, pair.node, "written on A");
        pair.a.sync(pair.project).await;
        assert_eq!(pair.b.notes(pair.project, pair.node), "written on A");

        pair.b.edit(pair.project, pair.node, "written on B");
        pair.b.sync(pair.project).await;
        assert_eq!(pair.a.notes(pair.project, pair.node), "written on B");
        pair.stop().await;
    });
}

#[test]
fn concurrent_edits_park_exactly_one_peer_copy_and_leave_each_local_file_untouched() {
    run_async(async {
        let pair = Pair::new(false).await;
        pair.a.edit(pair.project, pair.node, "A keeps this");
        pair.b.edit(pair.project, pair.node, "B keeps this");
        let a_path = node_path(&pair.a.root, pair.node, &pair.a.home.join("inspect.sqlite"));
        let b_path = node_path(&pair.b.root, pair.node, &pair.b.home.join("inspect.sqlite"));
        let a_before = fs::read(&a_path).unwrap();
        let b_before = fs::read(&b_path).unwrap();

        pair.a.sync(pair.project).await;

        assert_eq!(fs::read(&a_path).unwrap(), a_before);
        assert_eq!(fs::read(&b_path).unwrap(), b_before);
        assert_eq!(conflicts(&a_path).len(), 1);
        assert_eq!(conflicts(&b_path).len(), 1);
        pair.stop().await;
    });
}

#[test]
fn independently_arriving_at_identical_bytes_converges_without_a_conflict() {
    run_async(async {
        let pair = Pair::new(false).await;
        pair.a.edit(pair.project, pair.node, "the same conclusion");
        let a_path = node_path(&pair.a.root, pair.node, &pair.a.home.join("inspect.sqlite"));
        let b_path = node_path(&pair.b.root, pair.node, &pair.b.home.join("inspect.sqlite"));
        fs::copy(&a_path, &b_path).unwrap();

        pair.a.sync(pair.project).await;

        assert!(conflicts(&a_path).is_empty());
        assert!(conflicts(&b_path).is_empty());
        assert_eq!(pair.a.manifest(pair.project), pair.b.manifest(pair.project));
        pair.stop().await;
    });
}

#[test]
fn losing_sync_state_forces_a_full_compare_without_inventing_a_conflict() {
    run_async(async {
        let pair = Pair::new(false).await;
        pair.b.drop_sync_table(pair.project);

        // Unchanged bytes rebuild agreement first. A missing table must cost a full
        // comparison, not turn identical local files into concurrent edits.
        pair.a.sync(pair.project).await;
        let b_path = node_path(&pair.b.root, pair.node, &pair.b.home.join("inspect.sqlite"));
        assert!(conflicts(&b_path).is_empty());

        pair.a.edit(pair.project, pair.node, "after the local sync table vanished");

        pair.a.sync(pair.project).await;

        assert_eq!(pair.b.notes(pair.project, pair.node), "after the local sync table vanished");
        assert!(conflicts(&b_path).is_empty());
        pair.stop().await;
    });
}

#[test]
fn an_obsidian_edit_between_rounds_is_reconciled_and_sent_as_a_local_change() {
    run_async(async {
        let pair = Pair::new(false).await;
        let a_path = node_path(&pair.a.root, pair.node, &pair.a.home.join("inspect.sqlite"));
        let raw = fs::read_to_string(&a_path).unwrap();
        let (frontmatter, _) = raw.split_once("\n---\n").unwrap();
        fs::write(
            &a_path,
            format!("{frontmatter}\n---\n\n## Notes\n\nchanged outside Wobu and made longer\n"),
        )
        .unwrap();

        pair.a.sync(pair.project).await;

        assert!(pair.b.notes(pair.project, pair.node).contains("changed outside Wobu"));
        pair.stop().await;
    });
}

#[test]
fn a_fresh_clone_opens_cleanly_and_matches_the_source_after_one_round() {
    run_async(async {
        let pair = Pair::new(true).await;
        pair.a.sync(pair.project).await;
        assert_eq!(pair.a.manifest(pair.project), pair.b.manifest(pair.project));
        assert_eq!(pair.a.notes(pair.project, pair.node), pair.b.notes(pair.project, pair.node));
        pair.stop().await;
    });
}

#[test]
fn three_peers_close_the_triangle_without_any_node_remaining_divergent() {
    run_async(three_peer_triangle());
}

async fn three_peer_triangle() {
    let base = Pair::new(false).await;
    let c_home = scratch("c-machine");
    let c_root = c_home.join(base.a.root.file_name().unwrap());
    copy_tree(&base.a.root, &c_root);
    let c = Peer::start(c_home, c_root, base.project).await;
    c.invite(&base.a.ticket);
    c.invite(&base.b.ticket);
    base.a.invite(&c.ticket);
    base.b.invite(&c.ticket);

    let a_new = base
        .a
        .manager()
        .replica(base.project)
        .unwrap()
        .with(|p| Ok(p.create_node(NodeKind::Setting, "Only A", None)?.id))
        .unwrap();
    let b_new = base
        .b
        .manager()
        .replica(base.project)
        .unwrap()
        .with(|p| Ok(p.create_node(NodeKind::Setting, "Only B", None)?.id))
        .unwrap();
    let c_new = c
        .manager()
        .replica(base.project)
        .unwrap()
        .with(|p| Ok(p.create_node(NodeKind::Setting, "Only C", None)?.id))
        .unwrap();

    base.a.sync_with(base.project, &base.b).await;
    base.b.sync_with(base.project, &c).await;
    base.a.sync_with(base.project, &c).await;

    let a = base.a.manifest(base.project);
    let b = base.b.manifest(base.project);
    let cm = c.manifest(base.project);
    assert_eq!(a, b);
    assert_eq!(b, cm);
    for id in [a_new, b_new, c_new] {
        assert!(a.iter().any(|(held, _)| *held == id));
    }

    let mut c = c;
    c.stop().await;
    base.stop().await;
}

struct Holds(Id);

impl Projects for Holds {
    fn admits(&self, project: &Id, _grant: Option<&Grant>) -> bool {
        *project == self.0
    }
}

struct SessionsInto(mpsc::UnboundedSender<Session>);

#[async_trait]
impl Sessions for SessionsInto {
    async fn opened(&self, session: Session) {
        let _ = self.0.send(session);
    }
}

#[test]
fn a_disconnect_halfway_through_a_body_lands_no_partial_file_or_index_row() {
    run_async(async {
        let pair = Pair::new(false).await;
        pair.a.edit(pair.project, pair.node, &"a body cut in flight ".repeat(4096));
        let outgoing = pair
            .a
            .manager()
            .replica(pair.project)
            .unwrap()
            .with(|project| Ok(project.outgoing(pair.node)?.unwrap()))
            .unwrap();
        let target_path = node_path(&pair.b.root, pair.node, &pair.b.home.join("inspect.sqlite"));
        let bytes_before = fs::read(&target_path).unwrap();
        let index_before = pair.b.manifest(pair.project);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let server = SyncEndpoint::bind(
            Config::loopback(),
            Arc::new(Holds(pair.project)),
            Arc::new(SessionsInto(tx)),
        )
        .await
        .unwrap();
        let client = SyncEndpoint::bind(
            Config::loopback(),
            Arc::new(Holds(pair.project)),
            Arc::new(SessionsInto(mpsc::unbounded_channel().0)),
        )
        .await
        .unwrap();
        let outbound = client.connect(server.addr(), pair.project).await.unwrap();
        let inbound = rx.recv().await.unwrap();

        let replica = pair.b.manager().replica(pair.project).unwrap();
        let peer = inbound.peer().to_string();
        let receiving =
            super::round::answer_until_cut(pair.b.manager(), &replica, &peer, inbound.connection());
        let (received, cut) =
            tokio::join!(receiving, super::bodies::cut_push(outbound.connection(), &outgoing),);
        assert!(cut.is_ok());
        assert!(received.is_err());

        assert_eq!(fs::read(&target_path).unwrap(), bytes_before);
        assert_eq!(pair.b.manifest(pair.project), index_before);
        pair.b
            .manager()
            .replica(pair.project)
            .unwrap()
            .with(|project| {
                project.reconcile()?;
                assert_eq!(project.manifest()?, index_before);
                Ok(())
            })
            .unwrap();

        client.shutdown().await.unwrap();
        server.shutdown().await.unwrap();
        pair.stop().await;
    });
}
