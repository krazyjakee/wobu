//! What the queue says, and to whom.
//!
//! The queue never emits anything itself. It hands an [`Event`] to a [`Notify`],
//! and the shell's implementation is the three lines that turn one into
//! `app.emit`. That indirection is the difference between a queue with tests and
//! a queue whose behaviour is only observable by running the app and watching a
//! status bar.
//!
//! The event names are here rather than in the shell because they are a contract
//! with `src/lib/api.ts`, not an implementation detail of the shell — the same
//! reason the error codes in `wobu-llm` are copied rather than invented. Two of
//! them are not in `docs/05-architecture.md`'s original list and both earn their
//! place: `job:state` is how the status bar knows the depth without
//! reconstructing it from a stream of deltas, and `job:retry` is where the queue
//! says out loud that it is about to spend money again.

use serde::Serialize;
use serde_json::Value;

use crate::job::{JobId, JobKind, QueueSnapshot};
use crate::retry::Failure;

/// The Tauri event names. Kept beside the payloads so the two cannot drift, and
/// mirrored in `src/lib/api.ts`.
pub mod events {
    /// The whole queue, on every transition. Payload: `QueueSnapshot`.
    pub const JOB_STATE: &str = "job:state";
    /// Payload: `ProgressEvent`.
    pub const JOB_PROGRESS: &str = "job:progress";
    /// Payload: `PreviewEvent`.
    pub const JOB_PREVIEW: &str = "job:preview";
    /// About to try again. Payload: `RetryEvent`.
    pub const JOB_RETRY: &str = "job:retry";
    /// Payload: `DoneEvent`.
    pub const JOB_DONE: &str = "job:done";
    /// Payload: `FailedEvent`. Cancellation is *not* one of these — a user who
    /// pressed Stop does not need to be told, and `job:state` already carries
    /// it.
    pub const JOB_ERROR: &str = "job:error";
}

/// Everything the queue has to say.
///
/// One enum with a name per variant rather than six callbacks, so an
/// implementation cannot silently ignore a kind of event by forgetting to
/// override a defaulted method.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    State(QueueSnapshot),
    Progress(ProgressEvent),
    Preview(PreviewEvent),
    Retry(RetryEvent),
    Done(DoneEvent),
    Failed(FailedEvent),
}

impl Event {
    /// The event name this goes out under.
    pub fn name(&self) -> &'static str {
        match self {
            Event::State(_) => events::JOB_STATE,
            Event::Progress(_) => events::JOB_PROGRESS,
            Event::Preview(_) => events::JOB_PREVIEW,
            Event::Retry(_) => events::JOB_RETRY,
            Event::Done(_) => events::JOB_DONE,
            Event::Failed(_) => events::JOB_ERROR,
        }
    }
}

/// Where events go.
///
/// **Implementations must not block and must not call back into the queue.**
/// The queue calls this from the task driving a job, with no lock held — but a
/// notifier that took a slow path here would stall the job it belongs to, and
/// one that reached back for a snapshot would be asking a lock a question its
/// own caller is answering. `app.emit` is neither, which is the whole design.
pub trait Notify: Send + Sync + 'static {
    fn notify(&self, event: Event);
}

/// So that a caller can keep a handle on its own notifier after handing one to
/// the queue. The shell wants this to keep the `AppHandle` bridge addressable,
/// and a test wants it to read back what was emitted.
impl<T: Notify + ?Sized> Notify for std::sync::Arc<T> {
    fn notify(&self, event: Event) {
        (**self).notify(event);
    }
}

/// A notifier that throws everything away, for callers that only want the
/// return value — a batch run, a test of something else. Named rather than an
/// empty closure so that reaching for it reads as a decision.
pub struct Silent;

impl Notify for Silent {
    fn notify(&self, _event: Event) {}
}

