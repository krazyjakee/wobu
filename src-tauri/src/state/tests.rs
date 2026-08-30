use super::*;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::time::Instant;

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("wobu-state-test-{}", wobu_core::new_id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open_test_state(name: &str) -> (TestDir, PathBuf, AppState) {
    let dir = TestDir::new();
    let project = Project::create(&dir.0, name).unwrap();
    let root = project.root().to_path_buf();
    let state = AppState::default();
    *state.slot.lock() =
        Some(Open { project, watcher: None, presence: Presence::start(&root), offline: false });
    (dir, root, state)
}

#[test]
fn a_delayed_full_observation_does_not_block_index_reads() {
    let (_dir, root, state) = open_test_state("Latency");

    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = state.handle();
    let observed_root = root.clone();
    let thread = std::thread::spawn(move || {
        worker.reconcile_full_with(&observed_root, 0, false, move |plan| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            plan.observe()
        })
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let began = Instant::now();
    let count = state.with(|project| Ok(project.list_nodes()?.len())).unwrap();
    let elapsed = began.elapsed();
    eprintln!("index read while full observation was blocked: {elapsed:?}");
    assert_eq!(count, 2, "new projects contain the two singleton nodes");
    assert!(
        elapsed < Duration::from_millis(100),
        "an index-only read waited {elapsed:?} behind filesystem observation"
    );

    release_tx.send(()).unwrap();
    assert!(matches!(thread.join().unwrap(), Outcome::Reconciled(false)));
}

#[test]
fn an_artificially_blocked_import_or_decode_does_not_delay_index_commands() {
    let (_dir, _root, state) = open_test_state("Unlocked asset work");
    let (ticket, ()) = state.ticket(|_| Ok(())).unwrap();

    // This is the exact shape used by import and lazy thumbnail commands:
    // bounded preparation, arbitrary filesystem/pixel work, bounded index
    // commit. Hold the middle phase open until the assertion has run.
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = state.handle();
    let thread = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        worker.with_ticket(&ticket, |project| Ok(project.list_assets()?.len()))
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let began = Instant::now();
    let count = state.with(|project| Ok(project.list_nodes()?.len())).unwrap();
    let elapsed = began.elapsed();
    assert_eq!(count, 2, "new projects contain the two singleton nodes");
    assert!(
        elapsed < Duration::from_millis(100),
        "node_list waited {elapsed:?} behind unlocked import/decode work"
    );

    release_tx.send(()).unwrap();
    assert_eq!(thread.join().unwrap().unwrap(), 0);
}

#[test]
fn reopening_the_same_folder_invalidates_an_old_project_ticket() {
    let (_dir, root, state) = open_test_state("Same folder");
    let original_id = state.open_id().unwrap();
    let (ticket, ()) = state.ticket(|_| Ok(())).unwrap();

    // Mirror close/install without needing a Tauri AppHandle: the old
    // Project/index handle must be gone before the same folder is opened.
    state.generation.fetch_add(1, Ordering::SeqCst);
    drop(state.slot.lock().take());
    let reopened = Project::open(&root).unwrap();
    assert_eq!(reopened.id(), original_id);
    state.generation.fetch_add(1, Ordering::SeqCst);
    *state.slot.lock() = Some(Open {
        project: reopened,
        watcher: None,
        presence: Presence::start(&root),
        offline: false,
    });

    let error = state.with_ticket(&ticket, |_| Ok(())).unwrap_err();
    assert_eq!(error.code, crate::error::Code::NoProjectOpen);
}

#[test]
fn switching_projects_rejects_the_old_projects_index_commit() {
    let (dir, _root, state) = open_test_state("First world");
    let (ticket, ()) = state.ticket(|_| Ok(())).unwrap();

    state.generation.fetch_add(1, Ordering::SeqCst);
    drop(state.slot.lock().take());
    let next = Project::create(&dir.0, "Second world").unwrap();
    let next_root = next.root().to_path_buf();
    state.generation.fetch_add(1, Ordering::SeqCst);
    *state.slot.lock() = Some(Open {
        project: next,
        watcher: None,
        presence: Presence::start(&next_root),
        offline: false,
    });

    assert!(state.with_ticket(&ticket, |_| Ok(())).is_err());
    assert_eq!(state.open_id(), state.peek(|project| project.map(Project::id)));
}

#[test]
fn going_offline_after_preparation_rejects_the_index_commit() {
    let (_dir, _root, state) = open_test_state("Offline transition");
    let (ticket, ()) = state.ticket(|_| Ok(())).unwrap();
    state.slot.lock().as_mut().unwrap().offline = true;

    let error = state.with_ticket(&ticket, |_| Ok(())).unwrap_err();
    assert_eq!(error.code, crate::error::Code::ShareUnmounted);
}

#[test]
fn overlapping_full_requests_coalesce_into_one_followup_observation() {
    let (_dir, root, state) = open_test_state("Coalescing");
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_calls = Arc::clone(&calls);
    let worker = state.handle();
    let observed_root = root.clone();
    let thread = std::thread::spawn(move || {
        worker.reconcile_full_with(&observed_root, 0, false, move |plan| {
            if worker_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
            plan.observe()
        })
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let waiting = state.handle();
    let waiter = std::thread::spawn(move || waiting.reconcile_now());
    let deadline = Instant::now() + Duration::from_secs(2);
    while !state.reconciling.lock().pending {
        assert!(Instant::now() < deadline, "explicit reconcile never joined the running pass");
        std::thread::yield_now();
    }
    for _ in 0..4 {
        assert!(matches!(
            state.reconcile_full_with(&root, 0, false, ReconcilePlan::observe),
            Outcome::Reconciled(false)
        ));
    }
    release_tx.send(()).unwrap();
    assert!(matches!(thread.join().unwrap(), Outcome::Reconciled(false)));
    assert!(!waiter.join().unwrap().unwrap());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "many overlapping requests should collapse to one followup"
    );
}

/// Records what sync would have been told, so the wiring can be asserted
/// without an `AppHandle` the unit tests have no way to mint.
#[derive(Default)]
struct SpyHandover {
    changed: Mutex<Vec<Id>>,
}

impl Handover for SpyHandover {
    fn opening(&self, _project: Id, _root: &Path) {}
    fn closing(&self, _project: Id) {}
    fn changed_locally(&self, project: Id) {
        self.changed.lock().push(project);
    }
}

fn bump_mtime(path: &Path) {
    let meta = std::fs::metadata(path).unwrap();
    let later = meta.modified().unwrap() + Duration::from_secs(2);
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(later).unwrap();
}

#[test]
fn an_edit_this_machine_made_nudges_sync_rather_than_waiting_for_its_backoff() {
    // The Obsidian case, followed one step further than the store tests take
    // it: the folder is canonical, so an edit made outside the app still has to
    // reach the collaborators. Reconciling it into the index is only half of
    // that — without the nudge the bytes then sit until the outbound poller's
    // backoff happens to come round, which at its far end is two minutes.
    let (_dir, root, state) = open_test_state("Nudge");
    let spy = Arc::new(SpyHandover::default());
    state.observe(Arc::downgrade(&spy) as Weak<dyn Handover>);
    let project = state.open_id().expect("open_test_state installs a project");

    state
        .with(|p| Ok(p.create_node(wobu_core::NodeKind::Species, "Vashk", None)?))
        .expect("a node in an open project");
    // Everything so far went through the index, so there is nothing outstanding
    // and a reload has nothing to announce.
    assert!(!state.reconcile_now().unwrap());
    assert!(spy.changed.lock().is_empty(), "an unchanged reload is not an edit");

    let path = root.join("nodes/species/vashk.md");
    let text = std::fs::read_to_string(&path).unwrap().replace("name: Vashk", "name: Vashk-Prime");
    std::fs::write(&path, text).unwrap();
    bump_mtime(&path);

    assert!(state.reconcile_now().unwrap(), "the external edit was not observed");
    assert_eq!(&*spy.changed.lock(), &[project], "sync was not told to fan the edit out");
}

#[test]
fn a_nudge_with_no_project_open_is_not_an_error() {
    // `announce_local_change` runs on the watcher callback, which can fire
    // against a project the user closed a moment ago.
    let state = AppState::default();
    let spy = Arc::new(SpyHandover::default());
    state.observe(Arc::downgrade(&spy) as Weak<dyn Handover>);

    state.announce_local_change();
    assert!(spy.changed.lock().is_empty());
}

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
