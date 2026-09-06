//! The queue: admission, cancellation, retries, and the promise that one bad
//! job cannot take the others with it.
//!
//! ## Shape
//!
//! One tokio task per job, and a [`Semaphore`] between them. That is the whole
//! scheduler. There is no dispatcher loop, and the reason is the two properties
//! that fall out of doing it this way:
//!
//! - **Admission is FIFO and starvation-free**, because tokio's semaphore hands
//!   out permits in the order they were asked for. A dispatcher picking the next
//!   job out of a map would have had to re-derive that, and would have got it
//!   subtly wrong the first time somebody added a priority.
//! - **Cancelling a job that has not started is trivially correct.** The job's
//!   task is parked on `acquire_owned`, racing [`Cancel::cancelled`]; if the
//!   cancellation wins, the task returns without ever having held a permit and
//!   the work simply never happens. There is no queue entry to find and remove,
//!   so there is no window in which it has been removed from one place and not
//!   yet another.
//!
//! What a job holds and when it holds it is the other half. A slot is taken when
//! an attempt starts and given back the moment it ends — *including before a
//! retry backoff*. A job sleeping out a forty-second rate limit while holding
//! one of three slots would be starving the queue on behalf of a provider that
//! has already said it is not listening.
//!
//! ## Ordering and fairness, stated
//!
//! - Jobs start in submission order.
//! - A job that backs off re-enters at the tail, so a job that keeps failing
//!   cannot hold the front of the queue against newer work.
//! - There are no priorities, so nothing can jump the line and nothing can be
//!   starved by something that did.
//! - Completion order is not ordering: three jobs of wildly different lengths
//!   finish in whatever order they finish.
//!
//! ## Panics
//!
//! The task's future is spawned in its own tokio task and joined, so a panic
//! arrives here as a `JoinError` rather than as an unwind through the queue. The
//! job is failed with `internal`, never retried — a panic is our bug and
//! retrying it spends money to reach the same bug — and its permit is released
//! by the ordinary path. The queue keeps running and the other jobs never notice.
//!
//! This is the one thing here that a build setting can defeat: under
//! `panic = "abort"` there is no unwind to catch and the process dies. The
//! workspace does not set it, and this is the reason not to.
//!
//! ## Shutdown
//!
//! The process leaving is the one event the queue cannot ride out, and dropping
//! it on the floor is not free: an image job killed by `exit(0)` has usually
//! already been billed, and a ComfyUI run has a prompt on somebody's GPU that
//! nothing will ever interrupt. So there is an explicit wind-down —
//! [`Queue::close`], then [`Queue::quiesce`] — and `src-tauri/src/shutdown.rs`
//! is its only caller. See `docs/15-exit-policy.md`.
//!
//! Two properties make that wind-down worth having rather than theatre:
//!
//! - **Closing is cancelling.** [`Queue::close`] cancels every unfinished job
//!   through the same token a Stop button uses, so every adapter's existing
//!   cancellation path runs — including ComfyUI's `/interrupt` and the
//!   `Billed` report that tells the user whether they were charged.
//! - **A closed queue cannot be re-armed.** A command racing the teardown could
//!   otherwise start a paid call during shutdown. A job submitted after `close`
//!   is born cancelled: it is admitted, recorded and finished as `cancelled`
//!   without ever running, so the submitter still gets an id and still gets an
//!   answer rather than a job that hangs in `queued` forever.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinError;

use crate::Cancel;
use crate::event::{DoneEvent, Event, FailedEvent, Notify, RetryEvent};
use crate::job::{JobContext, JobId, JobKind, JobSnapshot, JobState, Outcome, QueueSnapshot, Task};
use crate::retry::{Attempts, Billed, Failure, RetryPolicy, Verdict, decide};

