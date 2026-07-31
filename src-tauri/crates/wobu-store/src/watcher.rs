//! Change detection, which is not the same problem on a local disk as on a share.
//!
//! `inotify`/`FSEvents` do **not** see writes made by other hosts over NFS or
//! SMB, so relying on `notify` alone means a collaborator's changes stay
//! invisible until restart. The strategy is therefore picked from the project
//! path. See `docs/07-file-shares.md`.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher as _};

use crate::error::{Error, Result};

const LOCAL_DEBOUNCE: Duration = Duration::from_millis(400);

/// How often a share is listed while something is actually happening on it.
pub const POLL_BUSY: Duration = Duration::from_secs(5);
/// And once nothing has changed for a while.
///
/// Polling is not free on a share: each round is a directory listing over the
/// wire, and with several people connected those add up into the latency
/// everyone else is waiting on. Five seconds is right when a collaborator is
/// typing and worth backing away from when nobody is — a world sitting
/// untouched overnight should not be generating traffic every five seconds.
pub const POLL_IDLE: Duration = Duration::from_secs(15);
/// Quiet rounds before backing off. Two so a single pause between saves does
/// not immediately triple the latency of noticing the next one.
const IDLE_ROUNDS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// `notify`, debounced. Near-instant.
    Local,
    /// Directory-listing poll. The only thing that observes another host's writes.
    Poll,
}

/// Signals that the project folder *may* have changed. The callback is a nudge,
/// not a description — the receiver calls `Project::reconcile`, which does the
/// cheap `(mtime, size)` comparison and decides whether anything really moved.
pub struct Watcher {
    strategy: Strategy,
    stop: Arc<AtomicBool>,
    _inner: Option<notify::RecommendedWatcher>,
}

