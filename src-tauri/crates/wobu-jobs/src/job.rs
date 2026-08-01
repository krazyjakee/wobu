//! What a job *is*: its id, what it is doing, how far through it is, and the
//! handle the work itself is given.
//!
//! Two words that are easy to run together and are kept apart everywhere below.
//! A [`Task`] is the work — an Enhance call, a ComfyUI prompt — written by
//! whoever needs it and handed over once. A *job* is one queued run of a task:
//! it has an id, a state, an attempt count, and a cancellation token, and it is
//! the thing the status bar draws and `job_cancel` names.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use crate::Cancel;
use crate::event::{Event, Notify, Preview, PreviewEvent, Progress, ProgressEvent};
use crate::retry::Failure;

/// A job's identity, for the lifetime of the process.
///
/// A ULID like everything else with an id here, but its own type rather than
/// `wobu_core::Id`: a job id names a run, not an entity, it is never written to
/// disk, and a function that would accept either is a function that will one day
/// be handed the wrong one. It is deliberately not persisted — a queue that
/// survived a restart would be a queue promising to resume paid calls whose
/// provider-side state is long gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(Ulid);

impl JobId {
    pub fn new() -> JobId {
        JobId(Ulid::generate())
    }

    /// Parse one back from the webview. `None` rather than an error type,
    /// because the only caller is a command boundary that has its own.
    pub fn parse(text: &str) -> Option<JobId> {
        Ulid::from_string(text).ok().map(JobId)
    }
}

impl Default for JobId {
    fn default() -> Self {
        JobId::new()
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What kind of work a job is doing.
///
/// An enum rather than a free string because the UI keys an icon and a label
/// off it, and a typo in a string would be an invisible blank. Adding a variant
/// is a one-line change here and one in `src/lib/api.ts`; that pair is the
/// whole cost of a new kind of long-running work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// A text provider filling in a description (#37).
    Enhance,
    /// An image backend producing a picture (#38–#40).
    Generate,
    /// Image-to-3D (#41). The one with a provider-imposed concurrency cap.
    Mesh,
    /// Thumbnailing an import. Cheap, local, and free — which is why it is
    /// worth having in the same queue as the calls that are not: the retry
    /// rules below are written so that free work is not treated like paid work.
    Thumbnail,
}

/// Where a job is right now.
///
/// Internally tagged so the bridge shape is `{ state: "running", … }` and the
/// frontend can switch on one field. The terminal three are terminal for good:
/// nothing moves out of `done`, `cancelled` or `failed`, so a status bar can
/// stop watching a job the moment it sees one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum JobState {
    /// Submitted, waiting for a slot. Cancelling here is free and the work
    /// never happens.
    Queued,
    /// Holding a slot and running.
    Running,
    /// An attempt failed, the queue is waiting before the next one, and the
    /// slot has already been given back — see `queue.rs` for why holding it
    /// through a sixty-second rate-limit wait would be its own hostage
    /// situation. Which attempt failed is [`JobSnapshot::attempt`]; carrying it
    /// twice would collide on the wire, since the snapshot flattens this in.
    Retrying {
        /// How long the wait is, from the moment this state was published. A
        /// duration rather than an instant because the two sides do not share a
        /// clock, and a countdown that is a second out is fine where a
        /// timestamp from the wrong epoch is not.
        in_ms: u64,
        /// Whether the attempt being waited for will be billed. Never true
        /// unless the submitter asked for paid retries; the UI is expected to
        /// say so out loud when it is.
        costs_money: bool,
    },
    Done,
    Cancelled,
    Failed {
        failure: Failure,
        /// The failure is retryable and the queue refused to retry it on its
        /// own, because the attempt that failed cost money. This is the flag
        /// that turns a dead end into a "try again — it will cost you" in the
        /// UI, and it is the difference between the queue being cautious and
        /// the queue being broken.
        retry_held: bool,
    },
}

impl JobState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, JobState::Done | JobState::Cancelled | JobState::Failed { .. })
    }
}

