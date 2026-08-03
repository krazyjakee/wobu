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

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, Emitter, Manager, State};
// Aliased because the command below has to *be* called `kind_registry` —
// Tauri v2 derives the invoke name from the function name, with no rename.
use wobu_core::kind_registry as registry;
use wobu_core::{
    Asset, AssetKind, AssetRole, FragmentTarget, Generation, Id, KindDef, Layer, LinkEdge,
    LinkRole, MeshAsset, Node, NodeKind, NodeSummary, Preset, default_preset,
};
use wobu_imagine::{
    View as MeshView, comfy, gemini as image_gemini, tencent::Region as HunyuanRegion,
};
use wobu_influence::{
    Budget, Chars, DropReason, Dropped, Fragment, FragmentBody, Reached, ResolvedStack, Shot,
    Sliders, World, compile, fragments, resolve,
};
use wobu_jobs::{JobId, QueueSnapshot};
use wobu_llm::{
    AnthropicProvider, Cancel, Discard, EnhanceOutcome, EnhanceRequest, GeminiProvider,
    TextProvider, Usage, anthropic, gemini,
};
use wobu_store::{
    AssetUsage, Conflict, CorruptFile, ImportedAsset, Keep, Peer, Project, ProjectSummary,
    GenerationPage, GenerationPageRequest, Resolved, SaveOutcome, TransferOutcome,
    TransferPreview, WikiExport, recent, transfer,
};

use crate::diag;
use crate::enhance::Pending;
use crate::error::{Code, CommandResult, WobuError};
use crate::keys::{KeyRemoval, KeyStatus, Keys, Secret};
use crate::machine::MachineSettings;
use crate::state::{AppState, Jobs, ProjectTicket, WORLD_CHANGED};

/// Reviewing a turnaround and reconstructing a mesh from it (#110). A submodule
/// rather than more of this file: it is the one command group with a job, a
/// provider adapter and a task of its own, and `mesh_concepts` above is only
/// the reading half of it.
pub mod mesh;

/// The one reader of `project.json`'s `providers` map, shared by images, text,
/// 3D and the status bar.
pub mod providers;

use providers::ProviderChoice;

/// How many thumbnails one IPC may ask for, assets or nodes alike.
///
/// A bound rather than a page size: the caller sends the window it is about to
/// draw, and a window that large is a caller that has stopped virtualizing.
const THUMB_BATCH_LIMIT: usize = 100;

const ASSET_TRANSFER_CHUNK_BYTES: usize = 1024 * 1024;
const ASSET_TRANSFER_MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Default)]
pub struct AssetTransfers(Mutex<HashMap<String, AssetTransfer>>);

struct AssetTransfer {
    project: ProjectTicket,
    path: PathBuf,
    file: File,
    kind: AssetKind,
    received_bytes: u64,
    total_bytes: u64,
}

impl AssetTransfers {
    fn take(&self, id: &str) -> Option<AssetTransfer> {
        self.0.lock().remove(id)
    }

    fn cancel(&self, id: &str) -> bool {
        self.take(id).is_some_and(|transfer| {
            remove_asset_transfer(transfer);
            true
        })
    }

    pub fn clear(&self) {
        for (_, transfer) in self.0.lock().drain() {
            remove_asset_transfer(transfer);
        }
    }
}

