//! The one piece of mutable process state: the open project, and the watcher
//! keeping it honest about what is on disk.
//!
//! ## Why a `Mutex` and not an `RwLock`
//!
//! It would be nice to let `node_list` and `node_get` run concurrently — they
//! only need `&Project`. They cannot. `Project` owns an `Index`, which owns a
//! `rusqlite::Connection`, which is `Send` but deliberately not `Sync`. An
//! `RwLock<Project>` is only `Sync` when `Project: Send + Sync`, so it will not
//! even compile as Tauri managed state; a `Mutex<Project>` needs `Send` alone.
//!
//! That is not as costly as it looks. Reads go to the local SQLite index, not
//! the filesystem, so the critical section is microseconds. The write path is
//! the one to watch, and it is bounded by a single guarded `write` + `rename`
//! of one Markdown file. What must *not* happen is holding the lock across
//! anything slow — an LLM call, a network stat, a dialog — and the rule that
//! keeps that true is that every helper below takes the lock, does one thing,
//! and gives it back.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};
use wobu_store::{Project, Watcher};

use crate::error::{CommandResult, WobuError};

/// Emitted whenever the project folder turns out to differ from what we last
/// indexed. `src/lib/queries.ts` listens for this and invalidates the world.
pub const WORLD_CHANGED: &str = "world:changed";

pub struct Open {
    pub project: Project,
    /// Dropped on close, which stops the watch thread. `None` only in the
    /// window between opening a project and the watcher starting, and if the
    /// platform refused to give us one at all.
    pub watcher: Option<Watcher>,
}

/// Cloneable handle to the slot, so the watcher callback can reach it without
/// borrowing from a Tauri `State<'_, _>`.
#[derive(Default)]
pub struct AppState {
    slot: Arc<Mutex<Option<Open>>>,
}

impl AppState {
    /// Run `f` against the open project, or fail with `no_project_open`.
    pub fn with<T>(&self, f: impl FnOnce(&mut Project) -> CommandResult<T>) -> CommandResult<T> {
        let mut guard = self.slot.lock();
        let open = guard.as_mut().ok_or_else(WobuError::no_project_open)?;
        f(&mut open.project)
    }

    /// Like [`with`](Self::with), but for callers that are fine with there
    /// being nothing open.
    pub fn peek<T>(&self, f: impl FnOnce(Option<&Project>) -> T) -> T {
        let guard = self.slot.lock();
        f(guard.as_ref().map(|o| &o.project))
    }

    /// Install a freshly opened project, replacing (and closing) whatever was
    /// open before, then start watching its folder.
    pub fn install(&self, app: &AppHandle, project: Project) {
        let root = project.root().to_path_buf();

        // Close the previous project first, and crucially do the *drop* with
        // the lock released — see `close`.
        self.close();
        *self.slot.lock() = Some(Open { project, watcher: None });

        let watcher = self.start_watcher(app, &root);
        if let Some(open) = self.slot.lock().as_mut() {
            open.watcher = watcher;
        }
    }

    /// Close the open project, if any. Idempotent.
    pub fn close(&self) {
        // Written as take-then-drop rather than `*self.slot.lock() = None` so
        // that the `Project` — and with it the SQLite index handle — is dropped
        // with the guard already released. The watch thread may be inside the
        // callback below at this instant; taking the value out under the lock
        // means we wait for that reconcile to finish, and dropping it outside
        // means the thread's next iteration finds `None` and no-ops rather than
        // queueing behind us.
        let taken = self.slot.lock().take();
        drop(taken);
    }

    fn start_watcher(&self, app: &AppHandle, root: &Path) -> Option<Watcher> {
        let slot = Arc::clone(&self.slot);
        let app = app.clone();

        let result = Watcher::start(root, move || {
            // Reconcile under the lock; emit outside it. `reconcile` reports
            // whether anything actually moved, and we only wake the frontend
            // when it did — otherwise every save the app itself makes would
            // bounce straight back as a cache invalidation and refetch.
            let changed = match slot.lock().as_mut() {
                Some(open) => open.project.reconcile().unwrap_or(false),
                // Closed between the event firing and us getting the lock.
                None => return,
            };
            if changed {
                let _ = app.emit(WORLD_CHANGED, ());
            }
        });

        match result {
            Ok(w) => Some(w),
            // Not fatal: without a watcher the app still reads and writes, the
            // view is just no longer live to edits made outside it.
            Err(e) => {
                eprintln!("wobu: could not watch {}: {e}", root.display());
                None
            }
        }
    }
}