/// One job as the status bar sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSnapshot {
    pub id: JobId,
    pub kind: JobKind,
    /// What to call it on screen — "Enhance Vashk", not "job 01J…".
    pub label: String,
    #[serde(flatten)]
    pub state: JobState,
    /// Attempts started so far, from 1. Zero while queued.
    pub attempt: u32,
    /// Milliseconds since the first attempt started. Frozen when the job
    /// becomes terminal, so the last-generation stopwatch survives a webview
    /// reload and does not depend on two frontend events arriving.
    pub elapsed_ms: u64,
}

/// The whole queue in one message: what is in it, and how deep it is.
///
/// Sent whole on every transition rather than as a diff. It is a handful of
/// jobs, a diff would have to be reassembled by the receiver, and a receiver
/// that reassembles state is a receiver that can be wrong about it — a webview
/// that reloads mid-generation being the obvious case.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    /// In submission order, oldest first, including a bounded tail of finished
    /// jobs so the last outcome is still on screen after it happened.
    pub jobs: Vec<JobSnapshot>,
    pub queued: usize,
    pub running: usize,
    pub retrying: usize,
}

impl QueueSnapshot {
    /// Everything not yet finished. The number the status bar shows.
    pub fn depth(&self) -> usize {
        self.queued + self.running + self.retrying
    }
}

/// How a run ended, from the task's own point of view.
///
/// [`Outcome::Cancelled`] is a variant rather than a flavour of failure because
/// a task that notices its token and stops cleanly has not failed at anything,
/// and because the difference decides whether a retry is even considered. The
/// queue treats its own token as authoritative either way — a task that returns
/// `Failed` after being cancelled is recorded as cancelled, not as a failure
/// somebody might retry.
#[derive(Debug)]
pub enum Outcome {
    /// Finished. The value rides out on `job:done` and is whatever the caller
    /// of the command needs to hear — an asset id, a node id, nothing at all.
    /// `serde_json::Value` because this crate has no business knowing which.
    Done(Option<Value>),
    Cancelled,
    Failed(Failure),
}

impl Outcome {
    pub fn done() -> Outcome {
        Outcome::Done(None)
    }

    pub fn done_with(value: impl Serialize) -> Outcome {
        Outcome::Done(serde_json::to_value(value).ok())
    }

    pub fn failed(failure: Failure) -> Outcome {
        Outcome::Failed(failure)
    }
}

/// A unit of work the queue can run.
///
/// `#[async_trait]` for the same reason `TextProvider` uses it: tasks are held
/// as `Box<dyn Task>` — the queue cannot be generic over every kind of work in
/// the app — and a native `async fn` in a trait gives no `Send` future to box.
///
/// `&mut self` rather than `&self` so a task can keep whatever it needs between
/// attempts without an interior-mutability dance. The queue owns the task and
/// hands it back to itself between retries.
///
/// Contract for implementors, and all of it is load-bearing:
///
/// 1. **Honour `ctx.cancel()`.** Not by discarding the answer at the end — by
///    stopping. Racing the next read against [`Cancel::cancelled`] is the shape
///    both provider adapters are written to. A task that ignores it is killed
///    after [`crate::Config::cancel_grace`], which is worse for everyone: the
///    task loses its chance to report what it was charged.
/// 2. **Report cost honestly.** [`crate::Billed`] on the returned failure is the
///    only thing standing between the user and a silent second charge, and the
///    queue has no way to check it. When a provider reported usage, say
///    [`crate::Billed::Charged`] whatever the error was.
/// 3. **Do not block the runtime.** This runs on Tauri's runtime, shared with
///    every other job. Filesystem work belongs in `spawn_blocking`.
#[async_trait]
pub trait Task: Send + 'static {
    fn kind(&self) -> JobKind;

    /// One line, in the user's language, naming the thing being worked on.
    /// Built once at submission and never again, so it must not depend on
    /// anything that changes while the job runs.
    fn label(&self) -> String;

    async fn run(&mut self, ctx: &JobContext) -> Outcome;
}

/// What a running task is given: who it is, when to stop, and where to say how
/// it is getting on.
///
/// Cheap to clone, and cloning it is how a task hands progress reporting to a
/// helper without borrowing itself into a corner.
#[derive(Clone)]
pub struct JobContext {
    id: JobId,
    attempt: u32,
    cancel: Cancel,
    notify: Arc<dyn Notify>,
}