impl Drop for AssetTransfers {
    fn drop(&mut self) {
        for (_, transfer) in self.0.get_mut().drain() {
            remove_asset_transfer(transfer);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetTransferProgress {
    transfer_id: String,
    received_bytes: u64,
    total_bytes: u64,
}

/* ── registry ─────────────────────────────────────────────────────────────── */

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

#[tauri::command]
pub fn project_create(
    app: AppHandle,
    state: State<'_, AppState>,
    parent_dir: String,
    name: String,
) -> CommandResult<ProjectSummary> {
    let project = Project::create(&PathBuf::from(parent_dir), &name)?;
    Ok(adopt(&app, &state, project))
}

/// Inspect a source with a throwaway local index. The open destination is not
/// locked during the source scan.
#[tauri::command]
pub async fn style_transfer_preview(
    state: State<'_, AppState>,
    source_path: String,
) -> CommandResult<TransferPreview> {
    let destination =
        state.peek(|project| project.map(|project| (project.id(), project.root().to_path_buf())));
    let source = PathBuf::from(source_path);
    let same_path = destination.as_ref().is_some_and(|(_, root)| {
        std::fs::canonicalize(&source)
            .ok()
            .zip(std::fs::canonicalize(root).ok())
            .is_some_and(|(source, destination)| source == destination)
    });
    if same_path {
        return Err(wobu_store::Error::TransferSameProject.into());
    }
    let preview = tauri::async_runtime::spawn_blocking(move || transfer::preview(&source))
        .await
        .map_err(|error| {
        WobuError::new(Code::Internal, "Style transfer preview stopped unexpectedly.")
            .with_detail(error.to_string())
    })??;
    if destination.is_some_and(|(id, _)| id == preview.source_project_id) {
        return Err(wobu_store::Error::TransferSameProject.into());
    }
    Ok(preview)
}

/// Stage the source outside the destination lock, then publish the preflighted
/// graph through guarded writes. A non-complete outcome is a recoverable
/// partial report, not a command error that would hide what already landed.
#[tauri::command]
pub async fn style_transfer_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    source_path: String,
    root_id: Id,
) -> CommandResult<TransferOutcome> {
    let destination = state
        .peek(|project| project.map(|project| (project.id(), project.root().to_path_buf())))
        .ok_or_else(WobuError::no_project_open)?;
    let source = PathBuf::from(source_path);
    let same_path = std::fs::canonicalize(&source)
        .ok()
        .zip(std::fs::canonicalize(&destination.1).ok())
        .is_some_and(|(source, destination)| source == destination);
    if same_path {
        return Err(wobu_store::Error::TransferSameProject.into());
    }
    let bundle = tauri::async_runtime::spawn_blocking(move || transfer::stage(&source, root_id))
        .await
        .map_err(|error| {
            WobuError::new(Code::Internal, "Style transfer staging stopped unexpectedly.")
                .with_detail(error.to_string())
        })??;
    if bundle.source_project_id() == destination.0 {
        return Err(wobu_store::Error::TransferSameProject.into());
    }
    let outcome =
        state.with_project(destination.0, |project| Ok(project.apply_transfer(bundle)?))?;
    let _ = app.emit(WORLD_CHANGED, ());
    Ok(outcome)
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
    path: String,
) -> CommandResult<ProjectSummary> {
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

#[tauri::command]
pub fn node_list(state: State<'_, AppState>) -> CommandResult<Vec<NodeSummary>> {
    state.with(|p| Ok(p.list_nodes()?))
}

/// All explicit influence edges for the read-only relationship map.
/// Parent edges are already present on `NodeSummary` and are derived by the UI.
#[tauri::command]
pub fn node_links(state: State<'_, AppState>) -> CommandResult<Vec<LinkEdge>> {
    state.with(|p| Ok(p.node_links()?))
}

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

/// Full-text search over names, summaries, notes and descriptions.
///
/// Returns ids in rank order rather than whole nodes: the caller already holds
/// every summary from `node_list`, so sending them back would duplicate the
/// world across the bridge on every keystroke. The order is the part that
/// cannot be reconstructed on the other side.
#[tauri::command]
pub fn node_search(state: State<'_, AppState>, query: String) -> CommandResult<Vec<Id>> {
    state.with(|p| Ok(p.index().search(&query)?))
}

#[tauri::command]
pub fn node_get(state: State<'_, AppState>, id: Id) -> CommandResult<Node> {
    state.with(|p| Ok(p.get_node(id)?))
}

#[tauri::command]
pub fn node_create(
    state: State<'_, AppState>,
    kind: NodeKind,
    name: String,
    parent_id: Option<Id>,
) -> CommandResult<Node> {
    state.with(|p| Ok(p.create_node(kind, &name, parent_id)?))
}

#[tauri::command]
pub fn node_upsert(state: State<'_, AppState>, node: Node) -> CommandResult<Node> {
    state.with(|p| saved(p.save_node(node)?))
}

/// Lock or clear the entity seed without posting a stale copy of the node.
#[tauri::command]
pub fn node_seed_lock_set(
    state: State<'_, AppState>,
    node_id: Id,
    seed: Option<u64>,
) -> CommandResult<Node> {
    state.with(|project| saved(project.set_locked_seed(node_id, seed)?))
}

#[tauri::command]
pub fn node_delete(state: State<'_, AppState>, id: Id) -> CommandResult<()> {
    state.with(|p| Ok(p.delete_node(id)?))
}

#[tauri::command]
pub fn node_move(
    state: State<'_, AppState>,
    id: Id,
    new_parent_id: Option<Id>,
) -> CommandResult<()> {
    state.with(|p| Ok(p.move_node(id, new_parent_id)?))
}

/// Add one explicit influence edge. Parent relationships are edited through
/// `node_move`; they are implicit, fixed at weight 1, and never represented as
/// an editable `Link`.
#[tauri::command]
pub fn node_link_add(
    state: State<'_, AppState>,
    node_id: Id,
    to_id: Id,
    role: LinkRole,
    weight: Option<f32>,
    enabled: Option<bool>,
) -> CommandResult<Node> {
    state.with(|p| saved(p.add_node_link(node_id, to_id, role, weight, enabled)?))
}

#[tauri::command]
pub fn node_link_remove(
    state: State<'_, AppState>,
    node_id: Id,
    to_id: Id,
    role: LinkRole,
) -> CommandResult<Node> {
    state.with(|p| saved(p.remove_node_link(node_id, to_id, role)?))
}

/// Change either mutable property without posting a stale copy of the other.
#[tauri::command]
pub fn node_link_update(
    state: State<'_, AppState>,
    node_id: Id,
    to_id: Id,
    role: LinkRole,
    weight: Option<f32>,
    enabled: Option<bool>,
) -> CommandResult<Node> {
    state.with(|p| saved(p.update_node_link(node_id, to_id, role, weight, enabled)?))
}

/// Every explicit edge pointing at this node. Names and kind labels stay out
/// of the payload because the webview already holds the node summaries.
#[tauri::command]
pub fn node_backlinks(state: State<'_, AppState>, id: Id) -> CommandResult<Vec<LinkEdge>> {
    state.with(|p| Ok(p.node_backlinks(id)?))
}

/* ── assets ───────────────────────────────────────────────────────────────── */

/// Bring a file the user dropped or picked into the project folder.
///
/// The path is only ever read from — it names a file somewhere on the user's
/// machine, and nothing about it reaches the name the blob is stored under. See
/// `wobu_store::assets`.
///
/// No `world:changed` is emitted. The import writes a file inside the folder,
/// so the watcher raises the event on its own, and firing a second one here
/// would refetch the whole world twice for one drag.
///
/// `async` with the work on a blocking thread: reading a 300 MB scan off a
/// share is minutes, drawing its thumbnail is a decode, and neither may run on
/// the thread painting the window (#25).
#[tauri::command]
pub async fn asset_import(
    state: State<'_, AppState>,
    path: String,
    kind: AssetKind,
) -> CommandResult<ImportedAsset> {
    let handle = state.handle();
    let (project, root) = handle.ticket(|project| Ok(project.asset_import_root()?))?;
    blocking("The import thread stopped unexpectedly.", move || {
        let imported = import_file_unlocked(&root, &PathBuf::from(path), kind)?;
        commit_import(&handle, &project, imported)
    })
    .await?
}

/// Start a bounded raw-byte import for a paste or browser drop.
///
/// The body chunks arrive through [`InvokeBody::Raw`], never JSON. They are
/// appended to a local temporary file with backpressure, so neither side owns
/// the whole image as a second in-memory representation. The upper bound is a
/// user-facing refusal before the webview reads even the first chunk.
#[tauri::command]
pub fn asset_import_transfer_begin(
    state: State<'_, AppState>,
    asset_transfers: State<'_, AssetTransfers>,
    total_bytes: u64,
    kind: AssetKind,
) -> CommandResult<AssetTransferProgress> {
    if total_bytes == 0 || total_bytes > ASSET_TRANSFER_MAX_BYTES {
        return Err(invalid_asset_transfer(format!(
            "Pasted images must be between 1 byte and {} MiB.",
            ASSET_TRANSFER_MAX_BYTES / 1024 / 1024
        )));
    }
    let (project, ()) = state.ticket(|_| Ok(()))?;
    let transfer_id = wobu_core::new_id().to_string();
    let path = std::env::temp_dir().join(format!("wobu-asset-{transfer_id}.part"));
    let file =
        OpenOptions::new().write(true).create_new(true).open(&path).map_err(|error| {
            asset_transfer_io("Could not start the pasted image transfer.", error)
        })?;
    asset_transfers.0.lock().insert(
        transfer_id.clone(),
        AssetTransfer { project, path, file, kind, received_bytes: 0, total_bytes },
    );
    Ok(AssetTransferProgress { transfer_id, received_bytes: 0, total_bytes })
}

/// Append one raw IPC body to a transfer.
///
/// One MiB is both the protocol limit and the frontend chunk size. A malformed
/// offset, oversized body, or write failure destroys the session immediately;
/// there is no abandoned partial file waiting for a later Cancel click.
#[tauri::command]
pub fn asset_import_transfer_chunk(
    asset_transfers: State<'_, AssetTransfers>,
    request: Request<'_>,
) -> CommandResult<AssetTransferProgress> {
    let transfer_id = transfer_header(&request, "x-wobu-transfer-id")?.to_owned();
    let offset = match transfer_header(&request, "x-wobu-offset").and_then(|value| {
        value
            .parse::<u64>()
            .map_err(|_| invalid_asset_transfer("The pasted image chunk offset was invalid."))
    }) {
        Ok(offset) => offset,
        Err(error) => {
            asset_transfers.cancel(&transfer_id);
            return Err(error);
        }
    };
    let InvokeBody::Raw(bytes) = request.body() else {
        asset_transfers.cancel(&transfer_id);
        return Err(invalid_asset_transfer("The pasted image chunk was not binary."));
    };
    append_asset_transfer_chunk(&asset_transfers, &transfer_id, offset, bytes)
}

fn append_asset_transfer_chunk(
    asset_transfers: &AssetTransfers,
    transfer_id: &str,
    offset: u64,
    bytes: &[u8],
) -> CommandResult<AssetTransferProgress> {
    if bytes.is_empty() || bytes.len() > ASSET_TRANSFER_CHUNK_BYTES {
        asset_transfers.cancel(transfer_id);
        return Err(invalid_asset_transfer(format!(
            "Pasted image chunks must be between 1 byte and {ASSET_TRANSFER_CHUNK_BYTES} bytes."
        )));
    }

    let result = {
        let mut transfers = asset_transfers.0.lock();
        let transfer = transfers
            .get_mut(transfer_id)
            .ok_or_else(|| invalid_asset_transfer("That pasted image transfer is not active."))?;
        if offset != transfer.received_bytes
            || transfer.received_bytes + bytes.len() as u64 > transfer.total_bytes
        {
            Err(invalid_asset_transfer(
                "The pasted image chunks arrived out of order or exceeded the declared size.",
            ))
        } else {
            match transfer.file.write_all(bytes) {
                Ok(()) => {
                    transfer.received_bytes += bytes.len() as u64;
                    Ok(AssetTransferProgress {
                        transfer_id: transfer_id.to_owned(),
                        received_bytes: transfer.received_bytes,
                        total_bytes: transfer.total_bytes,
                    })
                }
                Err(error) => Err(asset_transfer_io("Could not buffer the pasted image.", error)),
            }
        }
    };
    if result.is_err() {
        asset_transfers.cancel(transfer_id);
    }
    result
}

/// Finish a complete transfer against the same project that began it.
#[tauri::command]
pub async fn asset_import_transfer_finish(
    state: State<'_, AppState>,
    asset_transfers: State<'_, AssetTransfers>,
    transfer_id: String,
) -> CommandResult<ImportedAsset> {
    let Some(mut transfer) = asset_transfers.take(&transfer_id) else {
        return Err(invalid_asset_transfer("That pasted image transfer is not active."));
    };
    if transfer.received_bytes != transfer.total_bytes {
        remove_asset_transfer(transfer);
        return Err(invalid_asset_transfer("The pasted image transfer is incomplete."));
    }
    let root =
        match state.with_ticket(&transfer.project, |project| Ok(project.asset_import_root()?)) {
            Ok(root) => root,
            Err(error) => {
                remove_asset_transfer(transfer);
                return Err(error);
            }
        };
    if let Err(error) = transfer.file.flush().and_then(|()| transfer.file.sync_all()) {
        remove_asset_transfer(transfer);
        return Err(asset_transfer_io("Could not finish buffering the pasted image.", error));
    }

    let AssetTransfer { project, path, file, kind, .. } = transfer;
    drop(file);
    let handle = state.handle();
    blocking("The import thread stopped unexpectedly.", move || {
        let result = import_file_unlocked(&root, &path, kind)
            .map_err(WobuError::from)
            .and_then(|imported| commit_import(&handle, &project, imported));
        let _ = fs::remove_file(path);
        result
    })
    .await?
}

/// Cancel is idempotent so an AbortSignal may race a completed chunk safely.
#[tauri::command]
pub fn asset_import_transfer_cancel(
    asset_transfers: State<'_, AssetTransfers>,
    transfer_id: String,
) {
    asset_transfers.cancel(&transfer_id);
}

fn transfer_header<'a>(request: &'a Request<'_>, name: &str) -> CommandResult<&'a str> {
    request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| invalid_asset_transfer(format!("The pasted image chunk omitted {name}.")))
}

fn invalid_asset_transfer(message: impl Into<String>) -> WobuError {
    WobuError::new(Code::Invalid, message)
}

fn asset_transfer_io(message: &'static str, error: std::io::Error) -> WobuError {
    WobuError::new(Code::Io, message).with_detail(error.to_string())
}

fn remove_asset_transfer(transfer: AssetTransfer) {
    let AssetTransfer { path, file, .. } = transfer;
    drop(file);
    let _ = fs::remove_file(path);
}

/// Read one source once, then publish its original and derive its thumbnail
/// from those same bytes. Everything in this function may be slow and callers
/// must run it without the project mutex held.
fn import_file_unlocked(
    root: &Path,
    source: &Path,
    kind: AssetKind,
) -> wobu_store::Result<ImportedAsset> {
    import_file_unlocked_with(root, source, kind, wobu_store::assets::read_cancellable)
}

fn import_file_unlocked_with(
    root: &Path,
    source: &Path,
    kind: AssetKind,
    read: impl FnOnce(&Path, &wobu_store::Cancel) -> wobu_store::Result<Vec<u8>>,
) -> wobu_store::Result<ImportedAsset> {
    let cancel = wobu_store::Cancel::new();
    let bytes = read(source, &cancel)?;
    let mut imported = wobu_store::assets::import_with(root, &bytes, kind, &cancel)?;

    let id = imported.asset.id;
    match wobu_store::thumbs::ensure_with_bytes(
        root,
        &imported.asset.hash,
        &imported.asset.rel_path,
        &bytes,
        &cancel,
    ) {
        Ok(thumb) => imported.asset.thumb_path = Some(thumb.rel_path),
        Err(error) => diag::error(format!("could not thumbnail {id}: {error}")),
    }
    Ok(imported)
}

/// The only mutex-held part of an import: install its already-published facts
/// in the local index, after verifying the exact open session is unchanged.
fn commit_import(
    state: &AppState,
    project: &ProjectTicket,
    imported: ImportedAsset,
) -> CommandResult<ImportedAsset> {
    state.with_ticket(project, |project| Ok(project.record_import(&imported)?))?;
    Ok(imported)
}

/// Filesystem and pixel half of one lazy thumbnail request.
fn ensure_thumb_unlocked(
    root: &Path,
    target: &wobu_store::ThumbTarget,
    can_write: bool,
) -> wobu_store::Result<Option<wobu_store::Thumbnail>> {
    if !wobu_store::thumbs::exists(root, &target.hash) && !can_write {
        return Ok(None);
    }
    match wobu_store::thumbs::ensure(
        root,
        &target.hash,
        &target.rel_path,
        &wobu_store::Cancel::new(),
    ) {
        Ok(thumbnail) => Ok(Some(thumbnail)),
        Err(wobu_store::Error::Undecodable { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Every blob in the open project, newest first.
#[tauri::command]
pub fn asset_list(state: State<'_, AppState>) -> CommandResult<Vec<Asset>> {
    state.with(|p| Ok(p.list_assets()?))
}

/// Every node/role/cover using every asset, assembled from the index-backed
/// world cache so tag and node filters do not open hundreds of Markdown files.
#[tauri::command]
pub fn asset_usage_list(state: State<'_, AppState>) -> CommandResult<Vec<AssetUsage>> {
    state.with(|p| Ok(p.asset_usages()?))
}

/// Permanently remove one true orphan. The store repeats the usage check after
/// UI confirmation and refuses any asset linked or used as a cover.
#[tauri::command]
pub fn asset_delete(state: State<'_, AppState>, asset_id: Id) -> CommandResult<()> {
    state.with(|p| Ok(p.delete_asset(asset_id)?))
}

/// One bounded page of lightweight Concepts history, newest first in SQLite.
#[tauri::command]
pub fn generation_list(
    state: State<'_, AppState>,
    node_id: Id,
    offset: u32,
    limit: u32,
) -> CommandResult<GenerationPage> {
    state.with(|project| {
        generation_page(
            project,
            GenerationPageRequest {
                node_id: Some(node_id),
                offset,
                limit,
                ..GenerationPageRequest::default()
            },
        )
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnaroundView {
    generation_id: Id,
    view_type: String,
    asset_id: Id,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshConcept {
    generation_id: Id,
    created_at: chrono::DateTime<chrono::Utc>,
    backend: String,
    model: String,
    asset: MeshAsset,
    turnaround: Vec<TurnaroundView>,
}

/// Lightweight 3D history. The directory scan reads fixed GLB headers only;
/// the complete mesh is not read or hashed until `mesh_asset_path` below.
#[tauri::command]
pub fn mesh_concepts(state: State<'_, AppState>, node_id: Id) -> CommandResult<Vec<MeshConcept>> {
    state.with(|project| {
        let generations = project.list_generations(node_id)?;
        let by_id: HashMap<_, _> = generations.iter().map(|item| (item.id, item)).collect();
        let meshes: HashMap<_, _> =
            project.list_meshes().into_iter().map(|mesh| (mesh.id, mesh)).collect();
        Ok(generations
            .iter()
            .filter_map(|generation| {
                let output = generation.mesh_output()?;
                let asset = meshes.get(&output.asset_id)?.clone();
                Some(MeshConcept {
                    generation_id: generation.id,
                    created_at: generation.created_at,
                    backend: generation.backend.clone(),
                    model: generation.model.clone(),
                    asset,
                    turnaround: turnaround_views(&output.turnaround_generation_ids, &by_id),
                })
            })
            .collect())
    })
}

fn turnaround_views(ids: &[Id], generations: &HashMap<Id, &Generation>) -> Vec<TurnaroundView> {
    if ids.len() != MeshView::ALL.len() {
        return Vec::new();
    }
    let views: Option<Vec<_>> = ids
        .iter()
        .map(|id| {
            let generation = generations.get(id)?;
            Some(TurnaroundView {
                generation_id: *id,
                view_type: generation.view_type.clone()?,
                asset_id: *generation.output_asset_ids.first()?,
            })
        })
        .collect();
    // A partial sheet is not "the sheet that produced this mesh". If even one
    // immutable source receipt is missing, show the explicit unavailable state.
    let views = views.unwrap_or_default();
    let distinct: HashSet<_> =
        views.iter().filter_map(|view| MeshView::parse(&view.view_type)).collect();
    if distinct.len() == MeshView::ALL.len() { views } else { Vec::new() }
}

/// Validate and expose one complete GLB. Async because this is the first full
/// mesh read and it may cross a slow share.
#[tauri::command]
pub async fn mesh_asset_path(
    state: State<'_, AppState>,
    asset_id: Id,
) -> CommandResult<Option<String>> {
    let (project_id, root) =
        state.with(|project| Ok((project.id(), project.root().to_path_buf())))?;
    let checked_root = root.clone();
    let mesh = blocking("The mesh validation thread stopped unexpectedly.", move || {
        wobu_store::assets::cached_mesh(&checked_root, project_id, asset_id)
    })
    .await??;
    let Some((_mesh, cached)) = mesh else { return Ok(None) };
    Ok(state
        .peek(|project| project.is_some_and(|project| project.id() == project_id))
        .then(|| cached.to_string_lossy().into_owned()))
}

/// Canonical project path for Finder/Explorer. Unlike the viewer path this is
/// not a local cache, and unlike loading it does not read the GLB body.
#[tauri::command]
pub fn mesh_source_path(state: State<'_, AppState>, asset_id: Id) -> CommandResult<Option<String>> {
    state.with(|project| {
        Ok(project
            .list_meshes()
            .into_iter()
            .find(|mesh| mesh.id == asset_id)
            .and_then(|mesh| absolute(project, &mesh.rel_path)))
    })
}

/// Copy a validated GLB to the location chosen by the modeller.
#[tauri::command]
pub async fn mesh_export(
    state: State<'_, AppState>,
    asset_id: Id,
    destination: String,
) -> CommandResult<()> {
    if destination.trim().is_empty() {
        return Err(WobuError::new(Code::Invalid, "Choose where to export the GLB."));
    }
    let destination = PathBuf::from(destination);
    let (project_id, root) =
        state.with(|project| Ok((project.id(), project.root().to_path_buf())))?;
    blocking("The mesh export thread stopped unexpectedly.", move || {
        let (_mesh, cached) = wobu_store::assets::cached_mesh(&root, project_id, asset_id)?
            .ok_or_else(|| wobu_store::Error::NoSuchAsset(asset_id.to_string()))?;
        std::fs::copy(&cached, &destination)
            .map(|_| ())
            .map_err(|error| wobu_store::Error::io(&destination, error))
    })
    .await??;
    Ok(())
}

/// The full immutable receipt for the one tile a person opened.
#[tauri::command]
pub fn generation_get(
    state: State<'_, AppState>,
    generation_id: Id,
) -> CommandResult<Option<Generation>> {
    state.with(|project| Ok(project.get_generation(generation_id)?))
}

/// Remove a generation from Concepts without erasing its spend record.
#[tauri::command]
pub fn generation_delete(state: State<'_, AppState>, generation_id: Id) -> CommandResult<()> {
    state.with(|project| Ok(project.delete_generation(generation_id)?))
}

/// Attach a reference image to a node in a role.
///
/// The role is the whole payload here: it is what decides whether the image
/// reaches a structure adapter, a style adapter, the colour pass, or — for
/// `mood` — nothing outside this machine at all. See `wobu_core::AssetRole`.
///
/// Returns the saved node, like `node_upsert`, because this is an edit to that
/// node's file and the caller needs the version that won.
#[tauri::command]
pub fn asset_link(
    state: State<'_, AppState>,
    node_id: Id,
    asset_id: Id,
    role: AssetRole,
    weight: Option<f32>,
) -> CommandResult<Node> {
    state.with(|p| saved(p.link_asset(node_id, asset_id, role, weight)?))
}

/// Detach one. The blob stays — assets are shared, and content-addressed, so
/// the last link going away says nothing about whether the file is wanted.
#[tauri::command]
pub fn asset_unlink(
    state: State<'_, AppState>,
    node_id: Id,
    asset_id: Id,
    role: AssetRole,
) -> CommandResult<Node> {
    state.with(|p| saved(p.unlink_asset(node_id, asset_id, role)?))
}

/// Adjust a link's weight, its enabled flag, or both.
///
/// Each is optional and absent means "leave it": the slider and the mute toggle
/// are separate controls, and posting the whole link from either would let a
/// stale copy of one overwrite what the user just did with the other.
#[tauri::command]
pub fn asset_link_update(
    state: State<'_, AppState>,
    node_id: Id,
    asset_id: Id,
    role: AssetRole,
    weight: Option<f32>,
    enabled: Option<bool>,
) -> CommandResult<Node> {
    state.with(|p| saved(p.update_asset_link(node_id, asset_id, role, weight, enabled)?))
}

/// Choose the image that represents a node, or clear it with a null.
///
/// Separate from linking on purpose: a cover is what a card shows, and making
/// it imply a link would mean picking a thumbnail quietly changed what gets
/// sent to a backend.
#[tauri::command]
pub fn asset_set_cover(
    state: State<'_, AppState>,
    node_id: Id,
    asset_id: Option<Id>,
) -> CommandResult<Node> {
    state.with(|p| saved(p.set_cover_asset(node_id, asset_id)?))
}

/* ── thumbnails (#25) ─────────────────────────────────────────────────────── */

/// Emitted while the library's missing thumbnails are being drawn. Payload is
/// `ScanProgress`, the same shape `project:open-progress` carries.
pub const THUMB_PROGRESS: &str = "assets:thumb-progress";

/// The absolute path of one blob's thumbnail, drawing it if the folder has not
/// got one.
///
/// **This is what a grid tile binds to, and it is the only thing it binds to.**
/// The path comes back for `convertFileSrc`, so the webview loads a ~30 KB WebP
/// over the asset protocol instead of being handed a base64 copy of a 40 MB
/// scan for every tile on screen. Full-resolution originals are `asset_original`
/// and are fetched one at a time, when an image is actually opened.
///
/// `async` with the work on a blocking thread, because drawing one is a decode
/// and a resize: cheap for a screenshot, a few hundred milliseconds for a
/// 6000px scan, and neither belongs on the thread painting the window.
///
/// `null` rather than an error for the three cases where there is legitimately
/// no thumbnail — no such asset, a read-only or unreachable folder, a blob whose
/// pixels will not decode. A tile draws a placeholder for all three; none of
/// them is something the user can act on.
#[tauri::command]
pub async fn asset_thumb(
    state: State<'_, AppState>,
    asset_id: Id,
) -> CommandResult<Option<String>> {
    let handle = state.handle();
    let prepared =
        handle.ticket(|project| Ok((project.thumb_target(asset_id)?, project.can_write_thumb())));
    let (project, (target, can_write)) = match prepared {
        Ok(prepared) => prepared,
        Err(error) if error.code == Code::ShareUnmounted => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(target) = target else { return Ok(None) };
    let root = project.root().to_path_buf();
    let work = target.clone();
    let thumbnail = blocking("The thumbnail thread stopped unexpectedly.", move || {
        ensure_thumb_unlocked(&root, &work, can_write)
    })
    .await??;
    let Some(thumbnail) = thumbnail else { return Ok(None) };

    // Pure local-index mutation under the mutex. The path was proved present
    // by `ensure` above; joining it does not touch the filesystem.
    let commit = handle.with_ticket(&project, |project| {
        if thumbnail.generated {
            project.verify_writable()?;
        }
        Ok(project.record_thumb_targets(std::slice::from_ref(&target))?)
    });
    if let Err(error) = commit {
        if error.code == Code::ShareUnmounted || error.code == Code::ReadOnly {
            return Ok(None);
        }
        return Err(error);
    }
    Ok(Some(
        wobu_store::paths::from_rel_string(project.root(), &thumbnail.rel_path)
            .to_string_lossy()
            .into_owned(),
    ))
}

/// Resolve/generate the thumbnails for one bounded history page in one IPC.
#[tauri::command]
pub async fn asset_thumb_batch(
    state: State<'_, AppState>,
    asset_ids: Vec<Id>,
) -> CommandResult<HashMap<String, String>> {
    if asset_ids.len() > THUMB_BATCH_LIMIT {
        return Err(WobuError::new(
            Code::Invalid,
            "A thumbnail page may contain at most 100 assets.",
        ));
    }
    thumb_paths(state.handle(), &asset_ids).await
}

/// One thumbnail per *node*, for every list that draws entities rather than blobs.
///
/// The navigator, the palette, the relation lists and the influence stack all
/// show entities, and an entity is not an asset — so the webview cannot use
/// `asset_thumb_batch` for them without first learning which picture stands for
/// which node, which is a question only the index can answer. Doing that here
/// keeps it at one IPC for a whole visible window rather than one per row,
/// which is the entire point (#146, and #97 before it).
///
/// Nodes with no picture are simply absent from the map rather than present
/// with a null: "no thumbnail" and "thumbnail not drawn yet" are the same thing
/// to a caller that draws a fallback for both, and an absent key is cheaper to
/// send for the common case of a text-only world.
#[tauri::command]
pub async fn node_thumb_batch(
    state: State<'_, AppState>,
    node_ids: Vec<Id>,
) -> CommandResult<HashMap<String, String>> {
    if node_ids.len() > THUMB_BATCH_LIMIT {
        return Err(WobuError::new(
            Code::Invalid,
            "A thumbnail page may contain at most 100 nodes.",
        ));
    }
    let pairs = match state.with(|project| node_thumb_assets(project, &node_ids)) {
        Ok(pairs) => pairs,
        // A closed or unmounted project draws placeholders, exactly as one
        // whose blobs will not decode does. Neither is actionable from a row.
        Err(error) if error.code == Code::NoProjectOpen || error.code == Code::ShareUnmounted => {
            return Ok(HashMap::new());
        }
        Err(error) => return Err(error),
    };
    if pairs.is_empty() {
        return Ok(HashMap::new());
    }
    let mut asset_ids: Vec<Id> = Vec::with_capacity(pairs.len());
    let mut seen = HashSet::new();
    for (_, asset_id) in &pairs {
        if seen.insert(*asset_id) {
            asset_ids.push(*asset_id);
        }
    }
    let by_asset = thumb_paths(state.handle(), &asset_ids).await?;
    Ok(pairs
        .into_iter()
        .filter_map(|(node_id, asset_id)| {
            by_asset.get(&asset_id.to_string()).map(|path| (node_id.to_string(), path.clone()))
        })
        .collect())
}

/// The picture that stands for each node, in the order the caller asked.
///
/// Read from the local index rather than from `world_nodes`: the navigator asks
/// for this on the first paint, and materialising the whole world to answer it
/// would move a cost that today is paid only by projects that open the
/// Inspector onto every project that opens at all.
fn node_thumb_assets(project: &Project, node_ids: &[Id]) -> CommandResult<Vec<(Id, Id)>> {
    let mut pairs = Vec::new();
    let mut seen = HashSet::new();
    for node_id in node_ids {
        if !seen.insert(*node_id) {
            continue;
        }
        if let Some(asset_id) = node_thumb_asset(project.index(), *node_id)? {
            pairs.push((*node_id, asset_id));
        }
    }
    Ok(pairs)
}

/// Cover first, then the first live reference, then the newest concept output.
///
/// That order is the user's own: a cover is an explicit choice about how this
/// entity should be shown, so nothing may override it. A disabled reference is
/// still preferred over nothing, because `enabled` says whether a picture is
/// *sent to a backend* — it was never a statement about display.
fn node_thumb_asset(index: &wobu_store::Index, node_id: Id) -> CommandResult<Option<Id>> {
    if let Some(cover) = index.cover_asset_of(node_id)? {
        return Ok(Some(cover));
    }
    let links = index.asset_links_of(node_id)?;
    if let Some(link) = links.iter().find(|link| link.enabled).or_else(|| links.first()) {
        return Ok(Some(link.asset_id));
    }
    let page = index.generation_page(&GenerationPageRequest {
        node_id: Some(node_id),
        offset: 0,
        limit: 1,
        ..GenerationPageRequest::default()
    })?;
    Ok(page.items.first().and_then(|item| item.first_asset_id))
}

/// The shared body of both thumbnail batches: asset ids in, absolute paths out.
async fn thumb_paths(
    handle: AppState,
    asset_ids: &[Id],
) -> CommandResult<HashMap<String, String>> {
    let prepared = handle.ticket(|project| {
        Ok((project.thumb_targets(asset_ids)?, project.can_write_thumb()))
    })?;
    let (project, (targets, can_write)) = prepared;
    if targets.is_empty() {
        return Ok(HashMap::new());
    }
    let root = project.root().to_path_buf();
    let work = targets.clone();
    let completed = blocking("The thumbnail batch thread stopped unexpectedly.", move || {
        if !can_write {
            return Ok(work
                .iter()
                .filter(|target| wobu_store::thumbs::exists(&root, &target.hash))
                .map(|target| target.asset_id)
                .collect());
        }
        wobu_store::thumbs::ensure_all(
            &root,
            &work,
            &wobu_store::Cancel::new(),
            &mut |_| {},
        )
    })
    .await??;
    let completed: HashSet<_> = completed.into_iter().collect();
    let completed_targets: Vec<_> = targets
        .iter()
        .filter(|target| completed.contains(&target.asset_id))
        .cloned()
        .collect();
    handle.with_ticket(&project, |project| {
        if can_write {
            project.verify_writable()?;
        }
        Ok(project.record_thumb_targets(&completed_targets)?)
    })?;
    Ok(completed_targets
        .into_iter()
        .map(|target| {
            let path = wobu_store::paths::from_rel_string(
                project.root(),
                &wobu_store::thumbs::rel_path(&target.hash),
            )
            .to_string_lossy()
            .into_owned();
            (target.asset_id.to_string(), path)
        })
        .collect())
}

/// The absolute path of one blob itself, for the viewer.
///
/// Deliberately a separate command from `asset_thumb` rather than a flag on it:
/// the grid must never be able to reach an original by accident, because a
/// hundred tiles each pulling a 40 MB file off a share is the failure this whole
/// issue is about. One call, one picture, when somebody opens it.
#[tauri::command]
pub fn asset_original(state: State<'_, AppState>, asset_id: Id) -> CommandResult<Option<String>> {
    state.with(|p| {
        let asset = p.get_asset(asset_id)?;
        Ok(asset.and_then(|a| absolute(p, &a.rel_path)))
    })
}

/// Draw every thumbnail the open project is missing.
///
/// The other half of "missing thumbs are regenerated lazily": a folder that
/// arrived over sync, out of a zip or off a USB stick can have a full
/// `assets/originals/` and no `assets/thumbs/` at all. `asset_thumb` covers one
/// tile scrolling into view; this covers the case where the answer is "all of
/// them", and it exists so that the grid is not drawing a thousand placeholders
/// while a thousand separate commands queue up behind the project mutex.
///
/// Three steps for one reason, and it is the rule in `state.rs`: read the list
/// under the lock, grind through it with *nothing* held, then record the results
/// under the lock again. The middle step is minutes for a large library on a
/// share, and holding the mutex across it would freeze every other command.
///
/// Returns how many blobs now have a thumbnail. Progress is emitted rather than
/// returned, exactly as `project_open`'s is.
#[tauri::command]
pub async fn asset_thumbs_ensure(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<usize> {
    let prepared =
        state.ticket(|project| Ok((project.missing_thumbs()?, project.can_write_thumb())));
    let (project, (targets, can_write)) = match prepared {
        Ok(prepared) => prepared,
        Err(error) if error.code == Code::ShareUnmounted => return Ok(0),
        Err(error) => return Err(error),
    };
    if targets.is_empty() {
        return Ok(0);
    }
    let root = project.root().to_path_buf();
    let work = targets.clone();

    let cancel = state.begin_thumbs();
    let done = {
        let emitter = app.clone();
        blocking("The thumbnail thread stopped unexpectedly.", move || {
            let mut last = 0u8;
            if !can_write {
                return Ok(work
                    .iter()
                    .filter(|target| wobu_store::thumbs::exists(&root, &target.hash))
                    .map(|target| target.asset_id)
                    .collect());
            }
            wobu_store::thumbs::ensure_all(&root, &work, &cancel, &mut |p| {
                // Throttled to whole percentage points, for the reason
                // `project_open` throttles: a thousand events through the
                // bridge is work taken from the pass itself.
                let pct = p.percent();
                if pct != last {
                    last = pct;
                    let _ = emitter.emit(THUMB_PROGRESS, p);
                }
            })
        })
        .await?
    };
    state.finish_thumbs();

    // Cancelling reports nothing and loses nothing. `Cancelled` is not a
    // failure (see `wobu_store::Error`), and every thumbnail the pass did draw
    // is on disk at a path only that picture can claim — so the next tile to ask
    // for one, or the next run of this, finds it already there and free.
    let made = match done {
        Ok(made) => made,
        Err(wobu_store::Error::Cancelled) => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let made: HashSet<_> = made.into_iter().collect();
    let completed: Vec<_> =
        targets.into_iter().filter(|target| made.contains(&target.asset_id)).collect();
    state.with_ticket(&project, |project| {
        if can_write {
            project.verify_writable()?;
        }
        Ok(project.record_thumb_targets(&completed)?)
    })?;
    // The folder gained files, but under `assets/thumbs/` — which the watcher
    // does not treat as a world change, and which nothing would otherwise
    // invalidate. Without this the grid keeps its placeholders until something
    // else happens to touch the project.
    let _ = app.emit(WORLD_CHANGED, ());
    Ok(completed.len())
}

/// Stop a thumbnail pass in progress.
///
/// A no-op when there is none, for the same reason `project_open_cancel` is: the
/// user can press it at the moment the pass finishes and that race must not be
/// an error.
#[tauri::command]
pub fn asset_thumbs_cancel(state: State<'_, AppState>) {
    state.cancel_thumbs();
}

fn generation_page(
    project: &Project,
    request: GenerationPageRequest,
) -> CommandResult<GenerationPage> {
    let mut page = project.generation_page(&request)?;
    for item in &mut page.items {
        if let Some(relative) = item.thumbnail_path.as_deref() {
            item.thumbnail_path = Some(
                wobu_store::paths::from_rel_string(project.root(), relative)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    Ok(page)
}

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
/// Shared by the thumbnail commands so that "the pool went away" reads the same
/// from all of them; `project_open` predates it and spells its own out inline.
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

/// Where one layer card's weight slider sits.
///
/// A list of pairs rather than an object keyed by node id. Both deserialize, but
/// a key that is not a ULID would be dropped in silence, and a slider that
/// quietly applies to no card is indistinguishable from an engine ignoring the
/// user.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SliderSetting {
    node_id: Id,
    value: f32,
    #[serde(default)]
    muted: bool,
}

/// The Shot layer, as the Inspector's controls describe it.
///
/// Both fields optional, because layer 7 is the one the panel owns and the panel
/// is #47. `label` is only what the card is titled — the framing text itself
/// comes from the preset — so it defaults to the preset's label, which is what
/// the card would have said anyway.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotControls {
    label: Option<String>,
    weight: Option<f32>,
    /// Extra framing typed for this run. Separate from `label`, which only
    /// names the card and is never sent to a provider.
    prompt: Option<String>,
}

/// What one compilation may spend on text.
///
/// Characters, not tokens: there is no tokenizer in this workspace and
/// deliberately will not be one (`wobu_influence::Chars`). Either limit absent
/// means unlimited, because no backend has been chosen yet — `Capabilities`
/// (#50) is what will state a real one, and inventing a number here would drop
/// fragments to fit a limit nobody has measured.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptBudget {
    prompt_chars: Option<usize>,
    negative_chars: Option<usize>,
}

/// One thing one layer contributes, with everything needed to point at where it
/// came from.
///
/// One shape for all three lists — a card's contributions, the spans in the
/// compiled prompt, and the drop report — because they are the same fragments
/// seen from three angles, and the panel draws the same row for each. The
/// alternative was three near-identical interfaces that would drift apart the
/// first time one gained a field.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfluenceFragment {
    layer: Layer,
    /// Null for the Shot layer, whose framing text comes from the preset rather
    /// than from any node.
    node_id: Option<Id>,
    source_name: String,
    section: &'static str,
    /// Prose. Null for a reference image, which carries `assetId` instead —
    /// exactly one of the two is ever set.
    text: Option<String>,
    asset_id: Option<Id>,
    /// `link.weight × section_priority × user_slider`, already multiplied out.
    weight: f32,
    target: FragmentTarget,
    /// Whether this may be put in front of a provider. False only for
    /// `moodboard_only`, and read from the engine rather than re-derived from
    /// the target here: two lists of what is private would be one rename away
    /// from disagreeing, and that disagreement fails in the direction of
    /// somebody's mood board on a third party's servers.
    sendable: bool,
}

/// One layer card.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerCard {
    layer: Layer,
    node_id: Option<Id>,
    /// What the card is titled — the node's name, or the shot's label.
    name: String,
    kind: Option<NodeKind>,
    reached: Reached,
    /// Hops from whichever root reached this source. The subject and the two
    /// seeded singletons are 0.
    distance: u16,
    /// The product of the link weights along the path that reached this source.
    /// Kept apart from `slider` so the panel can show what each contributed.
    weight: f32,
    slider: f32,
    fragments: Vec<InfluenceFragment>,
}

/// The resolved stack for one subject.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfluenceStack {
    subject_id: Id,
    /// The preset this was resolved under — the caller's, or the kind's default
    /// when it named none. Returned whole rather than as an id because the panel
    /// needs its aspect and image count to describe what Generate would do, and
    /// a second round trip for a `&'static` table would be a round trip for
    /// nothing.
    preset: &'static Preset,
    layers: Vec<LayerCard>,
}

/// One fragment the compiler left out, and why.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DroppedFragment {
    fragment: InfluenceFragment,
    reason: DropReason,
}

/// The two prompt strings, and the account of everything that is not in them.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledPrompt {
    subject_id: Id,
    preset: &'static Preset,
    prompt: String,
    negative: String,
    /// The fragments that are in the two strings above, in emission order. This
    /// is what lets the compiled-prompt box tint each span by origin, which
    /// `docs/04-influence-engine.md` calls the main feedback loop for learning to
    /// write good upstream notes rather than a debug feature.
    spans: Vec<InfluenceFragment>,
    /// Everything left out, in reading order, so the panel can walk it alongside
    /// the layer cards. Present because "the Inspector reports what was dropped
    /// rather than truncating silently" — a command that returned only the
    /// prompt would throw away the thing that makes the panel worth having.
    dropped: Vec<DroppedFragment>,
    /// How far over its budget the positive prompt is, or null when it fits.
    /// Only ever set when the budget could not fit even one fragment: the
    /// compiler keeps the heaviest and says so, because an empty prompt is not a
    /// smaller picture, it is a different one that still costs money.
    overflow: Option<usize>,
}

/// The resolved stack for a subject, with the per-layer detail the Inspector's
/// layer cards read.
///
/// Answers from the local index and never touches the project folder, so this is
/// as fast on a share that is currently unplugged as on an SSD — see
/// `Project::world_nodes`. A project with no Style Guide, or none of the links
/// the stack walks, resolves to a short list rather than an error; that is the
/// state every project is in on day one and the panel is on screen for all of it.
#[tauri::command]
pub fn influence_resolve(
    state: State<'_, AppState>,
    subject_id: Id,
    preset: Option<String>,
    sliders: Option<Vec<SliderSetting>>,
    shot: Option<ShotControls>,
) -> CommandResult<InfluenceStack> {
    let sliders = sliders_from(sliders);
    state.with(|p| {
        resolved(p.world_nodes()?, subject_id, preset.as_deref(), &sliders, shot.as_ref())
    })
}

/// The compiled positive and negative prompt, the spans they are made of, and
/// the account of what did not make it.
///
/// Called on every Inspector interaction — every slider drag, every preset
/// change — so it does no IO at all: the world comes out of the local index and
/// the engine itself is pure.
#[tauri::command]
pub fn prompt_compile(
    state: State<'_, AppState>,
    subject_id: Id,
    preset: Option<String>,
    sliders: Option<Vec<SliderSetting>>,
    shot: Option<ShotControls>,
    budget: Option<PromptBudget>,
) -> CommandResult<CompiledPrompt> {
    let sliders = sliders_from(sliders);
    let budget = budget_from(budget);
    state.with(|p| {
        compiled(p.world_nodes()?, subject_id, preset.as_deref(), &sliders, shot.as_ref(), budget)
    })
}

/// Everything [`influence_resolve`] does once it has the nodes.
///
/// Separated from the command so that the tests below exercise the payload the
/// webview actually receives rather than a re-assembly of it — the bridge
/// contract is the thing worth pinning, and a test that built its own struct
/// would agree with itself no matter what the command sent.
fn resolved<'a>(
    nodes: &'a [Node],
    subject_id: Id,
    preset: Option<&str>,
    sliders: &Sliders,
    shot: Option<&'a ShotControls>,
) -> CommandResult<InfluenceStack> {
    let sheet = preset_for(nodes, subject_id, preset)?;
    let user_prompt = shot.and_then(|controls| controls.prompt.as_deref());
    // No Shot layer unless the caller named one: resolving for display is not
    // resolving for a generation, and a card invented here would put framing on
    // screen for a shot nobody has set up (`wobu_influence::Shot`).
    let shot = shot.map(|controls| Shot {
        label: controls.label.as_deref().unwrap_or("Shot"),
        weight: controls.weight.unwrap_or(1.0),
    });
    let (stack, mut extracted) = prepare(nodes, subject_id, sheet, sliders, shot)?;
    append_shot_prompt(&stack, &mut extracted, user_prompt);
    Ok(InfluenceStack {
        subject_id,
        preset: sheet,
        layers: layer_cards(&stack, &extracted, sliders),
    })
}

/// Everything [`prompt_compile`] does once it has the nodes.
fn compiled<'a>(
    nodes: &'a [Node],
    subject_id: Id,
    preset: Option<&str>,
    sliders: &Sliders,
    shot: Option<&'a ShotControls>,
    budget: Budget,
) -> CommandResult<CompiledPrompt> {
    let sheet = preset_for(nodes, subject_id, preset)?;
    let user_prompt = shot.and_then(|controls| controls.prompt.as_deref());
    // Always a shot, unlike `resolved`. The preset's framing text is the Shot
    // layer's whole contribution, so a prompt compiled without one would differ
    // from the prompt a generation actually sends — which is the single thing
    // this panel must never be wrong about.
    let shot = Shot {
        label: shot.and_then(|c| c.label.as_deref()).unwrap_or(sheet.label),
        weight: shot.and_then(|c| c.weight).unwrap_or(1.0),
    };
    let (stack, mut extracted) = prepare(nodes, subject_id, sheet, sliders, Some(shot))?;
    append_shot_prompt(&stack, &mut extracted, user_prompt);

    let compiled = compile(&extracted, budget);
    Ok(CompiledPrompt {
        subject_id,
        preset: sheet,
        prompt: compiled.prompt().to_owned(),
        negative: compiled.negative().to_owned(),
        spans: prompt_spans(&extracted, compiled.dropped()),
        dropped: compiled
            .dropped()
            .iter()
            .map(|d| DroppedFragment { fragment: fragment_view(&d.fragment), reason: d.reason })
            .collect(),
        overflow: compiled.overflow().map(Chars::get),
    })
}

/// Resolve the stack and extract its fragments — the half both commands share.
fn prepare<'a>(
    nodes: &'a [Node],
    subject_id: Id,
    sheet: &Preset,
    sliders: &Sliders,
    shot: Option<Shot<'a>>,
) -> CommandResult<(ResolvedStack<'a>, Vec<Fragment<'a>>)> {
    let world = World::new(nodes.iter());
    // `resolve` is `None` only for a subject outside the view, which
    // `preset_for` has already ruled out at both call sites; restated rather
    // than unwrapped because a panic here would take the window with it.
    let stack = resolve(&world, subject_id, shot).ok_or_else(|| no_such_subject(subject_id))?;
    let extracted = fragments(&stack, sheet, sliders);
    Ok((stack, extracted))
}

fn append_shot_prompt<'a>(
    stack: &ResolvedStack<'a>,
    extracted: &mut Vec<Fragment<'a>>,
    prompt: Option<&'a str>,
) {
    let Some(prompt) = prompt.map(str::trim).filter(|prompt| !prompt.is_empty()) else {
        return;
    };
    if let Some(source) = stack.sources().iter().find(|source| source.layer == Layer::Shot) {
        extracted.push(Fragment::new(
            source,
            "user_prompt",
            FragmentBody::Text(prompt),
            source.weight,
            FragmentTarget::Prompt,
        ));
    }
}

/// The preset a compilation runs under, and the check that the subject exists.
///
/// The two together because the default preset is a property of the subject's
/// kind, so there is no answering the first without the second.
///
/// A preset id the registry has never heard of falls back to the kind's default
/// rather than failing. `Generation.preset` is a string that outlives any one
/// build (`wobu-core`'s `preset.rs`), so a snapshot naming a preset since
/// renamed must still compile to something rather than take the panel down.
fn preset_for(
    nodes: &[Node],
    subject_id: Id,
    preset: Option<&str>,
) -> CommandResult<&'static Preset> {
    let subject =
        nodes.iter().find(|n| n.id == subject_id).ok_or_else(|| no_such_subject(subject_id))?;
    Ok(preset.and_then(wobu_core::preset).unwrap_or_else(|| default_preset(subject.kind)))
}

/// A subject that is not in the world.
///
/// The ordinary cause is a tab or an Inspector still pointing at something a
/// collaborator deleted, which is why it is `node.not_found` and not an internal
/// error: the frontend already knows what to do with that code.
fn no_such_subject(id: Id) -> WobuError {
    WobuError::new(Code::NoSuchNode, "That entity is not in this project any more.")
        .with_detail(id.to_string())
}

fn sliders_from(settings: Option<Vec<SliderSetting>>) -> Sliders {
    Sliders::from_pairs(
        settings
            .unwrap_or_default()
            .into_iter()
            .map(|s| (s.node_id, if s.muted { 0.0 } else { s.value })),
    )
}

fn budget_from(budget: Option<PromptBudget>) -> Budget {
    let Some(budget) = budget else { return Budget::unlimited() };
    Budget {
        prompt: budget.prompt_chars.map_or(Chars::UNLIMITED, Chars::new),
        negative: budget.negative_chars.map_or(Chars::UNLIMITED, Chars::new),
    }
}

fn layer_cards<'a>(
    stack: &ResolvedStack<'a>,
    extracted: &[Fragment<'a>],
    sliders: &Sliders,
) -> Vec<LayerCard> {
    stack
        .sources()
        .iter()
        .map(|source| LayerCard {
            layer: source.layer,
            node_id: source.node_id(),
            name: source.name().to_owned(),
            kind: source.node().map(|n| n.kind),
            reached: source.reached,
            distance: source.distance,
            weight: source.weight,
            slider: sliders.for_source(source),
            // Grouped on layer and node because resolution visits each node
            // exactly once — first visit wins — so the pair names one card and
            // never two. Extraction does emit in source order, but grouping on
            // that would make the cards depend on an ordering no type states.
            fragments: extracted
                .iter()
                .filter(|f| f.layer() == source.layer && f.node_id() == source.node_id())
                .map(fragment_view)
                .collect(),
        })
        .collect()
}

fn fragment_view(fragment: &Fragment<'_>) -> InfluenceFragment {
    InfluenceFragment {
        layer: fragment.layer(),
        node_id: fragment.node_id(),
        source_name: fragment.source_name().to_owned(),
        section: fragment.section(),
        text: fragment.text().map(str::to_owned),
        asset_id: fragment.asset_id(),
        weight: fragment.weight(),
        target: fragment.target(),
        sendable: fragment.is_sendable(),
    }
}

/// The fragments that are actually in the two compiled strings, in the order
/// they were emitted.
///
/// Derived from the drop report rather than re-decided: by `compile`'s own
/// account, a text fragment that is sendable and not in `dropped` is in one of
/// the two prompts. Working it out that way keeps one compiler in the workspace
/// — a second opinion here about what fits would disagree with the first the
/// moment either changed, and the symptom would be a prompt box highlighting
/// spans that are not in the prompt.
///
/// A `moodboard_only` fragment is in neither list and so appears in neither: it
/// is not sendable, and `compile` deliberately does not report it as a casualty.
///
/// The report is a subsequence of `extracted` in reading order, so one cursor
/// over each is enough. Two fragments that compare equal are identical in every
/// field a caller can see, which is why crediting the wrong one of a pair cannot
/// change the answer.
fn prompt_spans<'a>(extracted: &[Fragment<'a>], dropped: &[Dropped<'a>]) -> Vec<InfluenceFragment> {
    let mut cut = dropped.iter().peekable();
    let mut out = Vec::new();
    for fragment in extracted {
        if cut.peek().is_some_and(|d| d.fragment == *fragment) {
            cut.next();
            continue;
        }
        if fragment.text().is_some() && fragment.is_sendable() {
            out.push(fragment_view(fragment));
        }
    }
    out
}

/* ── conflicts ────────────────────────────────────────────────────────────── */

/// Unresolved conflict siblings in the open project.
///
/// Read from the folder on every call rather than pushed. A sibling can be
/// parked by a *different machine*, so there is no event on this side that
/// could keep a cached list honest — `world:changed` invalidates this alongside
/// the node list, and a failing save invalidates it directly.
#[tauri::command]
pub fn conflicts(state: State<'_, AppState>) -> CommandResult<Vec<Conflict>> {
    state.with(|p| Ok(p.conflicts()?))
}

/// Carry out the user's decision about one conflict sibling.
///
/// The only command in Wobu that deletes a file the user did not name as a
/// node, which is why `expected_hash` is required rather than optional: it is
/// the hash of the node file as the card rendered it, and a mismatch means the
/// diff the user answered is not the one on disk. That comes back as
/// `{ outcome: "stale" }` with both files untouched, and the card redraws.
#[tauri::command]
pub fn conflict_resolve(
    app: AppHandle,
    state: State<'_, AppState>,
    rel_path: String,
    keep: Keep,
    expected_hash: String,
) -> CommandResult<Resolved> {
    let outcome = state.with(|p| Ok(p.resolve_conflict(&rel_path, keep, &expected_hash)?))?;
    if matches!(outcome, Resolved::Done) {
        diag::info(format!("conflict resolved at {rel_path} keeping {keep:?}"));
        // Only on a real change. A stale or raced resolution left the folder
        // exactly as it was, and waking the whole world for it would refetch
        // the node the user is mid-decision about.
        let _ = app.emit(WORLD_CHANGED, ());
    }
    Ok(outcome)
}

/* ── presence ─────────────────────────────────────────────────────────────── */

/// Who else has this project open.
///
/// Polled rather than pushed. The answer only changes at human speed, and an
/// event per beat per peer would be traffic on the very share whose latency the
/// presence is there to explain.
///
/// Never fails, and is empty when no project is open — advisory information
/// arriving as an error would put a toast on screen for a poll that merely
/// raced a close.
#[tauri::command]
pub fn presence_peers(state: State<'_, AppState>) -> Vec<Peer> {
    state.peers()
}

/// Record which nodes this session has open, for everyone else's benefit.
///
/// The whole list rather than an add/remove pair: the frontend already knows
/// which nodes are open, and a delta protocol drifts the first time one closes
/// during a disconnection and then stays wrong for the rest of the session.
///
/// Advisory in the strongest sense. No write path reads this, and naming a node
/// here neither reserves it nor stops anyone — including the person who named
/// it — from saving or deleting it.
#[tauri::command]
pub fn presence_editing(state: State<'_, AppState>, node_ids: Vec<Id>) {
    state.set_editing(node_ids);
}

/* ── storage and about ────────────────────────────────────────────────────── */

/// The local index for the open project.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    path: String,
    size_bytes: u64,
    /// Zero until something has been indexed — a project that failed to scan
    /// looks identical to one that has not been opened otherwise.
    node_count: u64,
}

#[tauri::command]
pub fn index_info(state: State<'_, AppState>) -> CommandResult<IndexInfo> {
    state.with(|p| {
        let path = p.index_path();
        Ok(IndexInfo {
            size_bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            path: path.to_string_lossy().into_owned(),
            node_count: p.list_nodes()?.len() as u64,
        })
    })
}

/// Throw the index away and rebuild it from the Markdown.
///
/// Safe to expose because the index holds no canonical data. It is the support
/// answer to "the navigator is showing something that isn't there".
#[tauri::command]
pub fn index_rebuild(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    state.with(|p| {
        p.rebuild_index()?;
        Ok(())
    })?;
    diag::info("index rebuilt on request");
    let _ = app.emit(WORLD_CHANGED, ());
    Ok(())
}

/// Version numbers worth quoting in a bug report.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutInfo {
    app_version: String,
    /// The on-disk format of the project folder. The number that decides
    /// whether a newer Wobu's world can be opened here at all.
    project_schema_version: u32,
    /// The local index layout. Bumping it silently rebuilds, so a user seeing
    /// a long pause after an update is seeing this change.
    index_schema_version: u32,
    log_path: String,
}

