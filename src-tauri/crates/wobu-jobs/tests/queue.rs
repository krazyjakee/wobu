//! The queue, driven from outside, with a fake task and a fake notifier.
//!
//! It lives out here rather than behind `#[cfg(test)]` for the same reason
//! `wobu-llm`'s fake provider does: an in-crate fake can reach for private
//! items, so it would prove the machinery works *here* rather than that it
//! works at all. Everything below uses only what `wobu-jobs` exports, which is
//! the position Enhance (#37) and the image backends (#40) will be in.
//!
//! Nothing here starts a Tauri app, and that is the point being made as much as
//! it is a convenience.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use wobu_jobs::{
    Billed, Config, Event, Failure, JobContext, JobId, JobKind, JobState, Notify, Outcome, Preview,
    Progress, Queue, QueueSnapshot, RetryPolicy, Task,
};

/* ── the fake notifier ────────────────────────────────────────────────────── */

/// Everything the queue said, in order, plus a bell so a test can wait for
/// something to happen without spinning — a spin would keep the runtime busy
/// and stop paused time from ever advancing.
#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<Event>>,
    bell: tokio::sync::Notify,
}

impl Notify for Recorder {
    fn notify(&self, event: Event) {
        self.events.lock().expect("recorder").push(event);
        // `notify_one` rather than `notify_waiters`: it leaves a permit behind
        // when nobody is listening yet, so an event that lands between a test's
        // check and its await is not a lost wakeup.
        self.bell.notify_one();
    }
}

impl Recorder {
    fn events(&self) -> Vec<Event> {
        self.events.lock().expect("recorder").clone()
    }

    /// The most recent whole-queue snapshot.
    fn last_snapshot(&self) -> QueueSnapshot {
        self.events()
            .into_iter()
            .rev()
            .find_map(|event| match event {
                Event::State(snapshot) => Some(snapshot),
                _ => None,
            })
            .expect("the queue publishes a snapshot on every transition")
    }

    /// Park until `ready` says so. Re-checked on every event the queue emits.
    async fn until(&self, ready: impl Fn() -> bool) {
        loop {
            if ready() {
                return;
            }
            self.bell.notified().await;
        }
    }
}

/* ── the fake task ────────────────────────────────────────────────────────── */

/// What one attempt does. A task is a script of these, one per attempt, and the
/// last one repeats — so `[Fail(…), Finish]` is "fails once, then works".
#[derive(Clone, Debug)]
enum Step {
    Finish,
    FinishWith(serde_json::Value),
    Fail(Failure),
    /// Take a while, honouring nothing in particular. Used for occupancy.
    Work(Duration),
    /// Notice the cancellation token and stop of its own accord, which is what
    /// a well-behaved adapter does.
    AwaitCancel,
    /// Ignore the token completely and sit there. The only way this job ever
    /// ends is the queue killing it.
    Ignore,
    Panic,
}

/// Shared observation of what the tasks actually did, which is the half a
/// snapshot cannot tell you: whether the work ran at all.
#[derive(Default)]
struct Log {
    started: AtomicUsize,
    /// Attempts that returned an outcome under their own power.
    returned: AtomicUsize,
    /// Attempts whose future was dropped without returning — i.e. aborted.
    dropped: AtomicUsize,
    live: AtomicUsize,
    peak: AtomicUsize,
    /// Labels in the order their first attempt started.
    order: Mutex<Vec<String>>,
}

impl Log {
    fn started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }

    fn returned(&self) -> usize {
        self.returned.load(Ordering::SeqCst)
    }

    fn dropped(&self) -> usize {
        self.dropped.load(Ordering::SeqCst)
    }

    fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    fn order(&self) -> Vec<String> {
        self.order.lock().expect("log").clone()
    }

    fn enter(&self, label: &str) {
        self.started.fetch_add(1, Ordering::SeqCst);
        self.order.lock().expect("log").push(label.to_owned());
        let live = self.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(live, Ordering::SeqCst);
    }
}

/// Decrements the occupancy on the way out however the attempt ends, and counts
/// which way that was.
///
/// This is what makes "the queue aborted it" observable: a future that is
/// dropped mid-await runs this and never reaches its own `return`, so `dropped`
/// rising while `returned` does not is proof the work was killed rather than
/// merely ignored.
struct Exit {
    log: Arc<Log>,
    returned: bool,
}