/// How the queue is set up. All four have defaults that are right for this app;
/// they exist as knobs because the provider decides two of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// How many jobs may run at once.
    ///
    /// Three, because Hunyuan3D allows exactly three concurrent Pro jobs and
    /// will rate-limit us for a fourth — and a self-inflicted 429 is worse than
    /// a queue, since it costs a round trip and looks like the provider's fault.
    /// Adjustable at runtime with [`Queue::set_concurrency`], because the right
    /// number is a property of whichever backend the project has selected.
    pub concurrency: usize,
    /// The retry rules for jobs submitted without their own.
    pub retry: RetryPolicy,
    /// How long a cancelled job is given to stop on its own before its future is
    /// aborted.
    ///
    /// Not zero, and the reason is money: an adapter that notices its token
    /// still has to unwind far enough to report what the provider charged, or
    /// the user is told a cancelled job cost nothing on no evidence. Not long
    /// either — past a couple of seconds the user has pressed Stop twice and
    /// started wondering what the button does.
    pub cancel_grace: Duration,
    /// How many finished jobs to keep in the snapshot.
    ///
    /// Finished jobs stay visible so the status bar can show the last outcome
    /// rather than an empty queue and no explanation. Bounded because nothing
    /// clears this but the process ending.
    pub history: usize,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            concurrency: 3,
            retry: RetryPolicy::default(),
            cancel_grace: Duration::from_secs(2),
            history: 32,
        }
    }
}

/// A handle to the queue. Cheap to clone; every clone is the same queue.
#[derive(Clone)]
pub struct Queue(Arc<Inner>);

struct Inner {
    permits: Arc<Semaphore>,
    notify: Arc<dyn Notify>,
    runtime: Handle,
    grace: Duration,
    history: usize,
    default_retry: RetryPolicy,
    registry: Mutex<Registry>,
    /// Set by [`Queue::close`] and never cleared. A queue that could be
    /// reopened would be a shutdown that a slow command could undo.
    closed: AtomicBool,
    /// Rung whenever the last unfinished job finishes. [`Queue::quiesce`] is
    /// the only listener, and it re-checks the registry on every wake, so a
    /// spurious ring costs one comparison and a missed one is impossible —
    /// the waiter is registered before it looks.
    idle: tokio::sync::Notify,
}

/// Everything the queue knows about jobs, behind one lock.
///
/// A `std::sync::Mutex` rather than `parking_lot`'s: this crate is a dependency
/// of the shell, not the other way round, and the critical sections here are a
/// handful of map operations with no IO and no awaits in them. Nothing is ever
/// notified while it is held — see [`Inner::transition`].
#[derive(Default)]
struct Registry {
    /// Submission order. The snapshot is rendered from this, so the status bar
    /// shows jobs in the order the user started them rather than in whatever
    /// order a hash map felt like.
    order: VecDeque<JobId>,
    jobs: HashMap<JobId, Record>,
    /// The concurrency cap as last set.
    cap: usize,
    /// Permits that have been promised away by a *lowered* cap and not yet
    /// reclaimed, because they were in use at the time. Settled as running jobs
    /// hand their permits back. Without this, lowering the cap while three jobs
    /// are in flight would silently do nothing.
    debt: usize,
}

struct Record {
    kind: JobKind,
    label: String,
    subject_id: Option<String>,
    state: JobState,
    attempt: u32,
    started_at: Option<Instant>,
    elapsed_ms: u64,
    cancel: Cancel,
}

impl Queue {
    /// Start a queue on the current runtime.
    ///
    /// Panics outside a runtime context, which is the right failure: everything
    /// below spawns, and a queue that accepted work it could never run would
    /// report jobs as queued forever.
    pub fn new(config: Config, notify: impl Notify) -> Queue {
        Queue::with_runtime(config, notify, Handle::current())
    }

    /// Start a queue on a runtime named explicitly.
    ///
    /// The shell uses this: `tauri::async_runtime::handle()` is reachable from
    /// `setup`, where `Handle::current` is not, and the queue must run on
    /// Tauri's runtime rather than a private one of its own.
    pub fn with_runtime(config: Config, notify: impl Notify, runtime: Handle) -> Queue {
        // A cap of zero would accept jobs and never start one, which is a hang
        // wearing a configuration value's clothes.
        let cap = config.concurrency.max(1);
        Queue(Arc::new(Inner {
            permits: Arc::new(Semaphore::new(cap)),
            notify: Arc::new(notify),
            runtime,
            grace: config.cancel_grace,
            history: config.history,
            default_retry: config.retry,
            registry: Mutex::new(Registry { cap, ..Registry::default() }),
            closed: AtomicBool::new(false),
            idle: tokio::sync::Notify::new(),
        }))
    }

    /// Queue a task and return immediately.
    ///
    /// The id is live before this returns, so a caller can hand it straight to
    /// the webview and a cancellation that arrives a millisecond later finds
    /// something to cancel.
    pub fn submit<T: Task>(&self, task: T) -> JobId {
        let retry = self.0.default_retry;
        self.submit_with(task, retry)
    }

