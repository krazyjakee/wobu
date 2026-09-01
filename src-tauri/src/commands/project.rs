//! Opening, closing and describing a project folder.
//!
//! Opening is the one command that may take a visible amount of time — it
//! indexes the folder — so it reports progress and can be cancelled. Everything
//! after it assumes an adopted project, which is what `adopt` establishes.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Emitter, Manager, State};
// Aliased because the command below has to *be* called `kind_registry` —
// Tauri v2 derives the invoke name from the function name, with no rename.
use wobu_core::kind_registry as registry;
use wobu_core::{Id, KindDef, NodeKind, Preset};
use wobu_store::{CorruptFile, Project, ProjectSummary, WikiExport, recent};

use super::assets::AssetTransfers;
use super::blocking;
use crate::diag;
use crate::enhance::Pending;
use crate::error::{Code, CommandResult, WobuError};
use crate::state::{AppState, WORLD_CHANGED};
use crate::sync::SyncState;

/// Static, and the frontend caches it forever (`staleTime: Infinity`).
#[tauri::command]
pub fn kind_registry() -> Vec<KindDef> {
    registry().to_vec()
}

/// Output presets offered for one kind. Static registry data, like kinds, but
/// filtered here so the Inspector never has to reproduce applicability rules.
#[tauri::command]
pub fn preset_list(kind: NodeKind) -> Vec<&'static Preset> {
    wobu_core::presets_for(kind)
}

/* ── project ──────────────────────────────────────────────────────────────── */

/// Create a project folder and adopt it.
///
/// `async` only so it can wait for [`SyncState::wait_until_named`] — see there
/// for why nothing may adopt a folder before this installation has its alias.
/// The creation itself is filesystem work and stays off the runtime.
#[tauri::command]
pub async fn project_create(
    app: AppHandle,
    state: State<'_, AppState>,
    sync: State<'_, SyncState>,
    parent_dir: String,
    name: String,
) -> CommandResult<ProjectSummary> {
    sync.wait_until_named().await;
    let root = PathBuf::from(parent_dir);
    let project =
        blocking("The create thread stopped unexpectedly.", move || Project::create(&root, &name))
            .await??;
    Ok(adopt(&app, &state, project))
}

/// Emitted while a first open is scanning. Payload is `ScanProgress`.
pub const OPEN_PROGRESS: &str = "project:open-progress";

/// Opening a project, which on a NAS is the one operation that can take
/// minutes.
///
/// `async` so the webview keeps painting, and the scan itself runs on a
/// blocking thread because it is filesystem-bound rather than await-bound. A
/// stalled mount must never present as a frozen app, which is the whole reason
/// this is not the three-line synchronous command it used to be.
///
/// Progress is emitted rather than returned: a return value arrives once, at
/// the end, which is exactly when the user no longer needs it.
#[tauri::command]
pub async fn project_open(
    app: AppHandle,
    state: State<'_, AppState>,
    sync: State<'_, SyncState>,
    path: String,
) -> CommandResult<ProjectSummary> {
    // Before `begin_open`, so a scan that is about to write a conflict sibling
    // cannot start under a name this installation is not going to keep. See
    // `SyncState::wait_until_named`.
    sync.wait_until_named().await;
    let cancel = state.begin_open();
    let emitter = app.clone();
    let root = PathBuf::from(path);

    let opened = tauri::async_runtime::spawn_blocking(move || {
        let mut last = 0u8;
        Project::open_with(&root, &cancel, &mut |p| {
            // Throttled to whole percentage points. A world of 4000 files would
            // otherwise put 4000 events through the bridge, and repainting the
            // same number is work taken from the scan itself.
            let pct = p.percent();
            if pct != last {
                last = pct;
                let _ = emitter.emit(OPEN_PROGRESS, p);
            }
        })
    })
    .await
    .map_err(|e| {
        WobuError::new(Code::Internal, "The scan thread stopped unexpectedly.")
            .with_detail(e.to_string())
    })?;

    state.finish_open();
    Ok(adopt(&app, &state, opened?))
}

/// Stop a scan in progress.
///
/// A no-op when nothing is open — the user can press this at the moment the
/// scan finishes, and that race must not be an error.
#[tauri::command]
pub fn project_open_cancel(state: State<'_, AppState>) {
    state.cancel_open();
}

/// Close the open project.
///
/// Any enhanced description still waiting to be accepted goes with it. It could
/// not be written into another project anyway — `enhance_accept` checks which
/// world it came from — but a description of somebody's world should not still
/// be sitting in this process after they have shut it.
#[tauri::command]
pub fn project_close(
    state: State<'_, AppState>,
    pending: State<'_, Pending>,
    asset_transfers: State<'_, AssetTransfers>,
) -> CommandResult<()> {
    asset_transfers.clear();
    state.close();
    pending.clear();
    Ok(())
}

#[tauri::command]
pub fn project_current(state: State<'_, AppState>) -> Option<ProjectSummary> {
    state.peek(|p| p.map(Project::summary))
}

#[tauri::command]
pub fn project_recent() -> Vec<ProjectSummary> {
    recent::list_summaries()
}

/// Forget one launcher entry. The project folder and everything inside it are
/// deliberately outside this command's scope.
#[tauri::command]
pub fn project_recent_forget(id: Id) -> CommandResult<()> {
    recent::forget(id)?;
    Ok(())
}

