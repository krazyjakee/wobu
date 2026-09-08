//! Prepared media work runs off the native event thread and stays bound to its project ticket.
use crate::{
    error::{CommandResult, WobuError},
    state::AppState,
};
use tauri::State;
use wobu_narrative_locale::LocaleId;
use wobu_narrative_media::{Diagnostic, Key, Policy};
use wobu_store::{
    Project, SourceSave,
    project::narrative_media::{Audition, ImportReport, MediaView},
};
async fn work<T: Send + 'static>(
    state: &AppState,
    operation: impl FnOnce(&mut Project) -> CommandResult<T> + Send + 'static,
) -> CommandResult<T> {
    let (ticket, ()) = state.ticket(|_| Ok(()))?;
    let state = state.handle();
    super::blocking("Recording worker stopped.", move || {
        state.with_ticket(&ticket, |_| Ok(()))?;
        if state.reconcile_project_now(ticket.project)? {
            state.announce_local_change(ticket.project);
        }
        state.with_ticket(&ticket, operation)
    })
    .await?
}
#[tauri::command]
pub async fn narrative_media_get(
    state: State<'_, AppState>,
    locale: LocaleId,
) -> CommandResult<MediaView> {
    work(&state, move |p| Ok(p.media_view(&locale)?)).await
}
#[tauri::command]
pub async fn narrative_media_policy(
    state: State<'_, AppState>,
    policy: Policy,
    expected: String,
) -> CommandResult<()> {
    work(&state, move |p| match p.save_media_policy(policy, &expected)? {
        SourceSave::Saved(_) => Ok(()),
        SourceSave::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    })
    .await
}
#[tauri::command]
pub async fn narrative_media_export(
    state: State<'_, AppState>,
    locale: LocaleId,
    csv: bool,
    destination: Option<String>,
) -> CommandResult<String> {
    work(&state, move |p| {
        let text = p.media_export(&locale, csv)?;
        if let Some(destination) = destination {
            super::narrative_locale::export_file(p, &text, destination)?;
        }
        Ok(text)
    })
    .await
}
#[tauri::command]
pub async fn narrative_media_preview(
    state: State<'_, AppState>,
    input: String,
    csv: bool,
    directory: String,
) -> CommandResult<Vec<Diagnostic>> {
    work(&state, move |p| Ok(p.media_preview(&input, csv, std::path::Path::new(&directory))?)).await
}
#[tauri::command]
pub async fn narrative_media_import(
    state: State<'_, AppState>,
    input: String,
    csv: bool,
    directory: String,
) -> CommandResult<ImportReport> {
    work(&state, move |p| Ok(p.media_import(&input, csv, std::path::Path::new(&directory))?)).await
}
#[tauri::command]
pub async fn narrative_media_audition(
    state: State<'_, AppState>,
    key: Key,
    history: usize,
) -> CommandResult<Audition> {
    work(&state, move |p| Ok(p.media_audition(&key, history)?)).await
}