    /// Queue a task with retry rules of its own.
    ///
    /// This is where a caller opts into paid retries
    /// ([`RetryPolicy::with_paid_attempts`]) — deliberately at submission, by
    /// the code that knows what the call is worth, rather than deep inside the
    /// queue where nothing knows anything about money.
    pub fn submit_with<T: Task>(&self, task: T, retry: RetryPolicy) -> JobId {
        let id = JobId::new();
        let cancel = Cancel::new();
        // Born cancelled on a closed queue. The job is still recorded and still
        // reported, because a caller that got an id back and never heard
        // anything again would be a spinner nobody clears — it simply never
        // runs, which is the whole point during a shutdown.
        if self.0.closed.load(Ordering::SeqCst) {
            cancel.cancel();
        }
        let snapshot = {
            let mut registry = self.0.lock();
            registry.order.push_back(id);
            registry.jobs.insert(
                id,
                Record {
                    kind: task.kind(),
                    label: task.label(),
                    subject_id: task.subject_id(),
                    state: JobState::Queued,
                    attempt: 0,
                    started_at: None,
                    elapsed_ms: 0,
                    cancel: cancel.clone(),
                },
            );
            registry.snapshot()
        };
        self.0.notify.notify(Event::State(snapshot));

        let inner = Arc::clone(&self.0);
        self.0.runtime.spawn(drive(inner, id, Box::new(task), retry, cancel));
        id
    }

