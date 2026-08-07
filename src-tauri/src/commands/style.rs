//! Applying one node's look to another.
//!
//! Two commands rather than one: the preview says what would change and the
//! apply does it, so the sheet can show the diff before the user commits to it.

use std::path::PathBuf;

use tauri::{AppHandle, Emitter, State};
// Aliased because the command below has to *be* called `kind_registry` —
// Tauri v2 derives the invoke name from the function name, with no rename.
use wobu_core::Id;
use wobu_store::{TransferOutcome, TransferPreview, transfer};

use crate::error::{Code, CommandResult, WobuError};
use crate::state::{AppState, WORLD_CHANGED};

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