impl Drop for Exit {
    fn drop(&mut self) {
        self.log.live.fetch_sub(1, Ordering::SeqCst);
        if self.returned {
            self.log.returned.fetch_add(1, Ordering::SeqCst);
        } else {
            self.log.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
}

struct Fake {
    kind: JobKind,
    label: String,
    steps: VecDeque<Step>,
    log: Arc<Log>,
}

impl Fake {
    fn new(label: &str, steps: impl IntoIterator<Item = Step>) -> Fake {
        Fake {
            kind: JobKind::Enhance,
            label: label.to_owned(),
            steps: steps.into_iter().collect(),
            log: Arc::new(Log::default()),
        }
    }

    fn logging_to(mut self, log: &Arc<Log>) -> Fake {
        self.log = Arc::clone(log);
        self
    }

    fn log(&self) -> Arc<Log> {
        Arc::clone(&self.log)
    }
}

#[async_trait::async_trait]
impl Task for Fake {
    fn kind(&self) -> JobKind {
        self.kind
    }

    fn label(&self) -> String {
        self.label.clone()
    }

    async fn run(&mut self, ctx: &JobContext) -> Outcome {
        self.log.enter(&self.label);
        let mut exit = Exit { log: Arc::clone(&self.log), returned: false };
        // Emitted from inside the attempt so that the recorder's event list
        // interleaves "the queue said it would retry" with "the task started
        // again" — which is how the ordering of those two is asserted.
        ctx.progress(Progress::new(ctx.attempt(), 0).with_note("attempt"));

        let step = if self.steps.len() > 1 {
            self.steps.pop_front().expect("checked")
        } else {
            self.steps.front().cloned().unwrap_or(Step::Finish)
        };

        let outcome = match step {
            Step::Finish => Outcome::done(),
            Step::FinishWith(value) => {
                ctx.preview(Preview::new("data:image/png;base64,AAA").at_step(1));
                Outcome::Done(Some(value))
            }
            Step::Fail(failure) => Outcome::failed(failure),
            Step::Work(duration) => {
                tokio::time::sleep(duration).await;
                Outcome::done()
            }
            Step::AwaitCancel => {
                ctx.cancel().cancelled().await;
                Outcome::Cancelled
            }
            Step::Ignore => {
                // A whole hour, and no look at the token. Nothing but the queue
                // killing this future ends it.
                tokio::time::sleep(Duration::from_secs(3600)).await;
                Outcome::done()
            }
            Step::Panic => panic!("the fake was told to panic"),
        };
        exit.returned = true;
        outcome
    }
}

/* ── helpers ──────────────────────────────────────────────────────────────── */

fn queue_with(config: Config) -> (Queue, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    (Queue::new(config, Arc::clone(&recorder)), recorder)
}

fn state_of(queue: &Queue, id: JobId) -> Option<JobState> {
    queue.snapshot().jobs.into_iter().find(|job| job.id == id).map(|job| job.state)
}

fn attempt_of(queue: &Queue, id: JobId) -> u32 {
    queue.snapshot().jobs.into_iter().find(|job| job.id == id).map(|job| job.attempt).unwrap_or(0)
}

/// Wait for one job to reach a terminal state and say which.
async fn settle(queue: &Queue, recorder: &Recorder, id: JobId) -> JobState {
    recorder.until(|| state_of(queue, id).is_some_and(|state| state.is_terminal())).await;
    state_of(queue, id).expect("still there")
}

/// Wait for the whole queue to empty.
async fn settle_all(queue: &Queue, recorder: &Recorder) {
    recorder.until(|| queue.snapshot().depth() == 0).await;
}

/// A failure that costs nothing and is worth another go — a 429, a refused
/// connection.
fn transient() -> Failure {
    Failure::new("provider.rate_limited", "The provider is rate limiting this key.")
        .retryable(true)
        .billed(Billed::Nothing)
}

/// A failure the user has already been charged for — the model answered, and
/// the answer is unusable.
fn paid_garbage() -> Failure {
    Failure::new("provider.bad_response", "The response stopped before the description was.")
        .retryable(true)
        .billed(Billed::Charged)
        .cost_note("812 in + 400 out")
}

fn retries(events: &[Event]) -> Vec<&wobu_jobs::RetryEvent> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Retry(retry) => Some(retry),
            _ => None,
        })
        .collect()
}