    /// Ask a job to stop. `false` if there is no such job, or it had already
    /// finished.
    ///
    /// Returns as soon as the flag is set rather than waiting for the job to
    /// wind down, because the caller is a Tauri command and the whole point is
    /// that the frontend does not block. The state change follows on
    /// `job:state` a moment later — immediately for a job that had not started,
    /// and within [`Config::cancel_grace`] at the very worst.
    pub fn cancel(&self, id: JobId) -> bool {
        // Taken out from under the lock: cancelling wakes whoever is parked on
        // the token, and waking runs code we do not control on this thread.
        let cancel = {
            let registry = self.0.lock();
            registry
                .jobs
                .get(&id)
                .filter(|record| !record.state.is_terminal())
                .map(|record| record.cancel.clone())
        };
        match cancel {
            Some(cancel) => {
                cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Ask every job that has not finished to stop. Returns how many were asked.
    ///
    /// The same token, and therefore the same path, as a Stop button — which is
    /// what makes this worth doing at all rather than letting the process take
    /// them: an adapter told to stop reports what it was billed and tells the
    /// provider, and one that is merely killed does neither.
    pub fn cancel_all(&self) -> usize {
        // Collected under the lock and cancelled outside it, for the reason
        // [`cancel`](Self::cancel) gives: waking a token runs code we do not
        // control on this thread.
        let tokens: Vec<Cancel> = {
            let registry = self.0.lock();
            registry
                .jobs
                .values()
                .filter(|record| !record.state.is_terminal())
                .map(|record| record.cancel.clone())
                .collect()
        };
        let asked = tokens.len();
        for cancel in tokens {
            cancel.cancel();
        }
        asked
    }

    /// Stop accepting work, and stop the work already here. Returns how many
    /// jobs were asked to stop.
    ///
    /// Idempotent, and one-way: see the module header for why a closed queue is
    /// never reopened.
    pub fn close(&self) -> usize {
        self.0.closed.store(true, Ordering::SeqCst);
        self.cancel_all()
    }

    pub fn is_closed(&self) -> bool {
        self.0.closed.load(Ordering::SeqCst)
    }

    /// Everything that has not finished — the number [`quiesce`](Self::quiesce)
    /// is waiting to reach zero.
    pub fn active(&self) -> usize {
        self.0.lock().active()
    }

    /// Wait until nothing is queued, running or retrying. `false` if `budget`
    /// ran out first.
    ///
    /// Called after [`close`](Self::close) and never instead of it — on an open
    /// queue this would be a wait for the user to stop working. The budget is
    /// not negotiable for the same reason `SyncManager::shutdown` has one: a
    /// quit that hangs is worse than a job that is cut off, and an adapter
    /// wedged inside a provider's socket is exactly the case where waiting for
    /// ever is the tempting mistake. The queue's own `cancel_grace` already
    /// bounds each job, so reaching the budget means something below that is
    /// wrong rather than merely slow.
    pub async fn quiesce(&self, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        loop {
            // Registered *before* the check, and `enable` is what makes that
            // true: `Notified` does not join the waiter list until it is first
            // polled, so building it and checking afterwards would still lose
            // the wake from a job that finished in between.
            let mut waiter = std::pin::pin!(self.0.idle.notified());
            waiter.as_mut().enable();
            if self.active() == 0 {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            if tokio::time::timeout(left, waiter).await.is_err() {
                return self.active() == 0;
            }
        }
    }

    /// Everything the status bar needs, for a webview that has just reloaded and
    /// missed the events.
    pub fn snapshot(&self) -> QueueSnapshot {
        self.0.lock().snapshot()
    }

    pub fn concurrency(&self) -> usize {
        self.0.lock().cap
    }

    /// Change how many jobs may run at once.
    ///
    /// Raising takes effect immediately. Lowering takes effect as fast as it
    /// honestly can: permits that are free are withdrawn now, and permits that
    /// are in use are withdrawn when their jobs give them back. Nothing running
    /// is interrupted — a cap is about how much to start, and killing paid work
    /// in flight to satisfy a settings change would be an expensive way to obey
    /// an instruction nobody meant that literally.
    pub fn set_concurrency(&self, cap: usize) {
        let cap = cap.max(1);
        let mut registry = self.0.lock();
        if cap > registry.cap {
            // Raising cancels an unpaid lowering before it adds anything, or the
            // two would cancel out later at the worst possible moment.
            let mut adding = cap - registry.cap;
            let settled = adding.min(registry.debt);
            registry.debt -= settled;
            adding -= settled;
            self.0.permits.add_permits(adding);
        } else if cap < registry.cap {
            let mut owed = registry.cap - cap;
            owed -= self.0.permits.forget_permits(owed);
            registry.debt += owed;
        }
        registry.cap = cap;
    }
}

impl Inner {
    /// A poisoned registry means a panic happened while it was held, which the
    /// code below never does — but stepping over it rather than propagating is
    /// still right: refusing to run any further jobs because one map operation
    /// once unwound would be a queue that stops working and never says why.
    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn describe(&self, id: JobId) -> Option<(JobKind, String)> {
        self.lock().jobs.get(&id).map(|record| (record.kind, record.label.clone()))
    }

    /// Move a job to a new state and publish the whole queue.
    ///
    /// The snapshot is built under the lock and the notification sent after it
    /// is released. A `Notify` that reached back into the queue — for a depth to
    /// put in a log line, say — would otherwise deadlock on a mutex its own
    /// caller is holding, and it would do it only sometimes.
    fn transition(&self, id: JobId, state: JobState, attempt: Option<u32>) {
        let (snapshot, idle) = {
            let mut registry = self.lock();
            let terminal = state.is_terminal();
            match registry.jobs.get_mut(&id) {
                Some(record) => {
                    if matches!(&state, JobState::Running) && record.started_at.is_none() {
                        record.started_at = Some(Instant::now());
                    }
                    if terminal {
                        record.elapsed_ms = elapsed_ms(record.started_at);
                    }
                    record.state = state;
                    if let Some(attempt) = attempt {
                        record.attempt = attempt;
                    }
                }
                None => return,
            }
            if terminal {
                registry.prune(self.history);
            }
            (registry.snapshot(), registry.active() == 0)
        };
        // Outside the lock, like the notification below it and for the same
        // reason: a woken waiter may run on this thread immediately.
        if idle {
            self.idle.notify_waiters();
        }
        self.notify.notify(Event::State(snapshot));
    }

    fn finish_done(&self, id: JobId, result: Option<Value>) {
        if let Some((kind, label)) = self.describe(id) {
            self.notify.notify(Event::Done(DoneEvent { id, kind, label, result }));
        }
        self.transition(id, JobState::Done, None);
    }

    /// No `job:error` for a cancellation. The user pressed Stop; an error toast
    /// telling them it stopped is the app arguing with them, and
    /// `src-tauri/src/error.rs` already says as much about the `cancelled` code.
    fn finish_cancelled(&self, id: JobId) {
        self.transition(id, JobState::Cancelled, None);
    }

    fn finish_failed(&self, id: JobId, failure: Failure, retry_held: bool) {
        if let Some((kind, label)) = self.describe(id) {
            self.notify.notify(Event::Failed(FailedEvent {
                id,
                kind,
                label,
                failure: failure.clone(),
                retry_held,
            }));
        }
        self.transition(id, JobState::Failed { failure, retry_held }, None);
    }

    /// Say that another attempt is coming, *before* waiting for it.
    ///
    /// Order matters more than content here. After the wait this would be a
    /// receipt for money already spent; before it, it is the notice that makes
    /// "never auto-retry something that costs money without saying so" true —
    /// and the only chance anyone has to press Stop in between.
    fn announce_retry(
        &self,
        id: JobId,
        attempt: u32,
        max_attempts: u32,
        delay: Duration,
        costs_money: bool,
        failure: &Failure,
    ) {
        let Some((kind, label)) = self.describe(id) else { return };
        self.notify.notify(Event::Retry(RetryEvent {
            id,
            kind,
            label,
            attempt,
            max_attempts,
            in_ms: delay.as_millis() as u64,
            costs_money,
            failure: failure.clone(),
        }));
    }

    /// Hand a slot back, honouring a cap that was lowered while it was in use.
    fn release(&self, permit: OwnedSemaphorePermit) {
        let mut registry = self.lock();
        if registry.debt > 0 {
            registry.debt -= 1;
            permit.forget();
        } else {
            drop(permit);
        }
    }
}

impl Registry {
    /// Jobs that have not reached a terminal state, history excluded.
    fn active(&self) -> usize {
        self.jobs.values().filter(|record| !record.state.is_terminal()).count()
    }

    fn snapshot(&self) -> QueueSnapshot {
        let mut queued = 0;
        let mut running = 0;
        let mut retrying = 0;
        let jobs = self
            .order
            .iter()
            .filter_map(|id| self.jobs.get(id).map(|record| (id, record)))
            .map(|(id, record)| {
                match record.state {
                    JobState::Queued => queued += 1,
                    JobState::Running => running += 1,
                    JobState::Retrying { .. } => retrying += 1,
                    _ => {}
                }
                JobSnapshot {
                    id: *id,
                    kind: record.kind,
                    label: record.label.clone(),
                    subject_id: record.subject_id.clone(),
                    state: record.state.clone(),
                    attempt: record.attempt,
                    elapsed_ms: if record.state.is_terminal() {
                        record.elapsed_ms
                    } else {
                        elapsed_ms(record.started_at)
                    },
                }
            })
            .collect();
        QueueSnapshot { jobs, queued, running, retrying }
    }

    /// Forget the oldest finished jobs once there are more than `keep` of them.
    ///
    /// Only finished ones: a queue that dropped a running job from its own
    /// registry would have nothing left to cancel it by.
    fn prune(&mut self, keep: usize) {
        let terminal = |registry: &Registry, id: &JobId| {
            registry.jobs.get(id).is_some_and(|record| record.state.is_terminal())
        };
        let mut finished = self.order.iter().filter(|id| terminal(self, id)).count();
        while finished > keep {
            let Some(position) = self.order.iter().position(|id| terminal(self, id)) else {
                break;
            };
            if let Some(id) = self.order.remove(position) {
                self.jobs.remove(&id);
            }
            finished -= 1;
        }
    }
}

fn elapsed_ms(started_at: Option<Instant>) -> u64 {
    started_at
        .map(|started| started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// One job, start to finish, including every attempt it makes.
async fn drive(
    inner: Arc<Inner>,
    id: JobId,
    mut task: Box<dyn Task>,
    retry: RetryPolicy,
    cancel: Cancel,
) {
    let mut attempts = Attempts::default();

    loop {
        let Some(permit) = admit(&inner, &cancel).await else {
            inner.finish_cancelled(id);
            return;
        };

        attempts.total += 1;
        inner.transition(id, JobState::Running, Some(attempts.total));

        let context =
            JobContext::new(id, attempts.total, cancel.clone(), Arc::clone(&inner.notify));
        // Spawned rather than awaited inline, and this is the panic barrier: an
        // unwind out of a task's own future arrives here as a `JoinError`
        // instead of tearing down the job driver, which would leak the permit
        // and leave the job stuck in `running` for the life of the process.
        let mut running = inner.runtime.spawn(async move {
            let mut task = task;
            let outcome = task.run(&context).await;
            (task, outcome)
        });

        let joined = tokio::select! {
            // Biased so that a token already set wins against a task that has
            // just finished. A job cancelled at the same instant it completes is
            // reported as cancelled either way; what must not happen is the
            // cancellation being missed and a retry starting.
            biased;
            () = cancel.cancelled() => None,
            joined = &mut running => Some(joined),
        };

        let joined = match joined {
            Some(joined) => joined,
            // Cancelled while running. The token is already set, so the task has
            // been told; this is the grace it gets to stop on its own terms —
            // which for a provider adapter means unwinding far enough to report
            // what it was charged before the socket closes.
            None => match tokio::time::timeout(inner.grace, &mut running).await {
                Ok(joined) => joined,
                Err(_) => {
                    // It did not stop when asked. Aborting drops the future at
                    // its next await point, which drops the response body and
                    // closes the connection — the difference between cancelling
                    // a job and merely ignoring it while the user is billed for
                    // the rest of it.
                    running.abort();
                    let _ = running.await;
                    inner.release(permit);
                    inner.finish_cancelled(id);
                    return;
                }
            },
        };

        let outcome = match joined {
            Ok((returned, outcome)) => {
                task = returned;
                outcome
            }
            Err(join) if join.is_cancelled() => {
                inner.release(permit);
                inner.finish_cancelled(id);
                return;
            }
            Err(join) => {
                inner.release(permit);
                inner.finish_failed(id, panicked(&join), false);
                return;
            }
        };

        // The token is authoritative over what the task claims. A task that
        // reports a failure after being cancelled is reporting the cancellation,
        // and running that through the retry rules could start a fresh paid
        // attempt for someone who pressed Stop. A `Done` is kept, though: the
        // work happened and was paid for, and throwing it away would be a second
        // cost on top of the first.
        if cancel.is_cancelled() && !matches!(outcome, Outcome::Done(_)) {
            inner.release(permit);
            inner.finish_cancelled(id);
            return;
        }

        let failure = match outcome {
            Outcome::Done(result) => {
                inner.release(permit);
                inner.finish_done(id, result);
                return;
            }
            Outcome::Cancelled => {
                inner.release(permit);
                inner.finish_cancelled(id);
                return;
            }
            Outcome::Failed(failure) => failure,
        };

        let verdict = decide(&retry, attempts, &failure);
        // Before the wait, always. A job holding one of three slots while it
        // sleeps out a rate limit is starving the queue on behalf of a provider
        // that has already said it is not listening.
        inner.release(permit);

        let Some(delay) = verdict.delay() else {
            inner.finish_failed(id, failure, verdict == Verdict::Hold);
            return;
        };
        if verdict.costs_money() {
            attempts.paid += 1;
        }
        inner.announce_retry(
            id,
            attempts.total,
            retry.max_attempts,
            delay,
            verdict.costs_money(),
            &failure,
        );
        let waiting = JobState::Retrying {
            in_ms: delay.as_millis() as u64,
            costs_money: verdict.costs_money(),
        };
        inner.transition(id, waiting, None);

        tokio::select! {
            biased;
            // Stop during a backoff is the cheapest cancel there is, and the
            // most likely: a job visibly waiting to try again is exactly when a
            // user reaches for the button.
            () = cancel.cancelled() => {
                inner.finish_cancelled(id);
                return;
            }
            () = tokio::time::sleep(delay) => {}
        }
        inner.transition(id, JobState::Queued, None);
    }
}

/// Wait for a slot, unless the job is cancelled first.
///
/// `None` means the job was cancelled before it ever ran — the cheapest possible
/// cancel, and the one that must never accidentally start the work. Both checks
/// are needed: the race covers a job waiting behind others, and the second look
/// covers the instant between a permit being granted and it being used.
async fn admit(inner: &Arc<Inner>, cancel: &Cancel) -> Option<OwnedSemaphorePermit> {
    let permits = Arc::clone(&inner.permits);
    let permit = tokio::select! {
        biased;
        () = cancel.cancelled() => return None,
        // Fair: tokio hands permits out in the order they were asked for, which
        // is what makes admission FIFO without a scheduler here to make it so.
        permit = permits.acquire_owned() => permit.ok()?,
    };
    if cancel.is_cancelled() {
        inner.release(permit);
        return None;
    }
    Some(permit)
}

/// A panicking task, described as a failure.
///
/// `internal` because it is our bug, not the provider's and not the user's, and
/// never retryable for the same reason: the second attempt reaches the same
/// panic and the first one may already have been paid for. [`Billed::Unknown`]
/// rather than `Nothing` — an unwind out of the middle of a call says nothing
/// about whether the call had already been billed.
fn panicked(join: &JoinError) -> Failure {
    Failure::new("internal", "The job stopped unexpectedly.")
        .billed(Billed::Unknown)
        .with_detail(join.to_string())
}
