//! Localisation authoring commands; all row decisions belong to the guarded store.
use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};
use tauri::State;
use wobu_narrative_locale::{LocaleId, Policy, Row};
use wobu_store::{
    Project, SourceSave,
    project::narrative_locale::{ImportReport, LocaleView},
};
/// Capture the session before queueing work; neither reconciliation nor publication
/// may follow the window into another project while the blocking pool is busy.
async fn work<T: Send + 'static>(
    state: &AppState,
    operation: impl FnOnce(&mut Project) -> CommandResult<T> + Send + 'static,
) -> CommandResult<T> {
    let (ticket, ()) = state.ticket(|_| Ok(()))?;
    let state = state.handle();
    super::blocking("Localisation worker stopped.", move || {
        state.with_ticket(&ticket, |_| Ok(()))?;
        if state.reconcile_project_now(ticket.project)? {
            state.announce_local_change(ticket.project);
        }
        state.with_ticket(&ticket, operation)
    })
    .await?
}
fn saved(outcome: SourceSave) -> CommandResult<()> {
    match outcome {
        SourceSave::Saved(_) => Ok(()),
        SourceSave::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    }
}
#[tauri::command]
pub async fn narrative_locale_get(state: State<'_, AppState>) -> CommandResult<LocaleView> {
    work(&state, |p| Ok(p.locale_view()?)).await
}
#[tauri::command]
pub fn narrative_locale_policy(
    state: State<'_, AppState>,
    policy: Policy,
    expected: String,
) -> CommandResult<()> {
    state.with(|p| saved(p.save_locale_policy(policy, &expected)?))
}
#[tauri::command]
pub async fn narrative_locale_export(
    state: State<'_, AppState>,
    locale: LocaleId,
    csv: bool,
    destination: Option<String>,
) -> CommandResult<String> {
    work(&state, move |p| {
        let text = p.locale_export(&locale, csv)?;
        if let Some(destination) = destination {
            use std::io::Write;
            let path = std::path::PathBuf::from(destination);
            let parent = path.parent().and_then(|p| p.canonicalize().ok()).ok_or_else(|| {
                WobuError::new(Code::Invalid, "Choose an existing destination directory.")
            })?;
            if parent.starts_with(
                p.root()
                    .canonicalize()
                    .map_err(|e| WobuError::new(Code::Invalid, e.to_string()))?,
            ) {
                return Err(WobuError::new(
                    Code::Invalid,
                    "Choose an interchange file outside the project.",
                ));
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| WobuError::new(Code::Invalid, e.to_string()))?;
            file.write_all(text.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|e| WobuError::new(Code::Invalid, e.to_string()))?;
        }
        Ok(text)
    })
    .await
}
#[tauri::command]
pub async fn narrative_locale_preview(
    state: State<'_, AppState>,
    input: String,
    csv: bool,
) -> CommandResult<Vec<wobu_narrative_locale::Diagnostic>> {
    work(&state, move |p| Ok(p.locale_preview(&input, csv)?)).await
}
#[tauri::command]
pub async fn narrative_locale_import(
    state: State<'_, AppState>,
    input: String,
    csv: bool,
) -> CommandResult<ImportReport> {
    work(&state, move |p| Ok(p.locale_import(&input, csv)?)).await
}
#[tauri::command]
pub async fn narrative_locale_approve(state: State<'_, AppState>, row: Row) -> CommandResult<()> {
    work(&state, move |p| saved(p.locale_approve(&row)?)).await
}