impl Watcher {
    /// `on_change` returns whether the reconcile it ran found anything. The
    /// event watcher ignores the answer — it is told what changed by the OS —
    /// but the poller uses it to decide how hard to keep looking.
    pub fn start(
        root: &Path,
        on_change: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Result<Watcher> {
        let strategy = if crate::paths::is_network_path(root) { Strategy::Poll } else { Strategy::Local };
        match strategy {
            Strategy::Local => Watcher::start_local(root, on_change),
            Strategy::Poll => Ok(Watcher::start_poll(on_change)),
        }
    }

    pub fn strategy(&self) -> Strategy {
        self.strategy
    }

    fn start_local(
        root: &Path,
        on_change: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Result<Watcher> {
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            // The event is discarded deliberately: the receiver reconciles
            // against the index rather than trusting a path out of the OS,
            // which is the only thing that behaves the same on every platform.
            if res.is_ok() {
                let _ = tx.send(());
            }
        })
        .map_err(|e| Error::io(root, std::io::Error::other(e)))?;

        let nodes = root.join("nodes");
        crate::paths::ensure_dir(&nodes)?;
        watcher
            .watch(&nodes, RecursiveMode::Recursive)
            .map_err(|e| Error::io(&nodes, std::io::Error::other(e)))?;

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                // Block until something happens, then swallow the burst: a
                // single save produces several events, and reconciling once per
                // event would re-scan the folder repeatedly.
                if rx.recv_timeout(Duration::from_millis(500)).is_err() {
                    continue;
                }
                while rx.recv_timeout(LOCAL_DEBOUNCE).is_ok() {}
                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                let _ = on_change();
            }
        });

        Ok(Watcher { strategy: Strategy::Local, stop, _inner: Some(watcher) })
    }

    /// The poller, which backs off when the world is quiet.
    ///
    /// `on_change` reports whether the reconcile it triggered actually found
    /// anything. That answer is the only signal available about whether anyone
    /// else is working in this world right now — there is no presence protocol
    /// yet (#16) — and it is enough: poll briskly while something is moving,
    /// and stretch out once it stops.
    fn start_poll(on_change: impl Fn() -> bool + Send + Sync + 'static) -> Watcher {
        Watcher::start_poll_every(POLL_BUSY, POLL_IDLE, on_change)
    }

    /// Intervals injected so the backoff can be tested in under a second
    /// instead of over half a minute.
    fn start_poll_every(
        busy: Duration,
        idle: Duration,
        on_change: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Watcher {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        std::thread::spawn(move || {
            let mut quiet_rounds: u32 = 0;
            while !thread_stop.load(Ordering::Relaxed) {
                let wait = if quiet_rounds >= IDLE_ROUNDS { idle } else { busy };

                // Sliced so a close does not wait out the full interval — at
                // the idle rate that would be fifteen seconds of a window that
                // has already gone.
                let slice = Duration::from_millis(50);
                let mut waited = Duration::ZERO;
                while waited < wait {
                    if thread_stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(slice);
                    waited += slice;
                }
                if thread_stop.load(Ordering::Relaxed) {
                    return;
                }

                if on_change() {
                    // Someone is working. Drop straight back to the fast rate
                    // rather than easing down, so the *second* change in a
                    // conversation is noticed as quickly as the first.
                    quiet_rounds = 0;
                } else {
                    quiet_rounds = quiet_rounds.saturating_add(1);
                }
            }
        });
        Watcher { strategy: Strategy::Poll, stop, _inner: None }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_local_project_uses_the_event_watcher() {
        let dir = tempfile::tempdir().unwrap();
        let watcher = Watcher::start(dir.path(), || false).unwrap();
        assert_eq!(watcher.strategy(), Strategy::Local);
    }

    #[test]
    fn a_local_edit_fires_the_callback_once_per_burst() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = dir.path().join("nodes/species");
        std::fs::create_dir_all(&nodes).unwrap();

        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let _watcher = Watcher::start(dir.path(), move || {
            counter.fetch_add(1, Ordering::Relaxed);
            true
        })
        .unwrap();

        // Several writes in quick succession, as one save produces.
        for i in 0..5 {
            std::fs::write(nodes.join("vashk.md"), format!("v{i}")).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(1200));

        let observed = hits.load(Ordering::Relaxed);
        assert!(observed >= 1, "the edit should have been noticed");
        assert!(observed <= 3, "debounce should collapse the burst, saw {observed}");
    }

    #[test]
    fn the_poll_backs_off_once_nothing_is_changing() {
        // Scaled down by 50× so this runs in under a second; the ratio is what
        // is being tested, not the wall-clock numbers.
        let busy = Duration::from_millis(100);
        let idle = Duration::from_millis(300);

        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let _watcher = Watcher::start_poll_every(busy, idle, move || {
            counter.fetch_add(1, Ordering::Relaxed);
            false // nothing ever changes: the world is idle
        });

        std::thread::sleep(Duration::from_millis(1000));
        let observed = hits.load(Ordering::Relaxed);

        // Flat out at the busy rate that window would be ~10 polls. With the
        // backoff it is 2 fast rounds then ~2-3 slow ones.
        assert!(observed >= 3, "the poller should still be running, saw {observed}");
        assert!(observed <= 7, "expected a backoff, but saw {observed} polls");
    }

    #[test]
    fn a_change_puts_the_poll_straight_back_to_the_fast_rate() {
        // The failure this guards against is a watcher that backs off and stays
        // backed off, so the first edit of the morning takes 15s to appear and
        // every one after it does too.
        let busy = Duration::from_millis(100);
        let idle = Duration::from_millis(300);

        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let _watcher = Watcher::start_poll_every(busy, idle, move || {
            counter.fetch_add(1, Ordering::Relaxed);
            true // somebody is working the whole time
        });

        std::thread::sleep(Duration::from_millis(1000));
        let observed = hits.load(Ordering::Relaxed);
        assert!(observed >= 7, "a busy world should stay on the fast poll, saw {observed}");
    }

    #[test]
    fn dropping_the_watcher_stops_it() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = dir.path().join("nodes");
        std::fs::create_dir_all(&nodes).unwrap();

        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        {
            let _watcher = Watcher::start(dir.path(), move || {
                counter.fetch_add(1, Ordering::Relaxed);
                true
            })
            .unwrap();
        }
        std::thread::sleep(Duration::from_millis(200));
        let settled = hits.load(Ordering::Relaxed);

        std::fs::write(nodes.join("late.md"), "after drop").unwrap();
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(hits.load(Ordering::Relaxed), settled, "no callbacks after drop");
    }
}