#[tauri::command]
pub fn about_info() -> AboutInfo {
    AboutInfo {
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        project_schema_version: wobu_core::SCHEMA_VERSION,
        index_schema_version: wobu_store::index::INDEX_VERSION,
        log_path: diag::global()
            .map(|d| d.path())
            .unwrap_or_else(|| diag::dir().join("wobu.log"))
            .to_string_lossy()
            .into_owned(),
    }
}

/* ── diagnostics ──────────────────────────────────────────────────────────── */

/// What Settings needs to describe the log without reading it.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogInfo {
    /// Absolute, and shown to the user — they may well go and find it by hand.
    path: String,
    level: diag::Level,
    /// False until something has been recorded. The UI says so rather than
    /// offering to reveal a file that is not there.
    exists: bool,
    size_bytes: u64,
}

#[tauri::command]
pub fn log_info() -> LogInfo {
    let (path, level) = match diag::global() {
        Some(d) => (d.path(), d.level()),
        None => (diag::dir().join("wobu.log"), diag::Level::default()),
    };
    let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    LogInfo { exists: path.is_file(), path: path.to_string_lossy().into_owned(), level, size_bytes }
}

#[tauri::command]
pub fn log_set_level(level: diag::Level) {
    if let Some(d) = diag::global() {
        d.set_level(level);
        // Recorded at error so it lands whatever the new level is — when
        // reading a log the first question is always "was it even on?".
        diag::error(format!("log level set to {level:?}"));
    }
}

