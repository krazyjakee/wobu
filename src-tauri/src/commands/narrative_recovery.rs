//! Recover retained canonical deletions without exposing their potentially large source bytes.
use crate::{error::CommandResult, state::AppState};
use serde::Serialize;
use tauri::State;
use wobu_core::Id;
use wobu_store::{Project, SourceSave};

#[derive(Debug, Serialize)]
pub struct RetainedDeletion {
    id: Id,
    name: String,
    target: String,
    hash: String,
    restored: bool,
}
fn list(project: &Project) -> CommandResult<Vec<RetainedDeletion>> {
    Ok(project
        .narrative_deletions()?
        .into_iter()
        .map(|view| RetainedDeletion {
            id: view.deletion.id,
            name: view.deletion.name,
            target: view.deletion.target,
            hash: view.deletion.hash,
            restored: view.restored,
        })
        .collect())
}
#[tauri::command]
pub fn narrative_recovery_list(state: State<'_, AppState>) -> CommandResult<Vec<RetainedDeletion>> {
    state.with(|project| list(project))
}
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Restoration {
    Saved {
        id: Id,
    },
    Conflict {
        id: Id,
        #[serde(rename = "conflictPath")]
        conflict_path: String,
    },
}
fn restore(project: &mut Project, id: Id) -> CommandResult<Restoration> {
    Ok(match project.restore_narrative_deletion(id)? {
        SourceSave::Saved(_) => Restoration::Saved { id },
        SourceSave::Conflict { conflict_path } => Restoration::Conflict { id, conflict_path },
    })
}
#[tauri::command]
pub fn narrative_recovery_restore(
    state: State<'_, AppState>,
    id: Id,
) -> CommandResult<Restoration> {
    state.with(|project| restore(project, id))
}
#[cfg(test)]
mod tests;