impl JobContext {
    pub(crate) fn new(id: JobId, attempt: u32, cancel: Cancel, notify: Arc<dyn Notify>) -> Self {
        JobContext { id, attempt, cancel, notify }
    }

    pub fn id(&self) -> JobId {
        self.id
    }

    /// Which attempt this is, from 1. A task that wants to behave differently
    /// on a retry — a smaller batch, a colder temperature — reads this.
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// The token to hand to a provider call, and to race a read against.
    pub fn cancel(&self) -> &Cancel {
        &self.cancel
    }

    /// Says the same thing as `ctx.cancel().is_cancelled()`, at the boundaries
    /// where a task is looping over work it does itself.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// How far through. Emitted as `job:progress`.
    ///
    /// Fire and forget: there is no back pressure and no delivery guarantee, so
    /// a task that emits one of these per token would be spending the bridge on
    /// nothing. Throttle at the source — `project_open` sends one per whole
    /// percentage point, and that is the pattern to copy.
    pub fn progress(&self, progress: Progress) {
        self.notify.notify(Event::Progress(ProgressEvent { id: self.id, progress }));
    }

    /// An image of the work in flight — a ComfyUI latent preview, mostly.
    pub fn preview(&self, preview: Preview) {
        self.notify.notify(Event::Preview(PreviewEvent { id: self.id, preview }));
    }
}

impl fmt::Debug for JobContext {
    /// Hand-written because `dyn Notify` is not `Debug` and requiring it of
    /// every implementor would be a bound bought with nothing.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobContext")
            .field("id", &self.id)
            .field("attempt", &self.attempt)
            .field("cancelled", &self.cancel.is_cancelled())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_id_survives_the_round_trip_the_webview_puts_it_through() {
        // `job_cancel` receives a string, because that is what JSON has. An id
        // that could not be parsed back would make cancellation impossible for
        // every caller outside Rust.
        let id = JobId::new();
        assert_eq!(JobId::parse(&id.to_string()), Some(id));
        assert_eq!(JobId::parse("not-a-ulid"), None);
        assert_eq!(serde_json::to_value(id).unwrap(), Value::String(id.to_string()));
    }

    #[test]
    fn two_ids_minted_together_are_still_different() {
        // ULIDs share a millisecond timestamp prefix, so a broken source of
        // randomness shows up as collisions between ids created back to back —
        // which would mean one job's cancel stopping another job's work.
        let ids: Vec<JobId> = (0..64).map(|_| JobId::new()).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn only_the_three_terminal_states_are_terminal() {
        // The status bar stops watching a job on this answer. A `retrying` job
        // read as finished would vanish from the UI and then produce a result.
        assert!(!JobState::Queued.is_terminal());
        assert!(!JobState::Running.is_terminal());
        assert!(!JobState::Retrying { in_ms: 1000, costs_money: false }.is_terminal());
        assert!(JobState::Done.is_terminal());
        assert!(JobState::Cancelled.is_terminal());
        assert!(
            JobState::Failed { failure: Failure::new("internal", "boom"), retry_held: false }
                .is_terminal()
        );
    }

    #[test]
    fn a_snapshot_serialises_flat_so_the_frontend_switches_on_one_field() {
        // The regression: a nested `{ state: { state: "running" } }`, which the
        // TypeScript in `src/lib/api.ts` does not describe and no test on this
        // side would notice.
        let snapshot = JobSnapshot {
            id: JobId::new(),
            kind: JobKind::Enhance,
            label: "Enhance Vashk".into(),
            state: JobState::Retrying { in_ms: 4000, costs_money: true },
            attempt: 2,
            elapsed_ms: 1234,
        };
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["state"], "retrying");
        assert_eq!(json["costsMoney"], true);
        assert_eq!(json["inMs"], 4000);
        assert_eq!(json["kind"], "enhance");
        assert_eq!(json["attempt"], 2);
        assert_eq!(json["elapsedMs"], 1234);
    }

    #[test]
    fn depth_counts_everything_that_has_not_finished() {
        // What the status bar shows. Counting only what is running would read
        // as "1 job" while nine sit behind it.
        let snapshot = QueueSnapshot { jobs: vec![], queued: 4, running: 3, retrying: 2 };
        assert_eq!(snapshot.depth(), 9);
    }
}