/// The end of the log, for showing the user what they are about to hand over.
#[tauri::command]
pub fn log_tail(lines: usize) -> String {
    diag::global().map(|d| d.tail(lines)).unwrap_or_default()
}

/// Show the file in the OS file manager, which is how it gets attached to
/// something. Falls back to the folder when nothing has been logged yet.
#[tauri::command]
pub fn log_reveal() -> CommandResult<()> {
    let dir = diag::dir();
    std::fs::create_dir_all(&dir).map_err(|e| {
        WobuError::new(Code::Io, "Could not open the log folder.").with_detail(e.to_string())
    })?;

    let path = diag::global().map(|d| d.path()).unwrap_or_else(|| dir.join("wobu.log"));
    let target = if path.is_file() { path } else { dir };

    tauri_plugin_opener::reveal_item_in_dir(&target).map_err(|e| {
        WobuError::new(Code::Io, "Could not show the log in the file manager.")
            .with_detail(e.to_string())
    })
}

/* ── provider keys ────────────────────────────────────────────────────────── */

/// Whether this machine has a key for each of these providers.
///
/// Presence, never value — `keys.rs` says why, and there is no command anywhere
/// that returns key material. A list rather than one provider at a time because
/// the pane that renders these renders every row at once, and a call per row
/// would be a credential-store round trip per row.
///
/// No `Result`: a machine with no keychain, or a locked one, is an ordinary
/// machine, and the answer for it is "unconfigured" rather than a failure.
#[tauri::command]
pub fn provider_key_status(keys: State<'_, Keys>, providers: Vec<String>) -> Vec<KeyStatus> {
    providers.iter().map(|p| keys.status(p)).collect()
}

/// Store a key for a provider.
///
/// The one command that carries key material, and it carries it *inwards*: the
/// user pasted it into a field, so it is already in the webview and the only
/// question is where it goes next. Nothing sends one back.
///
/// The argument is never logged. `WobuError::new` and `diag` both scrub, so even
/// a mistake here would be masked rather than published — but the rule is that
/// nothing in this function mentions `key` at all.
#[tauri::command]
pub fn provider_key_set(
    keys: State<'_, Keys>,
    provider: String,
    key: String,
) -> CommandResult<KeyStatus> {
    keys.set(&provider, &key)
}

/// Remove this machine's stored key for a provider.
///
/// The result tells "removed" and "there was nothing to remove" apart, because
/// they are different sentences and only one of them is worth showing.
#[tauri::command]
pub fn provider_key_delete(keys: State<'_, Keys>, provider: String) -> CommandResult<KeyRemoval> {
    keys.delete(&provider)
}

/* ── the provider selection ───────────────────────────────────────────────── */

/// Which job a selection is for.
///
/// Three, not one list. A user enhancing with Gemini, generating on a ComfyUI
/// running on the machine under their desk and meshing through Hunyuan3D is the
/// ordinary case rather than the exotic one (`docs/08-providers.md`), and a
/// single "provider" setting would make it unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Text,
    Image,
    Mesh,
}

impl Capability {
    /// The key this capability's selection sits under in `project.json`'s
    /// `providers`.
    ///
    /// `"text"` is not a name chosen here: `enhance.rs` already reads that key
    /// to decide who writes descriptions, and the two have to agree or a
    /// selection made in Settings is written somewhere Enhance never looks.
    fn key(self) -> &'static str {
        match self {
            Capability::Text => "text",
            Capability::Image => "image",
            Capability::Mesh => "mesh",
        }
    }
}

/// The shared half of the providers pane: what `project.json` says.
///
/// Carried as the raw map rather than as three typed fields, because a project
/// written by a build that knows a fourth capability must survive a round trip
/// through this one. The frontend reads the capabilities it understands and
/// leaves the rest alone, which is the same contract `ProjectMeta` has with the
/// file.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSelections {
    pub providers: serde_json::Map<String, serde_json::Value>,
    /// Whether the *selection* can be changed here. Keys are unaffected: they
    /// are per installation and go to this machine's keychain, so a read-only
    /// world is one you can still add a key for and still generate from.
    pub read_only: bool,
}