/* ── the basics ───────────────────────────────────────────────────────────── */

#[tokio::test]
async fn a_job_reports_its_id_before_it_has_started_and_its_result_when_it_ends() {
    // The contract the whole app is written against: submit returns now, the
    // answer arrives later. If `submit` ever awaited the work, every command
    // that used it would block the webview.
    let (queue, recorder) = queue_with(Config::default());
    let result = json!({ "nodeId": "01J" });
    let id = queue.submit(Fake::new("Enhance Vashk", [Step::FinishWith(result.clone())]));

    assert_eq!(state_of(&queue, id), Some(JobState::Queued), "queued before it runs");
    assert_eq!(settle(&queue, &recorder, id).await, JobState::Done);

    let done = recorder
        .events()
        .into_iter()
        .find_map(|event| match event {
            Event::Done(done) => Some(done),
            _ => None,
        })
        .expect("a finished job says so");
    assert_eq!(done.id, id);
    assert_eq!(done.label, "Enhance Vashk");
    assert_eq!(done.result, Some(result));
    // And the preview the task emitted went out as its own event rather than
    // being buffered until the end, which would make it useless.
    assert!(recorder.events().iter().any(|event| matches!(event, Event::Preview(_))));
}

#[tokio::test]
async fn the_snapshot_is_what_the_status_bar_needs_and_it_arrives_on_every_transition() {
    // Depth is the number the status bar shows. A snapshot that only counted
    // what was running would read "1 job" with nine behind it.
    let (queue, recorder) = queue_with(Config { concurrency: 1, ..Config::default() });
    let log = Arc::new(Log::default());
    let ids: Vec<JobId> = (0..3)
        .map(|n| {
            queue.submit(
                Fake::new(&format!("job {n}"), [Step::Work(Duration::from_millis(20))])
                    .logging_to(&log),
            )
        })
        .collect();

    recorder.until(|| queue.snapshot().running == 1).await;
    let snapshot = queue.snapshot();
    assert_eq!(snapshot.depth(), 3);
    assert_eq!(snapshot.queued, 2);
    assert_eq!(snapshot.jobs.iter().map(|job| job.id).collect::<Vec<_>>(), ids);

    settle_all(&queue, &recorder).await;
    assert_eq!(recorder.last_snapshot().depth(), 0);
    assert_eq!(recorder.last_snapshot().jobs.len(), 3, "finished jobs stay visible");
}

#[tokio::test]
async fn finished_jobs_are_forgotten_once_there_are_more_than_the_history_keeps() {
    // Nothing else clears this. A session that enhances two hundred nodes would
    // otherwise carry every one of them in every snapshot for the rest of the
    // day.
    let (queue, recorder) = queue_with(Config { history: 2, ..Config::default() });
    for n in 0..5 {
        queue.submit(Fake::new(&format!("job {n}"), [Step::Finish]));
    }
    settle_all(&queue, &recorder).await;

    let snapshot = queue.snapshot();
    assert_eq!(snapshot.jobs.len(), 2);
    assert_eq!(
        snapshot.jobs.iter().map(|job| job.label.clone()).collect::<Vec<_>>(),
        vec!["job 3".to_owned(), "job 4".to_owned()],
        "the newest are the ones worth keeping",
    );
}

/* ── concurrency, ordering, fairness ──────────────────────────────────────── */

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_concurrency_cap_is_honoured_on_a_real_thread_pool() {
    // Hunyuan3D allows exactly three concurrent Pro jobs and rate-limits a
    // fourth, so exceeding this does not mean "a bit faster", it means a
    // self-inflicted 429 that costs a round trip and looks like their fault.
    // Multi-threaded on purpose: a cap that only holds on one thread is not a
    // cap, and the app's runtime has several.
    let (queue, recorder) = queue_with(Config { concurrency: 3, ..Config::default() });
    let log = Arc::new(Log::default());
    for n in 0..9 {
        queue.submit(
            Fake::new(&format!("job {n}"), [Step::Work(Duration::from_millis(20))])
                .logging_to(&log),
        );
    }
    settle_all(&queue, &recorder).await;

    assert_eq!(log.started(), 9);
    assert_eq!(log.peak(), 3, "more than three ran at once");
}

