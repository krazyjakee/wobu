//! Importing images, and the links that say what one is for.
//!
//! The chunked-transfer commands exist because the webview cannot hand Tauri a
//! path: a drag-and-drop gives it bytes, and a 500 MB PSD sent as one IPC
//! payload is a 500 MB string in the webview and another in Rust. Every
//! transfer is staged to a `.part` file under the project and either committed
//! whole or removed, so a cancelled or crashed import leaves no half an asset.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::Serialize;
use tauri::State;
use tauri::ipc::{InvokeBody, Request};
// Aliased because the command below has to *be* called `kind_registry` —
// Tauri v2 derives the invoke name from the function name, with no rename.
use wobu_core::{Asset, AssetKind, AssetRole, Id, Node};
use wobu_store::{AssetUsage, ImportedAsset};

use super::{absolute, blocking, saved};
use crate::diag;
use crate::error::{Code, CommandResult, WobuError};
use crate::state::{AppState, ProjectTicket};

pub(super) const ASSET_TRANSFER_CHUNK_BYTES: usize = 1024 * 1024;

const ASSET_TRANSFER_MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Default)]
pub struct AssetTransfers(pub(super) Mutex<HashMap<String, AssetTransfer>>);

pub(super) struct AssetTransfer {
    pub(super) project: ProjectTicket,
    pub(super) path: PathBuf,
    pub(super) file: File,
    pub(super) kind: AssetKind,
    pub(super) received_bytes: u64,
    pub(super) total_bytes: u64,
}

impl AssetTransfers {
    fn take(&self, id: &str) -> Option<AssetTransfer> {
        self.0.lock().remove(id)
    }

    pub(super) fn cancel(&self, id: &str) -> bool {
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
    pub(super) received_bytes: u64,
    total_bytes: u64,
}

/* ── registry ─────────────────────────────────────────────────────────────── */

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

pub(super) fn append_asset_transfer_chunk(
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

pub(super) fn import_file_unlocked_with(
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
pub(super) fn ensure_thumb_unlocked(
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