/// What this project has chosen, for the pane that has to show it as shared.
#[tauri::command]
pub fn project_providers(state: State<'_, AppState>) -> CommandResult<ProviderSelections> {
    state.with(|p| Ok(selections(p)))
}

/// Choose a provider for one capability, and write it into `project.json`.
///
/// This is the only command that writes project metadata, and it exists because
/// there was no other: `wobu-store` writes `project.json` when a project is
/// created and never again. Anything else that needs to change that file should
/// come through here rather than open a second way to do it.
///
/// The selection is *shared*. It travels with the folder to everyone on the
/// share, which is exactly why the key does not — see `keys.rs`.
#[tauri::command]
pub fn project_provider_select(
    state: State<'_, AppState>,
    capability: Capability,
    provider: String,
    model: Option<String>,
    region: Option<String>,
) -> CommandResult<ProviderSelections> {
    let provider = provider.trim().to_owned();
    if provider.is_empty() {
        return Err(WobuError::new(Code::Invalid, "A capability needs a provider."));
    }
    // Trimmed to nothing means "whatever the adapter's default is", which is a
    // real answer and is spelled as the absence of the field — the same thing
    // `enhance.rs` reads an empty string as.
    let model = model.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty());
    let region = provider_region(capability, &provider, region)?;

    state.with(|project| {
        if project.is_read_only() {
            return Err(WobuError::new(
                Code::ReadOnly,
                "This project folder is read-only, so the provider it uses cannot be changed \
                 here. Keys can still be added — those live on this machine.",
            ));
        }

        let root = project.root().to_path_buf();
        let mut providers = project.meta().providers.clone();
        // Merged into whatever is already under this capability rather than
        // replacing it. Default params live in the same object
        // (`docs/08-providers.md`), and a build that only knows about `provider`
        // and `model` must not delete the rest of somebody's settings by
        // touching a dropdown.
        let mut chosen = providers
            .get(capability.key())
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        chosen.insert("provider".to_owned(), serde_json::Value::String(provider));
        match model {
            Some(model) => {
                chosen.insert("model".to_owned(), serde_json::Value::String(model));
            }
            None => {
                chosen.remove("model");
            }
        }
        // Omitted means "leave the existing region alone": provider buttons
        // and model edits do not silently move an existing project between
        // data-processing regions. A value only comes from the explicit
        // Hunyuan region picker above.
        if let Some(region) = region {
            chosen.insert("region".to_owned(), serde_json::Value::String(region));
        }
        providers.insert(capability.key().to_owned(), serde_json::Value::Object(chosen));
        write_providers(&root, &providers)?;

        // Reopened rather than patched in memory, because `Project` hands out
        // `&ProjectMeta` and nothing else — `meta` is what was read at open
        // time. Without this the user would change the provider, press Enhance,
        // and be billed by the one they just moved away from, which is the
        // exact failure `enhance.rs`'s selection code is written to prevent.
        //
        // It costs a `reconcile` — the same walk the Reload button does — and
        // this runs when somebody picks from a dropdown, not in a loop.
        *project = Project::open(&root)?;
        Ok(selections(project))
    })
}

fn provider_region(
    capability: Capability,
    provider: &str,
    region: Option<String>,
) -> CommandResult<Option<String>> {
    let region = region.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
    if let Some(region) = &region
        && (capability != Capability::Mesh
            || provider != "hunyuan3d"
            || HunyuanRegion::parse(region).is_none())
    {
        return Err(WobuError::new(
            Code::Invalid,
            "Tencent Hunyuan3D region must be ap-singapore, na-siliconvalley or eu-frankfurt.",
        ));
    }
    Ok(region)
}

fn selections(project: &Project) -> ProviderSelections {
    ProviderSelections {
        providers: project.meta().providers.clone(),
        read_only: project.is_read_only(),
    }
}

/* ── status-bar provider health ──────────────────────────────────────────── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveModel {
    pub provider: String,
    pub label: String,
    pub model: String,
    /// Known window for a shipped text model. Unknown custom model ids stay
    /// `None`; guessing a context window would make the status bar dangerous.
    pub context_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum BackendHealth {
    Connected { external_queue: Option<u32> },
    Unavailable { detail: String },
    Unconfigured { detail: String },
    Unsupported { detail: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusBarBackend {
    pub image: Option<ActiveModel>,
    pub text: ActiveModel,
    pub health: BackendHealth,
}

/// The provider facts the status bar can defend.
///
/// The project lock is released before the reachability request. Holding it
/// across a network call would freeze every editor read behind a health check.
#[tauri::command]
pub async fn status_bar_backend(
    state: State<'_, AppState>,
    keys: State<'_, Keys>,
    machine: State<'_, MachineSettings>,
) -> CommandResult<StatusBarBackend> {
    let (image, text) = state.with(|project| {
        Ok((ProviderChoice::of(project, "image"), ProviderChoice::of(project, "text")))
    })?;

    let text_provider = text.as_ref().map_or(anthropic::ID, |choice| choice.provider.as_str());
    let text_model = text
        .as_ref()
        .and_then(|choice| choice.configured_model.clone())
        .unwrap_or_else(|| text_default(text_provider).to_owned());
    let text = ActiveModel {
        provider: text_provider.to_owned(),
        label: provider_label(text_provider).to_owned(),
        context_tokens: context_window(text_provider, &text_model),
        model: text_model,
    };

    let Some(image) = image else {
        return Ok(StatusBarBackend {
            image: None,
            text,
            health: BackendHealth::Unconfigured {
                detail: "No image backend is selected for this project.".into(),
            },
        });
    };

    let image_provider = image.provider.clone();
    let image_model = image.model(None, image_default(&image_provider));
    let image = ActiveModel {
        provider: image_provider.clone(),
        label: provider_label(&image_provider).to_owned(),
        model: image_model.clone(),
        context_tokens: None,
    };

    let health = match image_provider.as_str() {
        comfy::ID => match machine.comfy_image() {
            Ok(backend) => match backend.health(&image_model).await {
                comfy::Health::Connected { queue, .. } => {
                    BackendHealth::Connected { external_queue: Some(queue) }
                }
                comfy::Health::Unreachable { detail } => BackendHealth::Unavailable { detail },
            },
            Err(error) => BackendHealth::Unavailable { detail: error.to_string() },
        },
        image_gemini::ID => match keys.secret(image_gemini::ID) {
            None => BackendHealth::Unconfigured {
                detail: "Gemini is selected for images, but this machine has no Gemini key.".into(),
            },
            Some(secret) => match image_gemini::GeminiBackend::new(secret.expose()) {
                Ok(backend) => match backend.check_key(&image_model, &Cancel::new()).await {
                    image_gemini::KeyCheck::Usable => {
                        BackendHealth::Connected { external_queue: None }
                    }
                    check => BackendHealth::Unavailable { detail: check.message() },
                },
                Err(error) => BackendHealth::Unavailable { detail: error.to_string() },
            },
        },
        other => BackendHealth::Unsupported {
            detail: format!("This build has no image adapter for {other}."),
        },
    };

    Ok(StatusBarBackend { image: Some(image), text, health })
}

fn provider_label(provider: &str) -> &str {
    match provider {
        anthropic::ID => anthropic::LABEL,
        gemini::ID => gemini::LABEL,
        comfy::ID => comfy::LABEL,
        _ => provider,
    }
}

fn text_default(provider: &str) -> &str {
    match provider {
        anthropic::ID => anthropic::DEFAULT_MODEL,
        gemini::ID => gemini::DEFAULT_MODEL,
        _ => "unknown provider default",
    }
}

fn image_default(provider: &str) -> &str {
    match provider {
        comfy::ID => comfy::DEFAULT_MODEL,
        image_gemini::ID => image_gemini::DEFAULT_MODEL,
        _ => "unknown provider default",
    }
}

fn context_window(provider: &str, model: &str) -> Option<u64> {
    match (provider, model) {
        (anthropic::ID, "claude-haiku-4-5") => Some(200_000),
        (anthropic::ID, "claude-opus-5" | "claude-sonnet-5" | "claude-fable-5") => Some(1_000_000),
        (gemini::ID, "gemini-3.6-flash") => Some(1_048_576),
        (gemini::ID, "gemini-3.5-flash" | "gemini-3.5-flash-lite") => Some(1_000_000),
        _ => None,
    }
}

/// Put `providers` into `project.json`, leaving every other byte of meaning
/// alone.
///
/// Read back as raw JSON and patched at one key rather than re-serialised from
/// `ProjectMeta`: a field written by a newer Wobu would not survive a round trip
/// through a struct that has never heard of it, and `project.json` is precisely
/// the file two builds of different vintages share across a drive.
///
/// Staged and renamed, on the same filesystem so the rename is atomic. This is
/// the file that decides whether a folder is a project at all — a half-written
/// one is a world that will not open, for everyone on the share at once.
fn write_providers(
    root: &Path,
    providers: &serde_json::Map<String, serde_json::Value>,
) -> CommandResult<()> {
    let path = root.join("project.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| meta_write_failed("could not be read", e.to_string()))?;
    let mut meta: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| meta_write_failed("could not be read", e.to_string()))?;
    let Some(object) = meta.as_object_mut() else {
        return Err(meta_write_failed("is not a JSON object", raw));
    };
    object.insert("providers".to_owned(), serde_json::Value::Object(providers.clone()));

    let staging = root.join(".wobu").join("tmp");
    std::fs::create_dir_all(&staging)
        .map_err(|e| meta_write_failed("could not be staged", e.to_string()))?;
    let part = staging.join("project.json.part");
    let text = serde_json::to_string_pretty(&meta)
        .map_err(|e| meta_write_failed("could not be written", e.to_string()))?;
    std::fs::write(&part, text)
        .map_err(|e| meta_write_failed("could not be staged", e.to_string()))?;
    std::fs::rename(&part, &path)
        .map_err(|e| meta_write_failed("could not be written", e.to_string()))?;
    Ok(())
}

fn meta_write_failed(what_happened: &str, detail: String) -> WobuError {
    WobuError::new(Code::Io, format!("This project's `project.json` {what_happened}."))
        .with_detail(detail)
}

/* ── the capability probe ─────────────────────────────────────────────────── */

/// The one node kind the probe asks about.
///
/// A prop has the shortest description in the registry, so it is the cheapest
/// schema to hand a provider and the fastest thing for one to start answering.
const PROBE_KIND: NodeKind = NodeKind::Prop;

/// Deliberately trivial, and deliberately not about anybody's world. The probe
/// asks a real question because a provider only reveals whether it will take our
/// schema by being given it.
const PROBE_PROMPT: &str = "A plain iron nail. One short line per section.";

/// How much of the answer to let the provider produce.
///
/// This is the whole trick that makes the check free enough to offer. Everything
/// the probe is there to find out — the key is accepted, the model id exists for
/// this account, the description schema is one the provider will take, and the
/// model has started emitting the structured document — is settled in the first
/// few tokens. Letting the answer finish would buy nothing except a bill.
const PROBE_MAX_OUTPUT_TOKENS: u32 = 24;

/// What a probe found out, in the terms the Settings pane renders.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub provider: String,
    /// The model actually asked about — the adapter's own default when the
    /// project named none, which is the fact a user has no other way to learn.
    pub model: String,
    pub ok: bool,
    /// One sentence for the pane. On success it says what was *proved*, because
    /// a green tick beside a key field is a claim the user cannot check.
    pub message: String,
    /// The stable dotted code, so a rejected key can be shown differently from
    /// a provider that is having an outage. `None` when the probe passed.
    pub code: Option<String>,
    /// What the check cost, as the provider reported it. Returned rather than
    /// assumed: a button that spends money silently is the thing this pane
    /// exists to prevent.
    pub usage: Usage,
}

/// Check a stored key against the provider it belongs to, at key-entry time.
///
/// The point is *when* it runs. Without this, a mistyped key is discovered by
/// pressing Enhance on a node and watching a job fail — the failure surfaces at
/// generate time, in a place that has nothing to do with credentials, and often
/// on somebody else's machine. Here it surfaces beside the field that caused it.
///
/// What it proves, in order: this machine has a key; the provider accepts it;
/// the model id resolves for this account; the description schema is one the
/// provider will take at all (Google documents a subset of JSON Schema, so this
/// is a real answer and not a formality); and the model begins emitting the
/// structured document. What it does *not* prove is that a full description
/// validates — the answer is cut off at [`PROBE_MAX_OUTPUT_TOKENS`] on purpose,
/// and pretending otherwise would mean charging for a description nobody asked
/// for every time a user pastes a key.
///
/// A provider failure is a *result*, not a rejection: "Anthropic says this key
/// is wrong" is the answer the pane asked for and belongs beside the field, not
/// in a toast. Only the two things that mean the probe could not run at all —
/// no key on this machine, a provider this build does not have — come back as
/// errors.
#[tauri::command]
pub async fn provider_probe(
    keys: State<'_, Keys>,
    provider: String,
    model: Option<String>,
) -> CommandResult<ProbeResult> {
    let secret = keys.secret(&provider).ok_or_else(|| {
        WobuError::new(
            Code::ProviderNoKey,
            "There is no key for this provider on this machine, so there is nothing to check.",
        )
    })?;
    let adapter = probe_provider(&provider, &secret)?;
    let model = model
        .map(|m| m.trim().to_owned())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| adapter.default_model().to_owned());

    let request = EnhanceRequest::new(PROBE_KIND, &model, PROBE_PROMPT)
        .with_max_output_tokens(PROBE_MAX_OUTPUT_TOKENS);
    // A fresh token that nothing holds: the probe is a few hundred milliseconds
    // and there is no Stop button in Settings to wire one to.
    let outcome = adapter.enhance(&request, &mut Discard, &Cancel::new()).await;

    Ok(verdict(adapter.as_ref(), model, outcome))
}

/// Read an `EnhanceOutcome` as an answer about the key rather than as an answer
/// about a nail.
fn verdict(adapter: &dyn TextProvider, model: String, outcome: EnhanceOutcome) -> ProbeResult {
    let label = adapter.label();
    let usage = outcome.usage;
    let (ok, message, code) = match outcome.result {
        // Only reachable if a provider fits a whole description into the token
        // ceiling above, which no current one does — but it is the strongest
        // possible pass and reporting it as a failure would be absurd.
        Ok(_) => (true, format!("{label} answered with a complete description."), None),
        Err(error) => {
            let code = error.code();
            // `wobu_llm::Error` is split into "the call" and "the answer", and
            // every variant on the answer side lands on this one code. Reaching
            // the answer side at all means the key, the model id and the schema
            // were all accepted — the request got as far as generating — which
            // is precisely what the probe set out to establish. Matching on the
            // code rather than on the variants keeps this from having to be
            // revisited every time one is added.
            if code == "provider.bad_response" {
                (
                    true,
                    format!(
                        "{label} took the key and started writing with {model}. The check stops \
                         the answer after a few tokens, so it did not finish one."
                    ),
                    None,
                )
            } else {
                (false, error.to_string(), Some(code.to_owned()))
            }
        }
    };
    ProbeResult { provider: adapter.id().to_owned(), model, ok, message, code, usage }
}

/// The text adapters this build has, by the id `project.json` and the keychain
/// both use.
///
/// A second construction site — `enhance.rs` has the same match, private to
/// itself — and the duplication is deliberate rather than overlooked: the
/// modules do not export to each other and the probe must not be the reason
/// `enhance.rs` grows a public surface. What has to stay true is the *set of
/// ids*, and both sides read those from `anthropic::ID` and `gemini::ID` rather
/// than spelling them out, so an adapter added to one and not the other is a
/// probe that cannot check the provider Enhance would actually run.
fn probe_provider(id: &str, key: &Secret) -> CommandResult<Arc<dyn TextProvider>> {
    let built = match id {
        anthropic::ID => {
            AnthropicProvider::new(key.expose()).map(|p| Arc::new(p) as Arc<dyn TextProvider>)
        }
        gemini::ID => {
            GeminiProvider::new(key.expose()).map(|p| Arc::new(p) as Arc<dyn TextProvider>)
        }
        // Not a bug and not a broken key: ComfyUI needs no credential at all,
        // and the image and mesh backends are not wired into this shell yet. A
        // key for one of those can still be stored — it is per installation and
        // will be waiting — there is simply nothing here to ask.
        _ => {
            return Err(WobuError::new(
                Code::Invalid,
                "This build has no way to check that provider's key.",
            )
            .with_detail(id.to_owned()));
        }
    };
    built.map_err(|e| {
        WobuError::new(Code::ProviderUnavailable, "This provider could not be started.")
            .with_detail(e.to_string())
    })
}