/// Whether the open project's folder is currently unreachable.
///
/// The `share:offline` / `share:online` events are the live signal; this is
/// for the one case events cannot cover — a webview that reloaded while
/// disconnected and so missed the event that would have raised the banner.
#[tauri::command]
pub fn share_offline(state: State<'_, AppState>) -> bool {
    state.is_offline()
}

/// Quit despite the share being away, having been told what that costs.
///
/// The window's close handler refuses the first attempt while offline; this is
/// the only way past it, and it exists so that the refusal is a warning rather
/// than a trap the user cannot get out of.
#[tauri::command]
pub fn force_quit(
    app: AppHandle,
    state: State<'_, AppState>,
    asset_transfers: State<'_, AssetTransfers>,
) {
    // Drop the project first so the watcher and reconnect threads stop before
    // the process does, rather than being killed mid-reconcile.
    asset_transfers.clear();
    state.close();
    app.exit(0);
}

/// Take ownership of a just-opened project: remember it in the recents list,
/// then hand it to the state (which starts the watcher).
fn adopt(app: &AppHandle, state: &AppState, project: Project) -> ProjectSummary {
    let summary = project.summary();
    // A recents file we cannot write is an annoyance, not a failure to open —
    // the project itself is already fine.
    if let Err(e) = recent::record(&summary) {
        diag::error(format!("could not record recent project: {e}"));
    }
    // The path is the single most useful line in a bug report: it says whether
    // the world was on a share, and `redact::scrub` leaves it intact because a
    // filesystem path is not a credential.
    diag::info(format!("opened project {} at {}", summary.id, summary.path));
    allow_assets(app, project.root());
    state.install(app, project);
    summary
}

/// Let the webview read this project's images off disk, and nothing else.
///
/// The asset protocol ships with an empty scope in `tauri.conf.json`; this is
/// the only thing that ever widens it, and it widens it to one directory of one
/// project the user has just picked. That is the whole security story for #25 —
/// a grid of a thousand tiles must not base64 a thousand files through the IPC
/// bridge, so the webview needs to load them by path, and the price of that is
/// saying exactly which paths.
///
/// Scoped to `assets/` rather than to the project root, which is narrower than
/// the issue asks for and deliberately so: the only things the webview ever
/// loads as *files* are thumbnails and originals, and both are under there.
/// Everything else in the folder — `nodes/**`, `project.json`, `.wobu/` —
/// already reaches the frontend through commands that decide what it may see,
/// and a scope covering the root would quietly also cover whatever lands in the
/// folder next.
///
/// **The scope only ever grows, within one run.** Tauri's filesystem scope has
/// no way to withdraw an allowance — `forbid_directory` is permanent, so using
/// it on close would make reopening the same project impossible — which means a
/// session that opens three worlds ends up able to read the assets of all three.
/// That is worth stating rather than hiding, and it is a small thing: they are
/// three folders this user opened themselves, in this session, and nothing
/// survives the process exiting.
fn allow_assets(app: &AppHandle, root: &Path) {
    let assets = root.join("assets");
    if let Err(e) = app.asset_protocol_scope().allow_directory(&assets, true) {
        // Not fatal, and not silent. Everything else about the project works;
        // what breaks is that the grid draws placeholders, and this line is the
        // only thing that would ever explain why.
        diag::error(format!("could not allow {}: {e}", assets.display()));
    }
}

/* ── nodes ────────────────────────────────────────────────────────────────── */

/// Node files that are on disk and cannot be parsed.
///
/// Separate from `node_list` because a file a sync client truncated may never
/// have had a node row to attach to.
#[tauri::command]
pub fn corrupt_files(state: State<'_, AppState>) -> CommandResult<Vec<CorruptFile>> {
    state.with(|p| Ok(p.corrupt_files()?))
}

/// Re-read the folder now, rather than waiting for the watcher.
///
/// The "reload" a broken file offers: the user has fixed it in a text editor
/// or restored it from a backup and wants to know whether that worked, without
/// having to guess at the debounce.
#[tauri::command]
pub fn project_reload(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    state.reconcile_now()?;
    let _ = app.emit(WORLD_CHANGED, ());
    Ok(())
}

/// Render a read-only, self-contained projection into a newly claimed folder.
///
/// Only reconciliation and cloning happen under the project lock. Strictly
/// reading every generation receipt, copying media, and rendering HTML can all
/// be slow on a share, so the store performs those steps on a blocking thread
/// after the lock has been released.
#[tauri::command]
pub async fn project_export_wiki(
    app: AppHandle,
    state: State<'_, AppState>,
    destination: String,
) -> CommandResult<WikiExport> {
    if destination.trim().is_empty() {
        return Err(wobu_store::Error::InvalidExportDestination(PathBuf::from(destination)).into());
    }
    let changed = state.reconcile_now()?;
    let snapshot = state.with(|project| Ok(project.wiki_snapshot()?))?;
    if changed {
        let _ = app.emit(WORLD_CHANGED, ());
    }
    let destination = PathBuf::from(destination);
    let exported = blocking("The static wiki export stopped unexpectedly.", move || {
        wobu_store::wiki::export(snapshot, &destination)
    })
    .await??;
    Ok(exported)
}
