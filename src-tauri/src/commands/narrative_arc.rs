use crate::{error::CommandResult, state::AppState};
use tauri::{AppHandle, Manager};
use wobu_store::project::narrative_arc::ArcGraph;

/// Content checking the full catalog stays off the desktop event thread.
#[tauri::command]
pub async fn narrative_arc(app: AppHandle) -> CommandResult<ArcGraph> {
    super::blocking("The arc projection thread stopped unexpectedly.", move || {
        app.state::<AppState>().with(|project| Ok(project.narrative_arc()?))
    })
    .await?
}