/* ── jobs ─────────────────────────────────────────────────────────────────── */

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

/// The bridge contract, pinned.
///
/// `src/lib/api.ts` hand-writes the TypeScript for every payload here. Nothing
/// generates one from the other, so the only thing stopping them drifting is a
/// test that feeds the Rust side exactly the JSON the webview sends. These use
/// literal JSON rather than round-tripping a Rust value on purpose — a
/// round-trip would agree with itself no matter what the frontend believes.
#[cfg(test)]
mod bridge {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use wobu_store::{AssetUsageRole, ImportWarning};

    use super::*;

    fn staged_asset_transfer(total_bytes: u64) -> (AssetTransfers, String, PathBuf) {
        let transfers = AssetTransfers::default();
        let transfer_id = wobu_core::new_id().to_string();
        let path = std::env::temp_dir().join(format!("wobu-transfer-test-{transfer_id}.part"));
        let file = OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        transfers.0.lock().insert(
            transfer_id.clone(),
            AssetTransfer {
                project: ProjectTicket {
                    project: wobu_core::new_id(),
                    root: PathBuf::new(),
                    generation: 0,
                },
                path: path.clone(),
                file,
                kind: AssetKind::Reference,
                received_bytes: 0,
                total_bytes,
            },
        );
        (transfers, transfer_id, path)
    }

    #[test]
    fn raw_asset_chunks_are_bounded_ordered_and_never_retained_in_memory() {
        let total = ASSET_TRANSFER_CHUNK_BYTES as u64 + 17;
        let (transfers, transfer_id, path) = staged_asset_transfer(total);
        let chunk = vec![7; ASSET_TRANSFER_CHUNK_BYTES];

        let first = append_asset_transfer_chunk(&transfers, &transfer_id, 0, &chunk).unwrap();
        assert_eq!(first.received_bytes, ASSET_TRANSFER_CHUNK_BYTES as u64);
        assert_eq!(fs::metadata(&path).unwrap().len(), ASSET_TRANSFER_CHUNK_BYTES as u64);
        assert!(
            std::mem::size_of::<AssetTransfer>() < 256,
            "a transfer session must hold file metadata, never a byte Vec"
        );

        let done = append_asset_transfer_chunk(
            &transfers,
            &transfer_id,
            ASSET_TRANSFER_CHUNK_BYTES as u64,
            &[9; 17],
        )
        .unwrap();
        assert_eq!(done.received_bytes, total);
        assert_eq!(fs::metadata(&path).unwrap().len(), total);
        assert!(transfers.cancel(&transfer_id));
        assert!(!path.exists(), "Cancel must remove the staged file");
    }

    #[test]
    fn the_chunked_transfer_import_reads_its_staged_source_once_before_the_index_commit() {
        let dir =
            std::env::temp_dir().join(format!("wobu-transfer-import-{}", wobu_core::new_id()));
        fs::create_dir_all(&dir).unwrap();
        let mut project = Project::create(&dir, "Transfer target").unwrap();
        let root = project.asset_import_root().unwrap();
        let staged = dir.join("completed-transfer.part");
        let reads = AtomicUsize::new(0);

        let imported =
            import_file_unlocked_with(&root, &staged, AssetKind::Reference, |path, _cancel| {
                assert_eq!(path, staged);
                reads.fetch_add(1, Ordering::SeqCst);
                // Header-complete so import succeeds; deliberately pixel-
                // incomplete so the best-effort immediate thumbnail is null.
                let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
                png.extend_from_slice(&13u32.to_be_bytes());
                png.extend_from_slice(b"IHDR");
                png.extend_from_slice(&64u32.to_be_bytes());
                png.extend_from_slice(&64u32.to_be_bytes());
                png.extend_from_slice(&[8, 6, 0, 0, 0]);
                Ok(png)
            })
            .unwrap();

        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert!(project.list_assets().unwrap().is_empty(), "slow work must not mutate the index");
        project.record_import(&imported).unwrap();
        assert_eq!(project.list_assets().unwrap(), [imported.asset]);
        drop(project);
        let _ = fs::remove_dir_all(dir);
    }

    /// A blob record without the blob: enough for every index question here.
    fn staged_asset(project: &Project, seed: &str) -> Id {
        let hash = seed.repeat(32);
        let asset = Asset {
            id: wobu_core::new_id(),
            hash: hash.clone(),
            kind: AssetKind::Reference,
            rel_path: format!("assets/originals/{}/{hash}.png", &hash[..2]),
            thumb_path: None,
            mime: "image/png".into(),
            width: 8,
            height: 8,
            bytes: 12,
            created_at: chrono::Utc::now(),
        };
        project.index().upsert_asset(&asset).unwrap();
        asset.id
    }

    fn concept_receipt(node_id: Id, output: Id) -> Generation {
        Generation {
            id: wobu_core::new_id(),
            node_id,
            created_at: chrono::Utc::now(),
            preset: "portrait".into(),
            view_type: None,
            user_prompt: String::new(),
            compiled_prompt: "an ashwalker".into(),
            negative_prompt: String::new(),
            backend: "comfyui".into(),
            model: "flux-dev".into(),
            seed: 7,
            params: serde_json::Map::new(),
            output_asset_ids: vec![output],
            influence_snapshot: wobu_core::InfluenceSnapshot { layers: Vec::new() },
        }
    }