/// How far through a job is.
///
/// Same `done`/`total` shape as `wobu_store::ScanProgress`, deliberately: the
/// two are read by the same eyes and there is no reason for a progress bar to
/// be told about a scan differently from a generation. `note` is the extra —
/// ComfyUI reports a sampler and a step, and "sampling 12/30" is more use than
/// "40%".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub done: u32,
    pub total: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Progress {
    pub fn new(done: u32, total: u32) -> Progress {
        Progress { done, total, note: None }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Progress {
        self.note = Some(note.into());
        self
    }

    /// 0–100, saturating. `total == 0` reads as complete rather than as a
    /// division by zero — the same rule as `ScanProgress::percent`, for the same
    /// reason: a step count that turns out to be wrong should not be able to
    /// produce 150% or a panic.
    pub fn percent(&self) -> u8 {
        if self.total == 0 {
            return 100;
        }
        ((self.done.min(self.total) as u64 * 100) / self.total as u64) as u8
    }
}

/// A picture of the work in flight — a latent preview, a first-pass render.
///
/// `image` is deliberately an opaque string. Whether ComfyUI's previews reach
/// the webview as a `data:` URL, an `asset://` path or something else is #40's
/// decision and it is not settled; what is settled is that the queue does not
/// buffer image bytes on the way past.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub image: String,
    /// Which step it came from, so a preview that arrives out of order can be
    /// dropped instead of flickering backwards.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
}

impl Preview {
    pub fn new(image: impl Into<String>) -> Preview {
        Preview { image: image.into(), step: None }
    }

    pub fn at_step(mut self, step: u32) -> Preview {
        self.step = Some(step);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub id: JobId,
    /// Flattened so the payload is `{ id, done, total }` rather than a nested
    /// object the frontend has to reach through.
    #[serde(flatten)]
    pub progress: Progress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewEvent {
    pub id: JobId,
    #[serde(flatten)]
    pub preview: Preview,
}

/// The queue is about to try again, and this is emitted *before* the wait
/// starts.
///
/// The ordering is the point of the event. Emitted afterwards it would be a
/// receipt; emitted here it is a warning, and when `costs_money` is true it is
/// the only warning the user gets before their card is charged a second time.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryEvent {
    pub id: JobId,
    pub kind: JobKind,
    pub label: String,
    /// The attempt that failed.
    pub attempt: u32,
    /// How many attempts the policy allows in total, so the UI can say "2 of 4"
    /// rather than counting forever.
    pub max_attempts: u32,
    pub in_ms: u64,
    /// Whether the attempt being waited for will be billed. True only when the
    /// submitter allowed paid retries.
    pub costs_money: bool,
    /// What went wrong, including what the failed attempt cost.
    pub failure: Failure,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoneEvent {
    pub id: JobId,
    pub kind: JobKind,
    pub label: String,
    /// Whatever the task decided its caller needs. Absent for work whose result
    /// is on disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedEvent {
    pub id: JobId,
    pub kind: JobKind,
    pub label: String,
    pub failure: Failure,
    /// The queue could have retried and would not, because the attempt cost
    /// money. The UI's answer to this is an offer, not an apology.
    pub retry_held: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_saturating_and_never_divides_by_zero() {
        // A step count from a backend can be zero or can turn out to be wrong;
        // neither is a reason to panic or to draw a 150% bar.
        assert_eq!(Progress::new(0, 0).percent(), 100);
        assert_eq!(Progress::new(0, 4).percent(), 0);
        assert_eq!(Progress::new(2, 4).percent(), 50);
        assert_eq!(Progress::new(9, 4).percent(), 100);
        assert_eq!(Progress::new(u32::MAX, u32::MAX).percent(), 100);
    }

    #[test]
    fn a_progress_payload_is_flat_on_the_wire() {
        // The regression: `{ id, progress: { done } }`, which `src/lib/api.ts`
        // does not describe and which no Rust-side assertion would catch.
        let event = ProgressEvent {
            id: JobId::new(),
            progress: Progress::new(12, 30).with_note("sampling"),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["done"], 12);
        assert_eq!(json["total"], 30);
        assert_eq!(json["note"], "sampling");
        assert!(json.get("progress").is_none());
    }

    #[test]
    fn every_event_has_a_name_and_no_two_share_one() {
        // The shell dispatches on these. Two events under one name would have
        // the frontend parsing one payload as another.
        let names = [
            events::JOB_STATE,
            events::JOB_PROGRESS,
            events::JOB_PREVIEW,
            events::JOB_RETRY,
            events::JOB_DONE,
            events::JOB_ERROR,
        ];
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
        let snapshot = QueueSnapshot { jobs: vec![], queued: 0, running: 0, retrying: 0 };
        assert_eq!(Event::State(snapshot).name(), "job:state");
    }
}
