//! One bounded discovery call; returned summaries never authorize an edit.
use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};
use tauri::State;
use wobu_store::project::narrative_library::{LibraryPage, LibraryQuery, QueryError};
#[tauri::command]
pub fn narrative_library_query(
    state: State<'_, AppState>,
    query: LibraryQuery,
) -> CommandResult<LibraryPage> {
    state.with(|project| {
        project.library_query(&query).map_err(|error| match error {
            QueryError::StaleRevision => WobuError::new(Code::Conflict, error.to_string()),
            QueryError::Invalid(_) => WobuError::new(Code::Invalid, error.to_string()),
            QueryError::Store(error) => error.into(),
        })
    })
}