    #[test]
    fn a_row_picture_prefers_the_cover_then_a_reference_then_the_newest_concept() {
        let dir = std::env::temp_dir().join(format!("wobu-node-thumb-{}", wobu_core::new_id()));
        fs::create_dir_all(&dir).unwrap();
        let mut project = Project::create(&dir, "Row pictures").unwrap();
        let node = project.create_node(NodeKind::Character, "Kael", None).unwrap();

        // A text-only entity is absent from the answer rather than present with
        // a null: the row draws its kind icon and asks for nothing.
        assert_eq!(node_thumb_asset(project.index(), node.id).unwrap(), None);
        assert!(node_thumb_assets(&project, &[node.id]).unwrap().is_empty());

        let concept = staged_asset(&project, "c0");
        let reference = staged_asset(&project, "b1");
        let cover = staged_asset(&project, "a2");

        // A generated concept stands in when nothing has been chosen, which is
        // the case the issue is really about: entities whose only picture was
        // produced by Forge and never pinned anywhere.
        project.record_generation(concept_receipt(node.id, concept)).unwrap();
        assert_eq!(node_thumb_asset(project.index(), node.id).unwrap(), Some(concept));

        // An attached reference outranks it — including one switched off, since
        // `enabled` decides what a backend is sent and was never a statement
        // about what the user should be able to see in a list.
        project.link_asset(node.id, reference, AssetRole::Pose, None).unwrap();
        project
            .update_asset_link(node.id, reference, AssetRole::Pose, None, Some(false))
            .unwrap();
        assert_eq!(node_thumb_asset(project.index(), node.id).unwrap(), Some(reference));

        // And an explicit cover outranks everything, because it is the one
        // answer the user gave on purpose.
        project.set_cover_asset(node.id, Some(cover)).unwrap();
        assert_eq!(node_thumb_asset(project.index(), node.id).unwrap(), Some(cover));

        // Repeats collapse: a caller may send whatever its window contains.
        assert_eq!(
            node_thumb_assets(&project, &[node.id, node.id]).unwrap(),
            vec![(node.id, cover)]
        );

        drop(project);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_read_only_thumbnail_snapshot_never_attempts_a_missing_file_write() {
        let root =
            std::env::temp_dir().join(format!("wobu-read-only-thumb-{}", wobu_core::new_id()));
        fs::create_dir_all(&root).unwrap();
        let target = wobu_store::ThumbTarget {
            asset_id: wobu_core::new_id(),
            hash: "a3f9c1d2e4b5a6978081726354453627a3f9c1d2e4b5a6978081726354453627".into(),
            rel_path: "assets/originals/a3/missing.png".into(),
        };

        assert_eq!(ensure_thumb_unlocked(&root, &target, false).unwrap(), None);
        assert!(
            !root.join("assets").exists(),
            "a read-only/missing thumbnail request must perform no write at all"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_oversized_or_out_of_order_chunk_destroys_its_temp_session() {
        let (oversized, oversized_id, oversized_path) =
            staged_asset_transfer((ASSET_TRANSFER_CHUNK_BYTES + 1) as u64);
        let too_large = vec![0; ASSET_TRANSFER_CHUNK_BYTES + 1];
        assert!(append_asset_transfer_chunk(&oversized, &oversized_id, 0, &too_large).is_err());
        assert!(!oversized_path.exists());

        let (out_of_order, out_of_order_id, out_of_order_path) = staged_asset_transfer(8);
        assert!(append_asset_transfer_chunk(&out_of_order, &out_of_order_id, 4, &[1; 4]).is_err());
        assert!(!out_of_order_path.exists());
    }

    /// Verbatim from the `WobuNode` interface in `src/lib/api.ts`, including
    /// the tagged `SectionValue` shape and a `null` description state.
    const NODE_FROM_THE_WEBVIEW: &str = r#"{
        "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
        "kind": "species",
        "name": "Vashk",
        "slug": "vashk",
        "summary": "Ash-adapted, subterranean.",
        "parentId": null,
        "notesRaw": "Notes typed in the editor.",
        "description": {
            "sections": {
                "silhouette": { "type": "text", "value": "Long-limbed." },
                "materials": { "type": "list", "value": ["ashglass", "bone"] }
            }
        },
        "descriptionState": "edited",
        "attributes": { "height_cm": 190 },
        "tags": ["playable"],
        "coverAssetId": null,
        "links": [
            { "toId": "01ARZ3NDEKTSV4RRFFQ69G5FAW", "role": "styled_by", "weight": 0.5, "enabled": true }
        ],
        "assetLinks": [
            { "assetId": "01ARZ3NDEKTSV4RRFFQ69G5FAX", "role": "pose", "weight": 0.5, "enabled": true }
        ],
        "createdAt": "2026-07-31T09:00:00Z",
        "updatedAt": "2026-07-31T09:30:00Z"
    }"#;

    #[test]
    fn node_upsert_accepts_what_the_webview_sends() {
        let node: Node = serde_json::from_str(NODE_FROM_THE_WEBVIEW).expect("node should decode");

        assert_eq!(node.kind, NodeKind::Species);
        assert_eq!(node.name, "Vashk");
        assert_eq!(node.parent_id, None);
        assert_eq!(node.notes_raw, "Notes typed in the editor.");
        assert_eq!(node.description_state, wobu_core::DescriptionState::Edited);
        assert_eq!(node.tags, ["playable"]);
        assert_eq!(node.links.len(), 1);
        assert_eq!(node.links[0].role, wobu_core::LinkRole::StyledBy);

        let description = node.description.as_ref().expect("description should decode");
        assert_eq!(description.text("silhouette"), Some("Long-limbed."));
        assert_eq!(
            description.list("materials"),
            Some(&["ashglass".to_string(), "bone".to_string()][..])
        );
    }

    #[test]
    fn a_node_serialises_back_under_the_keys_the_webview_reads() {
        let node: Node = serde_json::from_str(NODE_FROM_THE_WEBVIEW).unwrap();
        let json = serde_json::to_value(&node).unwrap();

        // The camelCase ones are the ones that would break silently: serde
        // renames them, TypeScript does not know that, and a missing key
        // arrives in the UI as `undefined` rather than as an error.
        for key in [
            "parentId",
            "notesRaw",
            "descriptionState",
            "coverAssetId",
            "assetLinks",
            "createdAt",
            "updatedAt",
        ] {
            assert!(json.get(key).is_some(), "`{key}` is missing from the node payload");
        }
        assert_eq!(json["links"][0]["toId"], "01ARZ3NDEKTSV4RRFFQ69G5FAW");
        assert_eq!(json["assetLinks"][0]["assetId"], "01ARZ3NDEKTSV4RRFFQ69G5FAX");
        assert_eq!(json["description"]["sections"]["materials"]["type"], "list");
    }

    #[test]
    fn asset_roles_are_the_strings_the_role_picker_sends_back() {
        // `role` crosses as a bare snake_case string, and a mismatch fails at
        // the bridge rather than at compile time — the picker would simply stop
        // working with nothing on screen to say why. `full_ref` is the one an
        // automatic rename would get wrong.
        for role in AssetRole::ALL {
            let json = serde_json::to_value(role).unwrap();
            assert_eq!(json.as_str().unwrap(), role.as_str());
            assert_eq!(
                serde_json::from_value::<AssetRole>(json).unwrap(),
                role,
                "{role} does not survive the bridge"
            );
        }
        assert_eq!(serde_json::from_str::<AssetRole>("\"full_ref\"").unwrap(), AssetRole::FullRef);
        assert!(serde_json::from_str::<AssetRole>("\"fullRef\"").is_err());
        assert!(serde_json::from_str::<AssetRole>("\"Mood\"").is_err());
    }

    #[test]
    fn an_asset_link_matches_the_assetlink_interface() {
        let link = wobu_core::asset::AssetRef::new(
            "01ARZ3NDEKTSV4RRFFQ69G5FAX".parse().unwrap(),
            AssetRole::Palette,
        );
        let json = serde_json::to_value(&link).unwrap();

        for key in ["assetId", "role", "weight", "enabled"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from AssetLink");
        }
        assert_eq!(json["role"], "palette");
        assert_eq!(json["assetId"], "01ARZ3NDEKTSV4RRFFQ69G5FAX");

        // The commands take a role and a weight as loose arguments rather than
        // a whole link, so this is also the shape `assetLink` posts back inside
        // a node — and a link that omits both must arrive at the documented
        // defaults rather than at zero.
        let bare: wobu_core::asset::AssetRef =
            serde_json::from_str(r#"{"assetId":"01ARZ3NDEKTSV4RRFFQ69G5FAX","role":"mood"}"#)
                .unwrap();
        assert_eq!(bare.weight, 1.0);
        assert!(bare.enabled);
    }

    #[test]
    fn asset_usage_matches_the_project_wide_library_interface() {
        let usage = AssetUsage {
            asset_id: "01ARZ3NDEKTSV4RRFFQ69G5FAX".parse().unwrap(),
            node_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
            node_name: "Vashk".into(),
            node_kind: NodeKind::Species,
            node_tags: vec!["playable".into()],
            roles: vec![AssetUsageRole { role: AssetRole::FullRef, weight: 0.8, enabled: true }],
            cover: true,
        };
        let json = serde_json::to_value(usage).unwrap();

        for key in ["assetId", "nodeId", "nodeName", "nodeKind", "nodeTags", "roles", "cover"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from AssetUsage");
        }
        assert_eq!(json["roles"][0]["role"], "full_ref");
        assert_eq!(json["nodeTags"][0], "playable");
    }

    #[test]
    fn node_link_roles_and_backlinks_match_the_relations_bridge() {
        for role in LinkRole::ALL {
            let json = serde_json::to_value(role).unwrap();
            assert_eq!(json.as_str(), Some(role.as_str()));
            assert_eq!(serde_json::from_value::<LinkRole>(json).unwrap(), role);
        }

        let edge = LinkEdge {
            from_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
            to_id: "01ARZ3NDEKTSV4RRFFQ69G5FAW".parse().unwrap(),
            role: LinkRole::MemberOf,
            weight: 0.4,
            enabled: true,
        };
        let json = serde_json::to_value(edge).unwrap();
        for key in ["fromId", "toId", "role", "weight", "enabled"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from LinkEdge");
        }
        assert_eq!(json["role"], "member_of");
    }

    #[test]
    fn log_info_matches_the_loginfo_interface() {
        let json = serde_json::to_value(log_info()).unwrap();
        for key in ["path", "level", "exists", "sizeBytes"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from LogInfo");
        }
    }

    #[test]
    fn log_levels_are_the_strings_the_level_buttons_send_back() {
        // The Settings buttons post these values straight back through
        // `log_set_level`, so a rename on either side silently stops working:
        // serde would reject the string and the level would never change.
        let levels: Vec<String> = [
            diag::Level::Off,
            diag::Level::Error,
            diag::Level::Warn,
            diag::Level::Info,
            diag::Level::Debug,
        ]
        .iter()
        .map(|l| serde_json::to_value(l).unwrap().as_str().unwrap().to_owned())
        .collect();

        assert_eq!(levels, ["off", "error", "warn", "info", "debug"]);
        // And back the other way, which is the direction the buttons use.
        assert_eq!(serde_json::from_str::<diag::Level>("\"debug\"").unwrap(), diag::Level::Debug);
    }

    #[test]
    fn a_peer_matches_the_peer_interface() {
        let peer = Peer {
            session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
            user: "nadia".into(),
            host: "nadia-mbp".into(),
            seen_secs_ago: 4,
            editing: vec!["01ARZ3NDEKTSV4RRFFQ69G5FAW".parse().unwrap()],
        };
        let json = serde_json::to_value(&peer).unwrap();

        for key in ["sessionId", "user", "host", "seenSecsAgo", "editing"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from Peer");
        }
        // Ids are strings on the far side; a ULID that serialised as an object
        // or a number would arrive as a key nothing in the navigator matches.
        assert_eq!(json["sessionId"], "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(json["editing"][0], "01ARZ3NDEKTSV4RRFFQ69G5FAW");
    }

    #[test]
    fn a_queue_snapshot_matches_the_queuesnapshot_interface() {
        // `job:state` carries this on every transition and `job_list` returns
        // it, so the status bar (#56) reads it two ways and neither is
        // generated from the other side.
        let snapshot = wobu_jobs::QueueSnapshot {
            jobs: vec![wobu_jobs::JobSnapshot {
                id: wobu_jobs::JobId::new(),
                kind: wobu_jobs::JobKind::Enhance,
                label: "Enhance Vashk".into(),
                subject_id: Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".into()),
                state: wobu_jobs::JobState::Running,
                attempt: 1,
                elapsed_ms: 4200,
            }],
            queued: 2,
            running: 1,
            retrying: 0,
        };
        let json = serde_json::to_value(&snapshot).unwrap();

        for key in ["jobs", "queued", "running", "retrying"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from QueueSnapshot");
        }
        // The state is flattened into the job rather than nested, because the
        // TypeScript is a discriminated union on one field. A `{ state: { … } }`
        // here would draw every job as unknown and no Rust test inside
        // `wobu-jobs` would notice which side was wrong.
        let job = &json["jobs"][0];
        assert_eq!(job["state"], "running");
        assert_eq!(job["kind"], "enhance");
        assert!(job["id"].is_string(), "a job id must cross as a string");
        for key in ["id", "kind", "label", "subjectId", "attempt", "elapsedMs"] {
            assert!(job.get(key).is_some(), "`{key}` is missing from JobSnapshot");
        }
    }

    #[test]
    fn a_held_retry_reaches_the_webview_as_the_offer_it_is() {
        // `retryHeld` and `billed` are the whole of the "never auto-retry
        // something that costs money" design as the user experiences it: they
        // are what turns a failure into "try again — it will cost you". Dropped
        // on the wire, the UI has no way to tell a dead end from a question.
        let failure =
            wobu_jobs::Failure::new("provider.bad_response", "The response was cut short.")
                .retryable(true)
                .billed(wobu_jobs::Billed::Charged)
                .cost_note("812 in + 400 out");
        let state = wobu_jobs::JobState::Failed { failure, retry_held: true };
        let json = serde_json::to_value(&state).unwrap();

        assert_eq!(json["state"], "failed");
        assert_eq!(json["retryHeld"], true);
        assert_eq!(json["failure"]["billed"], "charged");
        assert_eq!(json["failure"]["costNote"], "812 in + 400 out");
        assert_eq!(json["failure"]["retryable"], true);
        // The same dotted codes command errors use, so `errorSurface` in
        // `src/lib/api.ts` can be pointed at either without a second taxonomy.
        assert_eq!(json["failure"]["code"], Code::ProviderBadResponse.as_str());
    }

    #[test]
    fn presence_editing_accepts_the_node_ids_the_webview_sends() {
        // `presenceEditing` posts a bare array of id strings. Anything else on
        // this side and the call fails silently at the bridge, leaving the
        // editing list frozen on whatever node was open first.
        let ids: Vec<Id> =
            serde_json::from_str(r#"["01ARZ3NDEKTSV4RRFFQ69G5FAV"]"#).expect("ids should decode");
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn a_conflict_matches_the_conflict_interface() {
        let conflict = Conflict {
            rel_path: "nodes/character/kael.conflict-nadia-20260731T142211Z.md".into(),
            node_rel_path: "nodes/character/kael.md".into(),
            node_id: Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()),
            node_name: Some("Kael Vantris".into()),
            user: Some("nadia".into()),
            saved_at: Some("2026-07-31T14:22:11Z".parse().unwrap()),
            mine: false,
            parked: "hers".into(),
            current: "ours".into(),
            current_hash: "abc123".into(),
        };
        let json = serde_json::to_value(&conflict).unwrap();

        for key in [
            "relPath",
            "nodeRelPath",
            "nodeId",
            "nodeName",
            "user",
            "savedAt",
            "mine",
            "parked",
            "current",
            "currentHash",
        ] {
            assert!(json.get(key).is_some(), "`{key}` is missing from Conflict");
        }
        // The card renders a time from this, so it has to arrive as something
        // `new Date()` understands rather than as a serde struct.
        assert_eq!(json["savedAt"], "2026-07-31T14:22:11Z");
        assert_eq!(json["nodeId"], "01ARZ3NDEKTSV4RRFFQ69G5FAV");
    }

    #[test]
    fn conflict_resolve_accepts_the_choice_the_buttons_send() {
        // `keep` is a bare string on the wire. Anything else on this side and
        // the buttons fail at the bridge rather than at compile time.
        assert_eq!(serde_json::from_str::<Keep>("\"parked\"").unwrap(), Keep::Parked);
        assert_eq!(serde_json::from_str::<Keep>("\"current\"").unwrap(), Keep::Current);
        assert!(serde_json::from_str::<Keep>("\"mine\"").is_err());
    }

    #[test]
    fn an_import_matches_the_importedasset_interface() {
        let imported = ImportedAsset {
            asset: Asset {
                id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
                hash: "a3f9c1d2e4b5a6978081726354453627a3f9c1d2e4b5a6978081726354453627".into(),
                kind: AssetKind::Reference,
                rel_path: "assets/originals/a3/a3f9c1.png".into(),
                thumb_path: None,
                mime: "image/png".into(),
                width: 1024,
                height: 768,
                bytes: 240_512,
                created_at: "2026-07-31T09:00:00Z".parse().unwrap(),
            },
            deduped: true,
            warnings: vec![ImportWarning::MeshTooSmall],
        };
        let json = serde_json::to_value(&imported).unwrap();

        assert!(json.get("deduped").is_some(), "`deduped` is missing from ImportedAsset");
        // The library card puts these next to the thumbnail, so they arrive as
        // the snake_case tags the far side switches on rather than as prose —
        // the wording is `ImportWarning::label`'s to change.
        assert_eq!(json["warnings"], serde_json::json!(["mesh_too_small"]));
        for key in [
            "id",
            "hash",
            "kind",
            "relPath",
            "thumbPath",
            "mime",
            "width",
            "height",
            "bytes",
            "createdAt",
        ] {
            assert!(json["asset"].get(key).is_some(), "`{key}` is missing from Asset");
        }
        // The id is the handle a node's `coverAssetId` and every AssetLink
        // carries, so it has to arrive as a plain ULID string.
        assert_eq!(json["asset"]["id"], "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        // A thumbnail nothing has made yet is `null`, not an absent key —
        // `thumbPath: string | null` on the far side.
        assert!(json["asset"]["thumbPath"].is_null());
    }

    #[test]
    fn asset_import_accepts_the_kind_the_webview_sends() {
        // `kind` is a bare snake_case string on the wire. A mismatch fails at
        // the bridge rather than at compile time, and every drop would be
        // rejected with nothing on screen to say why.
        assert_eq!(
            serde_json::from_str::<AssetKind>("\"reference\"").unwrap(),
            AssetKind::Reference
        );
        assert_eq!(
            serde_json::from_str::<AssetKind>("\"generated\"").unwrap(),
            AssetKind::Generated
        );
        assert_eq!(serde_json::from_str::<AssetKind>("\"upload\"").unwrap(), AssetKind::Upload);
        assert!(serde_json::from_str::<AssetKind>("\"Reference\"").is_err());
    }

    #[test]
    fn the_kind_registry_matches_the_kinddef_interface() {
        let json = serde_json::to_value(kind_registry()).unwrap();
        let first = &json[0];

        for key in [
            "kind",
            "label",
            "plural",
            "icon",
            "color",
            "layer",
            "dir",
            "nests",
            "singleton",
            "attributes",
            "sections",
            "defaultLinkRoles",
        ] {
            assert!(first.get(key).is_some(), "`{key}` is missing from KindDef");
        }
        for key in ["key", "label", "valueKind"] {
            assert!(first["sections"][0].get(key).is_some(), "`{key}` is missing from SectionDef");
        }
        let world = json.as_array().unwrap().iter().find(|d| d["kind"] == "world_bible").unwrap();
        for key in ["key", "label", "valueKind"] {
            assert!(
                world["attributes"][0].get(key).is_some(),
                "`{key}` is missing from AttributeDef"
            );
        }
        // The union in `api.ts` is snake_case; the enum has to agree.
        let kinds: Vec<&str> =
            json.as_array().unwrap().iter().map(|d| d["kind"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"style_guide"), "got {kinds:?}");
        assert!(kinds.contains(&"world_bible"), "got {kinds:?}");
    }

    #[test]
    fn the_three_capabilities_cross_as_the_strings_the_pane_sends_back() {
        // The Settings pane posts one of these on every provider change, and a
        // rename on either side is a dropdown that silently stops working:
        // serde refuses the string and nothing is ever written.
        for (capability, wire) in
            [(Capability::Text, "text"), (Capability::Image, "image"), (Capability::Mesh, "mesh")]
        {
            assert_eq!(serde_json::to_value(capability).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<Capability>(serde_json::json!(wire)).unwrap(),
                capability,
            );
            // And the key in `project.json` is the same string, which is the
            // half `enhance.rs` reads.
            assert_eq!(capability.key(), wire);
        }
        assert!(serde_json::from_str::<Capability>("\"Text\"").is_err());
    }

    #[test]
    fn only_the_three_hunyuan_regions_can_cross_the_command_boundary() {
        for region in ["ap-singapore", "na-siliconvalley", "eu-frankfurt"] {
            assert_eq!(
                provider_region(Capability::Mesh, "hunyuan3d", Some(region.into())).unwrap(),
                Some(region.into()),
            );
        }
        assert!(
            provider_region(Capability::Mesh, "hunyuan3d", Some("ap-guangzhou".into())).is_err()
        );
        assert!(provider_region(Capability::Image, "gemini", Some("eu-frankfurt".into())).is_err());
        assert_eq!(provider_region(Capability::Mesh, "hunyuan3d", None).unwrap(), None);
    }

    #[test]
    fn a_probe_result_matches_the_proberesult_interface() {
        let result = ProbeResult {
            provider: "anthropic".into(),
            model: "claude-sonnet-5".into(),
            ok: false,
            message: "Anthropic rejected the API key".into(),
            code: Some("provider.bad_key".into()),
            usage: Usage { input_tokens: 412, cached_input_tokens: 0, output_tokens: 24 },
        };
        let json = serde_json::to_value(&result).unwrap();

        for key in ["provider", "model", "ok", "message", "code", "usage"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from ProbeResult");
        }
        // The usage fields are camelCase on the wire and the pane prints them
        // as what the check cost; an absent one reads as zero and understates
        // the bill.
        for key in ["inputTokens", "cachedInputTokens", "outputTokens"] {
            assert!(json["usage"].get(key).is_some(), "`{key}` is missing from the probe usage");
        }
        assert_eq!(json["code"], "provider.bad_key");
    }

    #[test]
    fn status_bar_models_use_the_same_defaults_and_only_known_context_windows() {
        assert_eq!(text_default(anthropic::ID), anthropic::DEFAULT_MODEL);
        assert_eq!(text_default(gemini::ID), gemini::DEFAULT_MODEL);
        assert_eq!(image_default(comfy::ID), comfy::DEFAULT_MODEL);
        assert_eq!(image_default(image_gemini::ID), image_gemini::DEFAULT_MODEL);
        assert_eq!(context_window(anthropic::ID, "claude-haiku-4-5"), Some(200_000));
        assert_eq!(context_window(anthropic::ID, "future-model"), None);
    }

    #[test]
    fn status_bar_health_matches_the_frontend_union() {
        let status = StatusBarBackend {
            image: Some(ActiveModel {
                provider: comfy::ID.into(),
                label: comfy::LABEL.into(),
                model: "flux-dev".into(),
                context_tokens: None,
            }),
            text: ActiveModel {
                provider: anthropic::ID.into(),
                label: anthropic::LABEL.into(),
                model: anthropic::DEFAULT_MODEL.into(),
                context_tokens: Some(1_000_000),
            },
            health: BackendHealth::Connected { external_queue: Some(2) },
        };
        let json = serde_json::to_value(status).unwrap();
        assert_eq!(json["health"]["state"], "connected");
        assert_eq!(json["health"]["externalQueue"], 2);
        assert_eq!(json["text"]["contextTokens"], 1_000_000);
        assert_eq!(json["image"]["model"], "flux-dev");
    }

    #[test]
    fn a_failed_probe_is_an_answer_rather_than_a_rejection() {
        // The regression: routing a rejected key through the command's `Err`
        // channel would put it in a toast, away from the field that caused it,
        // and the pane would have nothing to disable. Every "the call" failure
        // has to arrive as `ok: false` with a code the pane can style.
        let outcome = EnhanceOutcome::unbilled(wobu_llm::Error::BadKey { provider: "Anthropic" });
        let rejected = verdict(&ProbeAdapter, "claude-sonnet-5".into(), outcome);
        assert!(!rejected.ok);
        assert_eq!(rejected.code.as_deref(), Some("provider.bad_key"));
        assert_eq!(rejected.usage, Usage::default());

        // And the other half: an answer cut off by the token ceiling is what a
        // *passing* probe looks like, because everything the check set out to
        // establish was already settled by the time the provider started
        // writing. Reading it as a failure would report every good key as bad.
        let truncated = EnhanceOutcome::new(
            Usage { input_tokens: 400, cached_input_tokens: 0, output_tokens: 24 },
            Err(wobu_llm::Error::Truncated),
        );
        let passed = verdict(&ProbeAdapter, "claude-sonnet-5".into(), truncated);
        assert!(passed.ok, "{}", passed.message);
        assert_eq!(passed.code, None);
        assert_eq!(passed.usage.output_tokens, 24, "the pane says what the check cost");
    }

    /// Stands in for whichever adapter the probe built. Only the three name
    /// methods are reached — [`verdict`] never calls one.
    struct ProbeAdapter;

    #[async_trait::async_trait]
    impl TextProvider for ProbeAdapter {
        fn id(&self) -> &'static str {
            anthropic::ID
        }
        fn label(&self) -> &'static str {
            anthropic::LABEL
        }
        fn default_model(&self) -> &'static str {
            "claude-sonnet-5"
        }
        async fn enhance(
            &self,
            _request: &EnhanceRequest,
            _deltas: &mut dyn wobu_llm::DeltaSink,
            _cancel: &Cancel,
        ) -> EnhanceOutcome {
            unreachable!("the verdict tests never make a call")
        }
    }

    /* ── the shared selection ─────────────────────────────────────────────── */

    #[test]
    fn writing_a_selection_leaves_the_rest_of_project_json_alone() {
        // `project.json` is shared across a drive, so this file is written by
        // builds of different vintages. Re-serialising `ProjectMeta` would drop
        // every field this build has never heard of — including a fourth
        // capability under `providers` — and the loss would be invisible until
        // the collaborator who set it noticed their world had changed.
        let root = std::env::temp_dir().join(format!("wobu-providers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("project.json");
        std::fs::write(
            &path,
            r#"{
              "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
              "name": "Ashfall",
              "schemaVersion": 1,
              "createdAt": "2026-07-31T09:00:00Z",
              "somethingANewerBuildWrote": { "keep": "me" },
              "providers": { "image": { "provider": "comfyui" } }
            }"#,
        )
        .unwrap();

        let mut providers = serde_json::Map::new();
        providers.insert("image".to_owned(), serde_json::json!({ "provider": "comfyui" }));
        providers.insert(
            "text".to_owned(),
            serde_json::json!({ "provider": "gemini", "model": "gemini-3.6-flash" }),
        );
        write_providers(&root, &providers).unwrap();

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["somethingANewerBuildWrote"]["keep"], "me");
        assert_eq!(written["name"], "Ashfall");
        assert_eq!(written["providers"]["text"]["provider"], "gemini");
        // The other capabilities are untouched: three selections are three
        // independent choices, and setting one must not clear the others.
        assert_eq!(written["providers"]["image"]["provider"], "comfyui");

        // Nothing is left in staging — a `.part` beside a project is litter that
        // replicates to everyone on the share.
        assert!(!root.join(".wobu/tmp/project.json.part").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// The two influence commands, over a world built here rather than on disk.
///
/// Everything below calls [`resolved`] and [`compiled`] — the exact functions
/// the commands call once they have the nodes — so what is asserted is the
/// payload the webview receives. Where the project folder and the index come
/// into it is `wobu-store`'s `world.rs`, which is the other half of this.
#[cfg(test)]
mod influence {
    use super::*;
    use wobu_core::asset::AssetRef;
    use wobu_core::{Description, Link, LinkRole, SectionValue};

    /// A style guide, a culture the subject belongs to, and the subject — the
    /// smallest world with more than one layer in it.
    struct Ashfall {
        nodes: Vec<Node>,
        kael: Id,
        mood: Id,
    }

    fn ashfall() -> Ashfall {
        let mut style = Node::new(NodeKind::StyleGuide, "Ashfall House Style").unwrap();
        style.description = Some(Description::from_sections([(
            "rendering".to_string(),
            SectionValue::Text("Ash-dusted, matte, hand-painted".into()),
        )]));

        let mut guild = Node::new(NodeKind::Culture, "Cinder Guild").unwrap();
        guild.description = Some(Description::from_sections([(
            "costume".to_string(),
            SectionValue::Text("Ash-grey longcoats, brass fastenings".into()),
        )]));

        let mut kael = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
        kael.links = vec![Link::new(guild.id, LinkRole::MemberOf)];
        kael.description = Some(Description::from_sections([
            ("silhouette".to_string(), SectionValue::Text("Tall, narrow, hooded".into())),
            (
                "never".to_string(),
                SectionValue::List(vec!["modern firearms".into(), "neon".into()]),
            ),
        ]));

        // One reference the compiler may send and one it may not. The mood board
        // is the whole reason the second exists.
        let mood = wobu_core::new_id();
        kael.asset_links = vec![
            AssetRef::new(wobu_core::new_id(), AssetRole::Palette),
            AssetRef::new(mood, AssetRole::Mood),
        ];

        let (kael_id, nodes) = (kael.id, vec![style, guild, kael]);
        Ashfall { nodes, kael: kael_id, mood }
    }

    fn stack(world: &Ashfall) -> InfluenceStack {
        resolved(&world.nodes, world.kael, None, &Sliders::neutral(), None).unwrap()
    }

    fn prompt(world: &Ashfall) -> CompiledPrompt {
        compiled(&world.nodes, world.kael, None, &Sliders::neutral(), None, Budget::unlimited())
            .unwrap()
    }

    #[test]
    fn a_layer_card_matches_the_layercard_interface() {
        // Hand-written TypeScript on the far side, so a serde rename nothing
        // noticed arrives in the panel as `undefined` rather than as an error.
        let json = serde_json::to_value(stack(&ashfall())).unwrap();

        for key in ["subjectId", "preset", "layers"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from InfluenceStack");
        }
        let card = &json["layers"][0];
        for key in [
            "layer",
            "nodeId",
            "name",
            "kind",
            "reached",
            "distance",
            "weight",
            "slider",
            "fragments",
        ] {
            assert!(card.get(key).is_some(), "`{key}` is missing from LayerCard");
        }
        // Layer 1 first, and the enums in the snake_case the unions in `api.ts`
        // are written in.
        assert_eq!(card["layer"], "style");
        assert_eq!(card["kind"], "style_guide");
        assert_eq!(card["reached"], "root");
        assert_eq!(json["preset"]["id"], "character_sheet");
    }

    #[test]
    fn a_fragment_matches_the_influencefragment_interface() {
        let json = serde_json::to_value(stack(&ashfall())).unwrap();
        let fragment = &json["layers"][0]["fragments"][0];

        for key in [
            "layer",
            "nodeId",
            "sourceName",
            "section",
            "text",
            "assetId",
            "weight",
            "target",
            "sendable",
        ] {
            assert!(fragment.get(key).is_some(), "`{key}` is missing from InfluenceFragment");
        }
        assert_eq!(fragment["section"], "rendering");
        assert_eq!(fragment["target"], "prompt");
        // Prose and picture are exclusive, and the unused one is `null` rather
        // than an absent key — `text: string | null` on the far side.
        assert!(fragment["assetId"].is_null());
    }

    #[test]
    fn a_compiled_prompt_matches_the_compiledprompt_interface() {
        let world = ashfall();
        let cramped = compiled(
            &world.nodes,
            world.kael,
            None,
            &Sliders::neutral(),
            None,
            Budget { prompt: Chars::new(40), negative: Chars::UNLIMITED },
        )
        .unwrap();
        let json = serde_json::to_value(cramped).unwrap();

        for key in ["subjectId", "preset", "prompt", "negative", "spans", "dropped", "overflow"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from CompiledPrompt");
        }
        // The drop report nests the fragment exactly as `wobu_influence::Dropped`
        // does, so the panel renders a casualty with the same component it
        // renders a span with.
        let dropped = &json["dropped"][0];
        assert!(dropped["fragment"].get("sourceName").is_some());
        assert!(["silenced", "budget"].contains(&dropped["reason"].as_str().unwrap()));
        // A prompt that fits reports no overflow — `number | null`, not absent.
        assert!(serde_json::to_value(prompt(&world)).unwrap()["overflow"].is_null());
    }

    #[test]
    fn a_mood_reference_reaches_the_layer_card_and_nothing_a_backend_would_see() {
        // The privacy property at the bridge, which is the last place it can be
        // lost. #26, #42 and #43 each preserved it; a command that put the whole
        // fragment list in its response would undo all three, and the failure
        // would be somebody's mood board arriving at a third party.
        let world = ashfall();
        let cards = serde_json::to_value(stack(&world)).unwrap();
        let mood = world.mood.to_string();

        // Visible to the panel, which is the point of attaching it: the card
        // counts it, and says it must not be sent.
        let fragments = cards["layers"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| c["fragments"].as_array().unwrap().iter());
        let shown = fragments
            .filter(|f| f["assetId"] == mood)
            .inspect(|f| {
                assert_eq!(f["target"], "moodboard_only");
                assert_eq!(f["sendable"], false);
            })
            .count();
        assert_eq!(shown, 1, "the mood reference should be on its layer card");

        // And absent from every part of what a generation would be built from —
        // asserted over the whole serialised payload rather than field by field,
        // so a field added later is covered without anyone remembering to.
        let compiled = serde_json::to_string(&prompt(&world)).unwrap();
        assert!(!compiled.contains(&mood), "the mood reference crossed the bridge in {compiled}");
        assert!(!compiled.contains("moodboard_only"), "got {compiled}");
    }

    #[test]
    fn the_spans_are_exactly_what_is_in_the_two_prompts() {
        // The attribution trail has to describe the string beside it, or the
        // tinting points at the wrong words. Derived from the drop report rather
        // than re-compiled (`prompt_spans`), so this is what catches the two
        // going out of step.
        let world = ashfall();
        let compiled = prompt(&world);
        let joined = |target: &str| {
            compiled
                .spans
                .iter()
                .filter(|s| serde_json::to_value(s.target).unwrap() == target)
                .map(|s| s.text.clone().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(", ")
        };

        assert_eq!(joined("prompt"), compiled.prompt);
        assert_eq!(joined("negative"), compiled.negative);
        assert!(compiled.negative.contains("modern firearms"), "got {}", compiled.negative);
        // The subject reads last, ahead of only the framing, where a text
        // encoder's recency bias does the most good.
        assert!(compiled.prompt.starts_with("Ash-dusted"), "got {}", compiled.prompt);
        assert!(compiled.prompt.ends_with("single subject"), "got {}", compiled.prompt);
    }

    #[test]
    fn a_subject_nobody_has_heard_of_is_a_missing_node_rather_than_a_panic() {
        // The ordinary cause is a tab still pointing at something a collaborator
        // deleted, which the frontend already knows how to handle — but only if
        // it arrives under the code it handles.
        let world = ashfall();
        let ghost = wobu_core::new_id();
        for error in [
            resolved(&world.nodes, ghost, None, &Sliders::neutral(), None).unwrap_err(),
            compiled(&world.nodes, ghost, None, &Sliders::neutral(), None, Budget::unlimited())
                .unwrap_err(),
        ] {
            assert_eq!(serde_json::to_value(&error).unwrap()["code"], "node.not_found");
        }
    }

    #[test]
    fn a_world_with_no_style_guide_and_no_links_resolves_to_a_short_stack() {
        // Every project between `project_create` and the user writing anything
        // is in some version of this state, and the Inspector is on screen for
        // all of it. A thin stack is an answer; an error is not.
        let lonely = Node::new(NodeKind::Prop, "Ash Lantern").unwrap();
        let (id, nodes) = (lonely.id, vec![lonely]);

        let stack = resolved(&nodes, id, None, &Sliders::neutral(), None).unwrap();
        assert_eq!(stack.layers.len(), 1, "the subject and nothing else");
        assert_eq!(stack.layers[0].layer, Layer::Subject);
        assert!(stack.layers[0].fragments.is_empty(), "nothing has been described yet");

        // And it compiles: the framing text is the preset's, so a subject with no
        // description of its own still has a prompt rather than an empty string.
        let compiled =
            compiled(&nodes, id, None, &Sliders::neutral(), None, Budget::unlimited()).unwrap();
        assert_eq!(compiled.preset.id, "prop_orthographic");
        assert!(compiled.prompt.starts_with("orthographic elevation"), "got {}", compiled.prompt);
        assert_eq!(compiled.negative, "");
        assert!(compiled.dropped.is_empty());
    }

    #[test]
    fn an_empty_world_answers_rather_than_panicking() {
        // `World::new` over nothing, which is what a project whose index has not
        // been built yet looks like. It cannot produce a stack, but it must
        // produce the same error as any other missing subject.
        let error =
            resolved(&[], wobu_core::new_id(), None, &Sliders::neutral(), None).unwrap_err();
        assert_eq!(serde_json::to_value(&error).unwrap()["code"], "node.not_found");
    }

    #[test]
    fn a_preset_the_registry_has_never_heard_of_falls_back_to_the_kind_default() {
        // `Generation.preset` is a string that outlives any one build, so a
        // snapshot naming a preset since renamed has to compile to something.
        // Refusing would take the panel down for a record it is trying to show.
        let world = ashfall();
        let sliders = Sliders::neutral();
        let unknown = compiled(
            &world.nodes,
            world.kael,
            Some("silhouette_study"),
            &sliders,
            None,
            Budget::unlimited(),
        )
        .unwrap();
        assert_eq!(unknown.preset.id, "character_sheet");

        // A preset the registry *does* know reweights the same fragments — the
        // costume plate lifts `costume` and all but silences `silhouette`.
        let plate = compiled(
            &world.nodes,
            world.kael,
            Some("costume_plate"),
            &sliders,
            None,
            Budget::unlimited(),
        )
        .unwrap();
        let weight = |c: &CompiledPrompt, section: &str| {
            c.spans.iter().find(|s| s.section == section).map(|s| s.weight)
        };
        assert!(weight(&plate, "costume") > weight(&unknown, "costume"));
        assert!(weight(&plate, "silhouette") < weight(&unknown, "silhouette"));
    }

    #[test]
    fn a_card_turned_down_to_nothing_keeps_its_rows_and_loses_its_words() {
        // The difference between "you turned this off" and "your notes are gone".
        // The panel that exists to explain the prompt must not answer the second
        // when the user did the first, so the fragments stay on the card and the
        // drop report says `silenced` rather than `budget`.
        let world = ashfall();
        let guild = world.nodes.iter().find(|n| n.kind == NodeKind::Culture).unwrap().id;
        let sliders = Sliders::from_pairs([(guild, 0.0)]);

        let stack = resolved(&world.nodes, world.kael, None, &sliders, None).unwrap();
        let card = stack.layers.iter().find(|c| c.node_id == Some(guild)).unwrap();
        assert_eq!(card.slider, 0.0);
        assert_eq!(card.fragments.len(), 1, "the longcoat is still on the card");

        let compiled =
            compiled(&world.nodes, world.kael, None, &sliders, None, Budget::unlimited()).unwrap();
        assert!(!compiled.prompt.contains("longcoat"), "got {}", compiled.prompt);
        let silenced: Vec<&str> = compiled
            .dropped
            .iter()
            .filter(|d| d.reason == DropReason::Silenced)
            .map(|d| d.fragment.section)
            .collect();
        assert_eq!(silenced, ["costume"]);
    }

    #[test]
    fn the_shot_card_is_only_there_once_a_shot_has_been_set_up() {
        // `influence_resolve` is the display path and has no shot until the panel
        // gives it one; `prompt_compile` always has one, because the framing text
        // is part of the prompt a generation would send. Getting this backwards
        // means the box shows a prompt the Generate button would not produce.
        let world = ashfall();
        assert!(!stack(&world).layers.iter().any(|c| c.layer == Layer::Shot));

        let controls = ShotControls {
            label: Some("Turnaround".into()),
            weight: Some(0.5),
            prompt: Some("at dusk in falling ash".into()),
        };
        let framed =
            resolved(&world.nodes, world.kael, None, &Sliders::neutral(), Some(&controls)).unwrap();
        let shot = framed.layers.last().unwrap();
        assert_eq!(shot.layer, Layer::Shot);
        assert_eq!(shot.name, "Turnaround");
        assert_eq!(shot.node_id, None, "the shot is not a node");
        assert_eq!(shot.weight, 0.5);
        assert!(
            shot.fragments.iter().any(|fragment| {
                fragment.section == "user_prompt"
                    && fragment.text.as_deref() == Some("at dusk in falling ash")
            }),
            "the extra shot prompt must be an exact attributed contribution",
        );

        let custom = compiled(
            &world.nodes,
            world.kael,
            None,
            &Sliders::neutral(),
            Some(&controls),
            Budget::unlimited(),
        )
        .unwrap();
        assert!(custom.prompt.ends_with("at dusk in falling ash"));

        // And the compiled prompt carries the preset's framing whether or not
        // anyone named the shot.
        assert!(prompt(&world).prompt.ends_with("single subject"));
    }

    #[test]
    fn a_budget_that_bites_reports_what_it_cut_instead_of_truncating() {
        // The acceptance criterion for the whole command: a caller that only
        // received the string could not tell a prompt that fitted from one that
        // had been quietly cut in half.
        let world = ashfall();
        let cramped = compiled(
            &world.nodes,
            world.kael,
            None,
            &Sliders::neutral(),
            None,
            Budget { prompt: Chars::new(40), negative: Chars::new(0) },
        )
        .unwrap();

        assert!(cramped.prompt.chars().count() <= 40, "got {}", cramped.prompt);
        assert_eq!(cramped.negative, "", "the negatives are emptied rather than overrun");
        let cut: Vec<(&str, DropReason)> =
            cramped.dropped.iter().map(|d| (d.fragment.section, d.reason)).collect();
        assert!(cut.contains(&("never", DropReason::Budget)), "got {cut:?}");
        assert!(cut.iter().all(|(_, reason)| *reason == DropReason::Budget), "got {cut:?}");
        // Everything that survived is still attributed, so the panel can say
        // which layer paid for the ones that did not.
        assert!(cramped.spans.iter().all(|s| !s.source_name.is_empty()));
    }

    #[test]
    fn the_sliders_the_panel_sends_arrive_as_the_weights_it_asked_for() {
        // `sliders` crosses as an array of `{ nodeId, value }`. A rename on
        // either side would fail at the bridge, and every drag would go nowhere
        // with nothing on screen to say why.
        let settings: Vec<SliderSetting> =
            serde_json::from_str(r#"[{"nodeId":"01ARZ3NDEKTSV4RRFFQ69G5FAV","value":0.25}]"#)
                .expect("sliders should decode");
        let id: Id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        assert_eq!(sliders_from(Some(settings)).get(id), 0.25);
        // Out of range is clamped rather than refused — the control's range is
        // the engine's business, not the panel's.
        assert_eq!(sliders_from(None).get(id), 1.0, "an untouched card is at full weight");
    }

    #[test]
    fn an_absent_budget_is_unlimited_and_a_partial_one_binds_only_its_own_pool() {
        // No backend has been chosen yet, so there is no real limit to apply and
        // inventing one would drop fragments to fit a number nobody measured.
        assert_eq!(budget_from(None), Budget::unlimited());

        let partial: PromptBudget =
            serde_json::from_str(r#"{"promptChars":900}"#).expect("budget should decode");
        let budget = budget_from(Some(partial));
        assert_eq!(budget.prompt, Chars::new(900));
        assert_eq!(budget.negative, Chars::UNLIMITED, "the two pools are metered separately");
    }
}
