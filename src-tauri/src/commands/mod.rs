//! The command surface the webview calls.
//!
//! Everything here is glue: argument shapes, the lock, and the error mapping.
//! There is no domain logic in this file and there should never be any — the
//! rules live in `wobu-core`, the persistence in `wobu-store`. If a command
//! grows a branch that decides something about the world, it is in the wrong
//! crate.
//!
//! Argument names are snake_case; Tauri v2 matches them against the camelCase
//! keys `src/lib/api.ts` sends.

use tauri::State;
// Aliased because the command below has to *be* called `kind_registry` —
// Tauri v2 derives the invoke name from the function name, with no rename.
use wobu_core::Node;
use wobu_jobs::{JobId, QueueSnapshot};
use wobu_store::{Project, SaveOutcome};

use crate::error::{Code, CommandResult, WobuError};
use crate::state::Jobs;

// One module per command group. Each is `pub` because
// `tauri::generate_handler!` resolves a command through its real module path.
pub mod assets;
pub mod credentials;
pub mod diagnostics;
pub mod generations;
pub mod influence;
/// Reviewing a turnaround and reconstructing a mesh from it (#110). The one
/// command group with a job, a provider adapter and a task of its own;
/// `generations::mesh_concepts` is only the reading half of it.
pub mod mesh;
pub mod nodes;
pub mod project;
/// The one reader of `project.json`'s `providers` map, shared by images, text,
/// 3D and the status bar.
pub mod providers;
pub mod settings;
pub mod style;
pub mod thumbs;

#[cfg(test)]
mod bridge;

/// How many thumbnails one IPC may ask for, assets or nodes alike.
///
/// A bound rather than a page size: the caller sends the window it is about to
/// draw, and a window that large is a caller that has stopped virtualizing.
const THUMB_BATCH_LIMIT: usize = 100;

/// A project-relative path as the absolute one `convertFileSrc` needs.
///
/// The join happens here rather than in the webview on purpose. Every path Wobu
/// stores is `/`-separated and project-relative, because the same share is
/// `/Volumes/art/…` on one machine and `Z:\art\…` on another — and a frontend
/// doing that join would be a second place that has to know which. `None` when
/// the path does not resolve to a file, so a tile is never pointed at a URL that
/// will 404.
fn absolute(project: &Project, rel: &str) -> Option<String> {
    let path = wobu_store::paths::from_rel_string(project.root(), rel);
    path.is_file().then(|| path.to_string_lossy().into_owned())
}

/// Run `f` on a blocking thread, turning a lost thread into a command error.
///
/// Shared by the thumbnail commands and `project_create` so that "the pool went
/// away" reads the same from all of them; `project_open` predates it and spells
/// its own out inline.
async fn blocking<T: Send + 'static>(
    lost: &'static str,
    f: impl FnOnce() -> T + Send + 'static,
) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| WobuError::new(Code::Internal, lost).with_detail(e.to_string()))
}

/// The node that was written, or the conflict that stopped it.
///
/// Shared by the four commands above so that attaching a reference reports a
/// lost save race in exactly the shape `node_upsert` does — the frontend has
/// one `write.conflict` handler and it must not need a second.
fn saved(outcome: SaveOutcome) -> CommandResult<Node> {
    match outcome {
        SaveOutcome::Saved(node) => Ok(*node),
        SaveOutcome::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    }
}

/* ── influence ────────────────────────────────────────────────────────────── */

/// Stop a job. `false` when there is no such job, or it had already finished.
///
/// Neither of those is an error, deliberately: the user can press Stop at the
/// instant a job ends, and a webview that reloaded may still hold an id for
/// something long gone. A *malformed* id is different — that one can never
/// work, so it is reported rather than swallowed.
///
/// Returns as soon as the queue has been told, not when the work has stopped.
/// A job that had not started is over immediately; one in flight is aborted
/// within the grace it gets to report what it was charged. Waiting for either
/// would be this command blocking the webview, which is the whole thing the
/// queue exists to prevent.
#[tauri::command]
pub fn job_cancel(jobs: State<'_, Jobs>, job_id: String) -> CommandResult<bool> {
    let id = JobId::parse(&job_id).ok_or_else(|| {
        WobuError::new(Code::Internal, "That is not a job id.").with_detail(job_id)
    })?;
    Ok(jobs.cancel(id))
}

/// The queue as it stands.
///
/// `job:state` is the live signal; this covers the one case events cannot — a
/// webview that reloaded while three generations were in flight, which is the
/// same argument `share_offline` makes.
#[tauri::command]
pub fn job_list(jobs: State<'_, Jobs>) -> QueueSnapshot {
    jobs.snapshot()
}
