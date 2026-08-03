//! What happens when Wobu is asked to stop.
//!
//! The written policy is `docs/15-exit-policy.md`; this module is the half of
//! it the process can enforce on its own, and it is deliberately one function
//! so that every exit path runs the same steps in the same order. Before it
//! existed, `RunEvent::Exit` wound down sync and nothing else, `force_quit`
//! closed the project and called `exit(0)`, and a SIGTERM did neither — three
//! exits with three different ideas of what had to happen first.
//!
//! ## The order, and why it is that order
//!
//! 1. **Close the job queue.** Cancelling first is what keeps the app from
//!    orphaning paid work: an adapter told to stop unwinds far enough to report
//!    what it was billed and to tell the provider, and ComfyUI gets its
//!    `/interrupt` rather than a prompt left running on the user's own GPU.
//!    Closing rather than merely cancelling also means a command racing the
//!    teardown cannot start a fresh paid call on the way out.
//! 2. **Wait for the queue, briefly.** [`JOB_BUDGET`] is a backstop, not the
//!    mechanism — `wobu_jobs::Config::cancel_grace` already bounds each job, so
//!    reaching this budget means something below it is wedged. A quit that
//!    hangs is a worse outcome than a job that is cut off, which is the same
//!    trade `SyncManager::shutdown` makes.
//! 3. **Close the project.** After the jobs, because a generation writing its
//!    result opens the folder by path and does not go through `AppState` — so
//!    closing first would not have stopped it, it would only have taken away
//!    the watcher that notices what it wrote. Closing here drops the presence
//!    entry, so collaborators see us leave now rather than when the heartbeat
//!    expires, and drops the SQLite handle with a clean WAL.
//! 4. **Stop sync.** Last, because a round holds the project mutex and step 3
//!    is where that mutex is contended. `SyncState::stop` carries its own
//!    deadline; see `src/sync/manager.rs`.
//!
//! ## What this cannot do
//!
//! Nothing here can flush the *renderer's* autosave debounce: by the time a
//! signal or a `RunEvent::Exit` arrives, the webview has already been told to
//! go. That gate lives in `src/hooks/useSafeWindowClose.ts` and runs before any
//! of this. A `SIGKILL`, a power cut or a webview crash reaches none of it, and
//! the answer there is not a handler but the storage design: node Markdown is
//! staged, fsynced and renamed, and the SQLite index is a rebuildable cache of
//! a folder that is canonical.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::diag;
use crate::state::{AppState, Jobs};
use crate::sync::SyncState;

/// How long a closed queue is given to wind down before the process leaves.
///
/// Three seconds, which is `cancel_grace` (two) plus room for the last job to
/// report itself. Long enough that a cancelled generation gets to say what it
/// cost, short enough that nobody watching the window disappear thinks the app
/// has hung.
pub const JOB_BUDGET: Duration = Duration::from_secs(3);

/// Runs once per process. A quit reaches [`wind_down`] twice by design — the
/// signal handler tears down and *then* asks the app to exit, which raises
/// `RunEvent::Exit` — and the second pass must be free rather than a second
/// wait on every budget below.
static WOUND_DOWN: AtomicBool = AtomicBool::new(false);

/// What one wind-down did. Returned rather than logged in place so that the
/// ordering can be tested without a Tauri app; see [`wind_down_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// Unfinished jobs asked to stop.
    pub cancelled: usize,
    /// Whether the queue emptied inside [`JOB_BUDGET`].
    pub quiesced: bool,
    /// `false` when a previous call had already done all of this.
    pub ran: bool,
}

/// Wind the process down. Idempotent, and safe to call from any thread that is
/// not inside the async runtime.
///
/// "Not inside the runtime" is a real constraint rather than a caution: both
/// this and `SyncState::stop` block on the runtime, which panics if the calling
/// thread is already one of its workers. The two callers are `RunEvent::Exit`
/// (the main thread) and the signal handler (a thread of its own, for exactly
/// this reason).
pub fn wind_down(app: &AppHandle) -> Report {
    let jobs = app.try_state::<Jobs>();
    let state = app.try_state::<AppState>();
    let sync = app.try_state::<SyncState>();

    let report = wind_down_with(
        || jobs.as_ref().map(|jobs| jobs.queue().close()).unwrap_or(0),
        |budget| {
            jobs.as_ref().is_none_or(|jobs| {
                let queue = jobs.queue().clone();
                tauri::async_runtime::block_on(async move { queue.quiesce(budget).await })
            })
        },
        || {
            if let Some(state) = state.as_ref() {
                state.close();
            }
        },
        || {
            if let Some(sync) = sync.as_ref() {
                sync.stop();
            }
        },
    );

    if report.ran {
        diag::info(format!(
            "wobu: wound down ({} job(s) stopped, queue {})",
            report.cancelled,
            if report.quiesced { "empty" } else { "still busy at the budget" }
        ));
    }
    report
}

