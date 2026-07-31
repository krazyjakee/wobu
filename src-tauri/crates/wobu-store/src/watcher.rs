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
const NETWORK_POLL: Duration = Duration::from_secs(5);

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
    pub fn start(
        root: &Path,
        on_change: impl Fn() + Send + Sync + 'static,
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

    fn start_local(root: &Path, on_change: impl Fn() + Send + Sync + 'static) -> Result<Watcher> {
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
                on_change();
            }
        });

        Ok(Watcher { strategy: Strategy::Local, stop, _inner: Some(watcher) })
    }

    fn start_poll(on_change: impl Fn() + Send + Sync + 'static) -> Watcher {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                // Sliced so a close does not wait out the full interval.
                for _ in 0..(NETWORK_POLL.as_millis() / 250) {
                    if thread_stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
                if thread_stop.load(Ordering::Relaxed) {
                    return;
                }
                on_change();
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
        let watcher = Watcher::start(dir.path(), || {}).unwrap();
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
    fn dropping_the_watcher_stops_it() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = dir.path().join("nodes");
        std::fs::create_dir_all(&nodes).unwrap();

        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        {
            let _watcher = Watcher::start(dir.path(), move || {
                counter.fetch_add(1, Ordering::Relaxed);
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