#[tokio::test(start_paused = true)]
async fn jobs_start_in_the_order_they_were_submitted() {
    // FIFO is not decoration: the user submitted these in an order they had a
    // reason for, and a queue that reordered them would show its working in the
    // status bar.
    let (queue, recorder) = queue_with(Config { concurrency: 1, ..Config::default() });
    let log = Arc::new(Log::default());
    for n in 0..5 {
        queue.submit(
            Fake::new(&format!("job {n}"), [Step::Work(Duration::from_millis(5))]).logging_to(&log),
        );
    }
    settle_all(&queue, &recorder).await;

    assert_eq!(log.order(), vec!["job 0", "job 1", "job 2", "job 3", "job 4"]);
}

#[tokio::test(start_paused = true)]
async fn a_job_that_keeps_failing_goes_to_the_back_rather_than_holding_the_front() {
    // The fairness guarantee, and the reason a backoff releases its slot: a job
    // retrying against a rate limit must not starve the job submitted after it.
    // The alternative is a queue where one unlucky job blocks everything for as
    // long as the provider stays unhappy.
    let (queue, recorder) = queue_with(Config { concurrency: 1, ..Config::default() });
    let log = Arc::new(Log::default());
    let unlucky = queue
        .submit(Fake::new("unlucky", [Step::Fail(transient()), Step::Finish]).logging_to(&log));
    let ordinary = queue.submit(Fake::new("ordinary", [Step::Finish]).logging_to(&log));

    settle(&queue, &recorder, ordinary).await;
    assert_eq!(settle(&queue, &recorder, unlucky).await, JobState::Done);
    assert_eq!(
        log.order(),
        vec!["unlucky", "ordinary", "unlucky"],
        "the retry jumped back in front of a job that was already waiting",
    );
}

#[tokio::test(start_paused = true)]
async fn lowering_the_cap_while_jobs_are_running_takes_effect_as_they_finish() {
    // Permits in use cannot be taken back, so a naive `forget_permits` on a busy
    // queue silently does nothing and the setting appears not to work. Nothing
    // running is interrupted either — killing paid work in flight to satisfy a
    // settings change obeys the instruction more literally than anyone meant it.
    let (queue, recorder) = queue_with(Config { concurrency: 3, ..Config::default() });
    let busy = Arc::new(Log::default());
    for n in 0..3 {
        queue.submit(
            Fake::new(&format!("busy {n}"), [Step::Work(Duration::from_millis(50))])
                .logging_to(&busy),
        );
    }
    recorder.until(|| queue.snapshot().running == 3).await;

    queue.set_concurrency(1);
    assert_eq!(queue.concurrency(), 1);
    assert_eq!(busy.peak(), 3, "the three already running were left alone");

    let after = Arc::new(Log::default());
    for n in 0..3 {
        queue.submit(
            Fake::new(&format!("after {n}"), [Step::Work(Duration::from_millis(5))])
                .logging_to(&after),
        );
    }
    settle_all(&queue, &recorder).await;
    assert_eq!(after.started(), 3);
    assert_eq!(after.peak(), 1, "the lowered cap never took hold");

    // And raising it again settles the debt rather than adding on top of it,
    // which would let the cap drift upwards every time it was toggled.
    queue.set_concurrency(3);
    let raised = Arc::new(Log::default());
    for n in 0..6 {
        queue.submit(
            Fake::new(&format!("raised {n}"), [Step::Work(Duration::from_millis(5))])
                .logging_to(&raised),
        );
    }
    settle_all(&queue, &recorder).await;
    assert_eq!(raised.peak(), 3);
}

/* ── cancellation ─────────────────────────────────────────────────────────── */