/// The ordered teardown with each stage passed in, so the order itself is
/// testable without a window, a runtime or a project on disk.
///
/// The argument this exists to protect is the module header's: jobs before the
/// project, the project before sync. Every one of those orderings was chosen
/// against a specific failure, and none of them is visible in the call site.
pub fn wind_down_with(
    close_jobs: impl FnOnce() -> usize,
    quiesce: impl FnOnce(Duration) -> bool,
    close_project: impl FnOnce(),
    stop_sync: impl FnOnce(),
) -> Report {
    if WOUND_DOWN.swap(true, Ordering::SeqCst) {
        return Report { cancelled: 0, quiesced: true, ran: false };
    }
    let cancelled = close_jobs();
    let quiesced = quiesce(JOB_BUDGET);
    close_project();
    stop_sync();
    Report { cancelled, quiesced, ran: true }
}

/// Catch the signals a logout, a `systemctl` stop or a Ctrl-C in a dev shell
/// sends, and take the ordinary exit path instead of dying where we stand.
///
/// This does not save an unsaved edit — the renderer is not asked anything and
/// there is no round trip to ask it in — and it is not pretending to. What it
/// buys is the difference between a session that ends and a session that stops
/// answering: the queue is cancelled rather than orphaned, our presence entry
/// leaves the share rather than lingering until it expires, and peers are told
/// the endpoint is closing rather than finding a severed socket.
///
/// Unix only. Windows sends `WM_CLOSE` for a logout, which arrives as an
/// ordinary `CloseRequested` and therefore already runs the full gate including
/// the renderer's; the console control events that do not are all
/// non-negotiable kills.
#[cfg(unix)]
pub fn install_signal_handlers(app: &AppHandle) {
    use tokio::signal::unix::{SignalKind, signal};

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(error) => {
                diag::error(format!("wobu: could not listen for SIGTERM: {error}"));
                return;
            }
        };
        let mut interrupt = match signal(SignalKind::interrupt()) {
            Ok(stream) => stream,
            Err(error) => {
                diag::error(format!("wobu: could not listen for SIGINT: {error}"));
                return;
            }
        };
        let mut hangup = match signal(SignalKind::hangup()) {
            Ok(stream) => stream,
            Err(error) => {
                diag::error(format!("wobu: could not listen for SIGHUP: {error}"));
                return;
            }
        };

        let name = tokio::select! {
            _ = terminate.recv() => "SIGTERM",
            _ = interrupt.recv() => "SIGINT",
            _ = hangup.recv() => "SIGHUP",
        };
        diag::info(format!("wobu: {name} received; winding down"));

        // A thread rather than this task: `wind_down` blocks on the runtime and
        // doing that from inside it panics. The exit request is made from there
        // too, so the teardown is complete before the event loop is told to end
        // — `RunEvent::Exit` then finds the work already done.
        std::thread::spawn(move || {
            wind_down(&app);
            app.exit(0);
        });
    });
}

#[cfg(not(unix))]
pub fn install_signal_handlers(_app: &AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn reset() {
        WOUND_DOWN.store(false, Ordering::SeqCst);
    }

    #[derive(Default)]
    struct Steps(Arc<Mutex<Vec<&'static str>>>);

    impl Steps {
        fn note(&self, step: &'static str) {
            self.0.lock().expect("steps").push(step);
        }

        fn taken(&self) -> Vec<&'static str> {
            self.0.lock().expect("steps").clone()
        }
    }

    /// The whole point of the module, asserted: every exit runs the same four
    /// steps in the same order. Jobs first, because cancelling is what stops a
    /// paid call being orphaned; sync last, because closing the project is
    /// where its mutex is contended.
    #[test]
    fn the_teardown_order_is_jobs_then_the_project_then_sync() {
        // These tests share one process-wide latch, so they run under one
        // `#[test]` rather than racing each other for it.
        reset();
        let steps = Steps::default();
        let report = wind_down_with(
            || {
                steps.note("close jobs");
                2
            },
            |budget| {
                steps.note("quiesce");
                assert_eq!(budget, JOB_BUDGET);
                true
            },
            || steps.note("close project"),
            || steps.note("stop sync"),
        );

        assert_eq!(steps.taken(), ["close jobs", "quiesce", "close project", "stop sync"]);
        assert_eq!(report, Report { cancelled: 2, quiesced: true, ran: true });

        // The second pass is free. A signal handler tears down and then asks
        // the app to exit, which raises `RunEvent::Exit` — so a wind-down that
        // was not idempotent would wait out every budget twice, with the window
        // already gone and nothing on screen to explain the delay.
        let again = Steps::default();
        let second = wind_down_with(
            || {
                again.note("close jobs");
                1
            },
            |_| {
                again.note("quiesce");
                true
            },
            || again.note("close project"),
            || again.note("stop sync"),
        );
        assert!(again.taken().is_empty(), "a second wind-down repeated the teardown");
        assert!(!second.ran);

        // A queue that could not empty inside its budget is reported rather
        // than waited on: the remaining steps still run and the process still
        // leaves.
        reset();
        let busy = Steps::default();
        let third = wind_down_with(
            || 1,
            |_| {
                busy.note("quiesce");
                false
            },
            || busy.note("close project"),
            || busy.note("stop sync"),
        );
        assert_eq!(busy.taken(), ["quiesce", "close project", "stop sync"]);
        assert!(third.ran && !third.quiesced);
        reset();
    }
}
