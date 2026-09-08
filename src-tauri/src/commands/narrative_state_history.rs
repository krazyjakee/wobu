//! Guarded declaration undo: compare the authored state, then retain the current source stamp.
use super::narrative::{Precondition, StateFileView, save_state};
use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};
use tauri::State;
use wobu_narrative::StateDocument;
use wobu_store::Project;

#[tauri::command]
pub fn narrative_state_restore(
    state: State<'_, AppState>,
    document: StateDocument,
    expected: StateDocument,
) -> CommandResult<StateFileView> {
    state.with(|project| restore(project, document, &expected))
}

pub(super) fn restore(
    project: &mut Project,
    document: StateDocument,
    expected: &StateDocument,
) -> CommandResult<StateFileView> {
    let (current, stamp) = project.state_document()?.map_or_else(
        || (StateDocument::new(vec![]), None),
        |(document, stamp)| (document, Some(stamp)),
    );
    if &current != expected {
        return Err(WobuError::new(
            Code::Invalid,
            "Variables changed since this edit. Reload before undoing or redoing; the newer declarations were kept.",
        ));
    }
    let precondition = stamp.map_or(Precondition::New, |stamp| Precondition::Stamp { stamp });
    save_state(project, document, &precondition)
}
