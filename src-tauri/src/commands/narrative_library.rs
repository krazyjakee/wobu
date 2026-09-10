//! One bounded discovery call; returned summaries never authorize an edit.
use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};
use tauri::{AppHandle, Manager};
use wobu_store::project::narrative_library::{LibraryPage, LibraryQuery, QueryError};
#[tauri::command]
pub async fn narrative_library_query(
    app: AppHandle,
    query: LibraryQuery,
) -> CommandResult<LibraryPage> {
    let (ticket, ()) = app.state::<AppState>().ticket(|_| Ok(()))?;
    super::blocking("The library read thread stopped unexpectedly.", move || {
        app.state::<AppState>().with_ticket(&ticket, |project| {
            project.library_query(&query).map_err(|error| match error {
                QueryError::StaleRevision => WobuError::new(Code::Conflict, error.to_string()),
                QueryError::Invalid(_) => WobuError::new(Code::Invalid, error.to_string()),
                QueryError::Store(error) => error.into(),
            })
        })
    })
    .await?
}