#[tokio::test(start_paused = true)]
async fn cancelling_a_job_that_has_not_started_means_it_never_runs() {
    // The cheapest cancel there is and the easiest to get wrong. A queue that
    // only stopped *listening* would still start this job the moment a slot
    // opened, and the user would be billed for work they cancelled before it
    // existed.
    let (queue, recorder) = queue_with(Config { concurrency: 1, ..Config::default() });
    let blocker = queue.submit(Fake::new("blocker", [Step::AwaitCancel]));
    let waiting = Fake::new("waiting", [Step::Finish]);
    let log = waiting.log();
    let id = queue.submit(waiting);

    recorder.until(|| state_of(&queue, blocker) == Some(JobState::Running)).await;
    assert!(queue.cancel(id));
    assert_eq!(settle(&queue, &recorder, id).await, JobState::Cancelled);

    // Now let the blocker go, so a slot opens. The cancelled job must not take
    // it.
    queue.cancel(blocker);
    settle_all(&queue, &recorder).await;
    assert_eq!(log.started(), 0, "a cancelled job ran anyway once a slot opened");
}

#[tokio::test(start_paused = true)]
async fn a_task_that_honours_its_token_stops_itself_and_gets_to_say_so() {
    // The graceful path, and the reason for the grace at all: an adapter that
    // notices the token still has to unwind far enough to report what the
    // provider charged, and #55's spend ceiling only counts what it is told.
    let (queue, recorder) = queue_with(Config::default());
    let task = Fake::new("polite", [Step::AwaitCancel]);
    let log = task.log();
    let id = queue.submit(task);

    recorder.until(|| state_of(&queue, id) == Some(JobState::Running)).await;
    assert!(queue.cancel(id));
    assert_eq!(settle(&queue, &recorder, id).await, JobState::Cancelled);
    assert_eq!(log.returned(), 1, "it returned under its own power");
    assert_eq!(log.dropped(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_task_that_ignores_its_token_is_aborted_rather_than_left_running() {
    // The whole point of the issue. A cancel that only unsubscribes from the
    // result leaves the provider generating tokens the user pays for, so after
    // the grace the future is dropped outright — which is what closes the
    // socket. `dropped` rising while `returned` does not is the proof: that
    // future never reached its own return.
    let (queue, recorder) =
        queue_with(Config { cancel_grace: Duration::from_millis(50), ..Config::default() });
    let task = Fake::new("stubborn", [Step::Ignore]);
    let log = task.log();
    let id = queue.submit(task);

    recorder.until(|| state_of(&queue, id) == Some(JobState::Running)).await;
    queue.cancel(id);
    assert_eq!(settle(&queue, &recorder, id).await, JobState::Cancelled);
    assert_eq!(log.dropped(), 1, "the work was left running");
    assert_eq!(log.returned(), 0);

    // And the slot came back with it, rather than being held by a job nobody
    // can see any more.
    let next = queue.submit(Fake::new("after", [Step::Finish]));
    assert_eq!(settle(&queue, &recorder, next).await, JobState::Done);
}

#[tokio::test(start_paused = true)]
async fn cancelling_during_a_backoff_stops_the_job_without_waiting_it_out() {
    // A job visibly counting down to another attempt is exactly when someone
    // reaches for Stop, and making them wait thirty seconds for the button to
    // work would be an odd way to honour it.
    let (queue, recorder) = queue_with(Config {
        retry: RetryPolicy { base_delay: Duration::from_secs(30), ..RetryPolicy::default() },
        ..Config::default()
    });
    let task = Fake::new("backing off", [Step::Fail(transient()), Step::Finish]);
    let log = task.log();
    let id = queue.submit(task);

    recorder.until(|| matches!(state_of(&queue, id), Some(JobState::Retrying { .. }))).await;
    assert!(queue.cancel(id));
    assert_eq!(settle(&queue, &recorder, id).await, JobState::Cancelled);
    assert_eq!(log.started(), 1, "the retry happened after the cancellation");
}

#[tokio::test]
async fn cancelling_something_that_is_not_there_is_not_an_error() {
    // The user can press Stop at the instant a job finishes, and a webview can
    // hold an id from before a reload. Neither is worth an error dialog.
    let (queue, recorder) = queue_with(Config::default());
    assert!(!queue.cancel(JobId::new()), "cancelled a job that never existed");

    let id = queue.submit(Fake::new("quick", [Step::Finish]));
    settle(&queue, &recorder, id).await;
    assert!(!queue.cancel(id), "cancelled a job that had already finished");
}

/* ── panics ───────────────────────────────────────────────────────────────── */

#[tokio::test(start_paused = true)]
async fn a_panicking_job_fails_alone_and_gives_its_slot_back() {
    // A panic that took the queue with it would strand every other job in
    // `running` for the life of the process, with no way to cancel them and no
    // sign of what happened. It is reported as `internal` because it is our bug,
    // and never retried because the second attempt reaches the same bug with the
    // user's money.
    let (queue, recorder) = queue_with(Config { concurrency: 2, ..Config::default() });
    let doomed = queue.submit(Fake::new("doomed", [Step::Panic]));
    let bystander = queue.submit(Fake::new("bystander", [Step::Work(Duration::from_millis(5))]));

    let state = settle(&queue, &recorder, doomed).await;
    match state {
        JobState::Failed { failure, retry_held } => {
            assert_eq!(failure.code, "internal");
            assert!(!failure.retryable);
            assert!(!retry_held, "a panic is not an offer to try again");
            assert_eq!(failure.billed, Billed::Unknown);
        }
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(settle(&queue, &recorder, bystander).await, JobState::Done);
    assert_eq!(attempt_of(&queue, doomed), 1, "a panic was retried");

    // The queue is still a queue afterwards.
    let later = queue.submit(Fake::new("later", [Step::Finish]));
    assert_eq!(settle(&queue, &recorder, later).await, JobState::Done);
}

/* ── retries, and what they cost ──────────────────────────────────────────── */

#[tokio::test(start_paused = true)]
async fn a_failure_that_cost_nothing_is_retried_without_being_asked() {
    // A 429 or a dropped connection before anything was generated is a blip.
    // Turning it into a dialog would be the app asking permission to wait.
    let (queue, recorder) = queue_with(Config::default());
    let task = Fake::new("blipped", [Step::Fail(transient()), Step::Finish]);
    let log = task.log();
    let id = queue.submit(task);

    assert_eq!(settle(&queue, &recorder, id).await, JobState::Done);
    assert_eq!(log.started(), 2);
    let announced = retries(&recorder.events()).len();
    assert_eq!(announced, 1, "the retry happened without being announced");
    assert!(!retries(&recorder.events())[0].costs_money);
}

#[tokio::test(start_paused = true)]
async fn a_failure_that_was_paid_for_is_never_retried_on_the_queues_own_initiative() {
    // The sentence this whole crate is built around. "The model produced
    // garbage" is retryable — it could work — but the tokens were billed, so
    // trying again is a decision to spend the user's money and the queue does
    // not get to make it. The job ends held rather than dead, so the UI can
    // offer it.
    let (queue, recorder) = queue_with(Config::default());
    let task = Fake::new("garbage in", [Step::Fail(paid_garbage()), Step::Finish]);
    let log = task.log();
    let id = queue.submit(task);

    match settle(&queue, &recorder, id).await {
        JobState::Failed { failure, retry_held } => {
            assert!(retry_held, "a paid failure was reported as a dead end");
            assert!(failure.retryable);
            assert_eq!(failure.billed, Billed::Charged);
            assert_eq!(failure.cost_note.as_deref(), Some("812 in + 400 out"));
        }
        other => panic!("expected a held failure, got {other:?}"),
    }
    assert_eq!(log.started(), 1, "the queue spent the user's money on a retry");
    assert!(retries(&recorder.events()).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_paid_retry_is_announced_before_it_happens_rather_than_after() {
    // "Without saying so" is the operative half, and the ordering is what makes
    // it true: after the wait this event would be a receipt for money already
    // spent. Here it is the notice, and the window in which Stop still works.
    let (queue, recorder) = queue_with(Config::default());
    let task = Fake::new("worth one more go", [Step::Fail(paid_garbage()), Step::Finish]);
    let log = task.log();
    let id = queue.submit_with(task, RetryPolicy::default().with_paid_attempts(1));

    assert_eq!(settle(&queue, &recorder, id).await, JobState::Done);
    assert_eq!(log.started(), 2);

    let events = recorder.events();
    let announced = events
        .iter()
        .position(|event| matches!(event, Event::Retry(retry) if retry.costs_money))
        .expect("a paid retry says so");
    // The fake emits a progress event as its first act of every attempt, so the
    // second one marks the moment the money was spent.
    let second_attempt = events
        .iter()
        .enumerate()
        .filter(|(_, event)| matches!(event, Event::Progress(_)))
        .map(|(index, _)| index)
        .nth(1)
        .expect("two attempts were made");
    assert!(announced < second_attempt, "the queue spent first and said so afterwards");

    let retry = retries(&events)[0];
    assert_eq!(retry.attempt, 1);
    assert_eq!(retry.max_attempts, RetryPolicy::default().max_attempts);
    assert_eq!(retry.failure.cost_note.as_deref(), Some("812 in + 400 out"));
}

#[tokio::test(start_paused = true)]
async fn the_paid_allowance_is_spent_rather_than_renewed_every_attempt() {
    // Otherwise `paid_attempts: 1` would mean "one paid retry per failure",
    // which is unbounded spending with a number next to it.
    let (queue, recorder) = queue_with(Config::default());
    let task = Fake::new("never right", [Step::Fail(paid_garbage())]);
    let log = task.log();
    let id = queue.submit_with(task, RetryPolicy::default().with_paid_attempts(1));

    match settle(&queue, &recorder, id).await {
        JobState::Failed { retry_held, .. } => assert!(retry_held),
        other => panic!("expected a held failure, got {other:?}"),
    }
    assert_eq!(log.started(), 2, "the allowance was renewed rather than spent");
}

#[tokio::test(start_paused = true)]
async fn a_failure_that_is_not_retryable_ends_the_job_immediately() {
    // A missing key does not get better by being asked again, and four attempts
    // at it would be four times the log noise for one thing the user has to fix
    // in Settings.
    let (queue, recorder) = queue_with(Config::default());
    let refused = Failure::new("provider.no_key", "No API key for Anthropic on this machine.");
    let task = Fake::new("no key", [Step::Fail(refused)]);
    let log = task.log();
    let id = queue.submit(task);

    match settle(&queue, &recorder, id).await {
        JobState::Failed { failure, retry_held } => {
            assert_eq!(failure.code, "provider.no_key");
            assert!(!retry_held, "an unretryable failure is not an offer");
        }
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(log.started(), 1);
}

#[tokio::test(start_paused = true)]
async fn free_retries_stop_at_the_attempt_ceiling() {
    // A provider that is genuinely down must be reported as down. Retrying
    // forever is indistinguishable, from the outside, from the app having hung.
    let (queue, recorder) = queue_with(Config {
        retry: RetryPolicy { max_attempts: 3, ..RetryPolicy::default() },
        ..Config::default()
    });
    let task = Fake::new("down", [Step::Fail(transient())]);
    let log = task.log();
    let id = queue.submit(task);

    match settle(&queue, &recorder, id).await {
        JobState::Failed { retry_held, .. } => {
            assert!(!retry_held, "an exhausted job is not waiting on a decision");
        }
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(log.started(), 3);
    assert_eq!(attempt_of(&queue, id), 3);
}

#[tokio::test(start_paused = true)]
async fn a_job_submitted_with_no_retries_at_all_is_attempted_once() {
    // For work whose second run would duplicate a side effect. The policy is
    // per submission because only the submitter knows that.
    let (queue, recorder) = queue_with(Config::default());
    let task = Fake::new("once only", [Step::Fail(transient())]);
    let log = task.log();
    let id = queue.submit_with(task, RetryPolicy::never());

    assert!(matches!(settle(&queue, &recorder, id).await, JobState::Failed { .. }));
    assert_eq!(log.started(), 1);
}

/* ── shutdown (#143) ──────────────────────────────────────────────────────── */

#[tokio::test(flavor = "multi_thread")]
async fn closing_the_queue_stops_everything_unfinished_and_then_quiesces() {
    // The exit path, whole: three jobs the user can see — one running, one
    // waiting behind the cap, one that has already finished — and the promise
    // that after `close` + `quiesce` there is nothing left in flight. The
    // running one is `AwaitCancel` because that is what a real adapter does,
    // and its `Cancelled` state is the evidence the token reached it rather
    // than the process merely ending underneath it.
    let (queue, recorder) = queue_with(Config { concurrency: 1, ..Config::default() });

    let finished = queue.submit(Fake::new("already done", [Step::Finish]));
    assert!(matches!(settle(&queue, &recorder, finished).await, JobState::Done));

    let running = Fake::new("mid generation", [Step::AwaitCancel]);
    let running_log = running.log();
    let running = queue.submit(running);
    let queued = Fake::new("waiting behind it", [Step::Finish]);
    let queued_log = queued.log();
    let queued = queue.submit(queued);
    recorder.until(|| running_log.started() == 1).await;

    assert_eq!(queue.active(), 2, "one running, one queued");
    assert_eq!(queue.close(), 2, "both unfinished jobs are asked to stop");
    assert!(queue.quiesce(Duration::from_secs(5)).await, "the queue did not wind down");

    assert_eq!(queue.active(), 0);
    assert!(matches!(state_of(&queue, running), Some(JobState::Cancelled)));
    assert!(matches!(state_of(&queue, queued), Some(JobState::Cancelled)));
    assert_eq!(queued_log.started(), 0, "a queued job must never start during a shutdown");
    assert!(
        matches!(state_of(&queue, finished), Some(JobState::Done)),
        "a job that had already finished is not rewritten by the shutdown"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_that_ignores_its_token_is_still_gone_when_the_queue_quiesces() {
    // The case the budget exists for, and the one that decides whether a quit
    // hangs: an adapter wedged in a socket that never looks at the token. The
    // queue's own `cancel_grace` aborts it, so `quiesce` returns on time rather
    // than waiting out a provider's idea of a timeout.
    let (queue, recorder) =
        queue_with(Config { cancel_grace: Duration::from_millis(50), ..Config::default() });
    let task = Fake::new("wedged", [Step::Ignore]);
    let log = task.log();
    let id = queue.submit(task);
    recorder.until(|| log.started() == 1).await;

    queue.close();
    assert!(queue.quiesce(Duration::from_secs(5)).await);
    assert!(matches!(state_of(&queue, id), Some(JobState::Cancelled)));
    assert_eq!(log.dropped(), 1, "the future should have been aborted, not merely ignored");
}

#[tokio::test(flavor = "multi_thread")]
async fn quiesce_gives_up_rather_than_holding_the_window_open_for_ever() {
    // A shutdown that cannot finish must still finish. `Duration::ZERO` is the
    // degenerate budget, and the answer has to be "no", not a hang.
    let (queue, recorder) = queue_with(Config::default());
    let task = Fake::new("still going", [Step::AwaitCancel]);
    let log = task.log();
    queue.submit(task);
    recorder.until(|| log.started() == 1).await;

    assert!(!queue.quiesce(Duration::ZERO).await);
    assert_eq!(queue.active(), 1, "quiesce alone must not cancel anything");
}

#[tokio::test(flavor = "multi_thread")]
async fn work_submitted_after_the_queue_closed_is_born_cancelled() {
    // A command racing the teardown. Starting a paid call while the process is
    // on its way out is the one outcome worth being strict about, and the
    // submitter still gets an id and a terminal state rather than silence.
    let (queue, recorder) = queue_with(Config::default());
    assert_eq!(queue.close(), 0, "an idle queue has nothing to stop");
    assert!(queue.is_closed());

    let task = Fake::new("too late", [Step::Finish]);
    let log = task.log();
    let id = queue.submit(task);

    assert!(matches!(settle(&queue, &recorder, id).await, JobState::Cancelled));
    assert_eq!(log.started(), 0, "a job admitted to a closed queue must never run");
    assert!(queue.quiesce(Duration::from_secs(5)).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn quiescing_an_already_idle_queue_returns_at_once() {
    // The common case on exit: nothing was running, and the user should not
    // wait out a budget to find that out.
    let (queue, _recorder) = queue_with(Config::default());
    let began = std::time::Instant::now();
    assert!(queue.quiesce(Duration::from_secs(30)).await);
    assert!(began.elapsed() < Duration::from_secs(1), "{:?}", began.elapsed());
}
