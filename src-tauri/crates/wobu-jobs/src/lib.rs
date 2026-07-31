//! The job queue: everything that takes longer than a click, and everything
//! that costs money.
//!
//! The contract the rest of the app is written against is one sentence —
//! *everything long-running returns a job id immediately and streams over
//! events, the frontend never blocks on a command* — and this crate is what
//! makes that true. A command hands a [`Task`] to [`Queue::submit`], gets a
//! [`JobId`] back before the work starts, and the webview learns the rest from
//! [`Notify`].
//!
//! ## Nothing here knows about Tauri
//!
//! Events are how the shell observes the queue, but a queue that can only be
//! tested through a running app is a queue nobody tests. So the queue talks to
//! a [`Notify`] trait, the shell implements it over `app.emit`, and
//! `tests/queue.rs` implements it with a `Vec`. The fake task and the fake
//! notifier live in `tests/` rather than behind `#[cfg(test)]` for the same
//! reason `wobu-llm`'s fake provider does: a fake that can reach private items
//! proves the trait is implementable *here* rather than implementable at all,
//! and the tasks that matter (#37, #40) are written from outside.
//!
//! ## Three things this exists to get right
//!
//! **Cancellation is a stop, not an unsubscribe.** A job cancelled before it
//! starts never runs; a job cancelled while running is asked to stop through
//! the [`Cancel`] token it was given, and if it has not stopped within
//! [`Config::cancel_grace`] its future is aborted outright. Dropping the result
//! and letting the provider finish would leave the user paying for a request
//! they stopped wanting, which is the failure mode the whole design is aimed at.
//!
//! **A retry that costs money is never silent.** The queue cannot price a call
//! and must not pretend it can. What it can know is whether the attempt that
//! just failed was billed, because the task tells it ([`Billed`]), and the rule
//! follows from that: an unbilled failure — a 429, a refused connection — is
//! retried on the queue's own initiative, and a *billed* failure is not.
//! "The model produced garbage, try again" is a decision to spend the user's
//! money again, so by default it is held and handed up ([`Verdict::Hold`]).
//! A submitter that wants those retries opts in with
//! [`RetryPolicy::paid_attempts`], and every one of them is announced on
//! `job:retry` *before* the wait — so the spending is visible while it is still
//! about to happen rather than after it has.
//!
//! **One bad job does not take the queue with it.** A task that panics is
//! caught, reported as an ordinary failure, and its slot released. See
//! [`queue`] for how, and for the one build setting that would defeat it.
//!
//! ## What is deliberately not here
//!
//! No priorities: admission is FIFO and a job that backs off re-enters at the
//! tail, so nothing can starve. A priority queue with no user-visible priority
//! is a starvation bug waiting for someone to add one. No per-provider
//! concurrency caps either — the cap is global and adjustable
//! ([`Queue::set_concurrency`]), which is what Hunyuan3D's three-concurrent-Pro
//! limit needs from a desktop app with one provider selected at a time.

pub mod event;
pub mod job;
pub mod queue;
pub mod retry;

pub use event::{
    DoneEvent, Event, FailedEvent, Notify, Preview, PreviewEvent, Progress, ProgressEvent,
    RetryEvent, Silent, events,
};
pub use job::{JobContext, JobId, JobKind, JobSnapshot, JobState, Outcome, QueueSnapshot, Task};
pub use queue::{Config, Queue};
pub use retry::{Attempts, Billed, Failure, RetryPolicy, Verdict, decide};

/// The cancellation token, re-exported from `wobu-llm` rather than defined
/// again here.
///
/// `wobu_llm::Cancel` was written with this crate in mind — its own header says
/// the job queue "will hand one of these to every provider call" — and that is
/// exactly what [`JobContext::cancel`] does. A third copy of the same flag would
/// buy nothing and cost every task a bridge: a spawned mirror whose only job is
/// to set one token when another is set, one per job, with its own way of
/// leaking.
///
/// It is the reason this crate depends on `wobu-llm` at all. The direction is
/// right — the queue drives the providers, not the other way round — and the
/// edge cannot become a cycle, because a provider adapter reaching for the queue
/// would be an adapter deciding its own retries, which #33 already put here.
pub use wobu_llm::Cancel;
