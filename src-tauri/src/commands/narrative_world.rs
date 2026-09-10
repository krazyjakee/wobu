//! Whole-document world edits share the source stamp and retain incomplete drafts.
use super::narrative::Precondition;
use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};
use serde::Serialize;
use tauri::State;
use wobu_narrative::{WorldDiagnostic, WorldDocument};
use wobu_store::{Project, SourceSave, atomic::Stamp};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldFile {
    pub document: WorldDocument,
    pub stamp: Option<Stamp>,
    pub diagnostics: Vec<WorldDiagnostic>,
}

fn view(
    project: &Project,
    document: WorldDocument,
    stamp: Option<Stamp>,
) -> CommandResult<WorldFile> {
    let nodes = project.list_nodes()?;
    let entities = nodes.iter().map(|node| node.id).collect();
    let characters = nodes
        .iter()
        .filter(|node| node.kind == wobu_core::NodeKind::Character)
        .map(|node| node.id)
        .collect();
    let diagnostics =
        document.diagnose(&project.state_schema()?, &characters, &entities, &project.scene_ids()?);
    Ok(WorldFile { document, stamp, diagnostics })
}

pub(crate) fn get(project: &Project) -> CommandResult<WorldFile> {
    match project.world_document()? {
        Some((document, stamp)) => view(project, document, Some(stamp)),
        None => view(project, WorldDocument::default(), None),
    }
}

#[tauri::command]
pub fn narrative_world_get(state: State<'_, AppState>) -> CommandResult<WorldFile> {
    state.with(|project| get(project))
}

#[tauri::command]
pub fn narrative_world_save(
    state: State<'_, AppState>,
    document: WorldDocument,
    expected: Precondition,
) -> CommandResult<WorldFile> {
    state.with(|project| save(project, document, &expected))
}

fn save(
    project: &mut Project,
    document: WorldDocument,
    expected: &Precondition,
) -> CommandResult<WorldFile> {
    let stamp = match expected {
        Precondition::Stamp { stamp } => Some(stamp),
        Precondition::New => None,
        Precondition::Current => {
            return Err(WobuError::new(
                Code::Invalid,
                "Reload the world before saving. A source stamp is required to protect other writers.",
            ));
        }
    };
    // Read before writing so unsupported/malformed files are never rewritten.
    project.world_document()?;
    persist(project, document, stamp)
}

fn persist(
    project: &mut Project,
    document: WorldDocument,
    expected: Option<&Stamp>,
) -> CommandResult<WorldFile> {
    // Resolve diagnostics before the mutation, so a failure cannot masquerade as a failed save.
    let document =
        document.for_save().map_err(|error| WobuError::new(Code::Invalid, error.to_string()))?;
    let mut result = view(project, document, None)?;
    match project.save_world(&result.document, expected)? {
        SourceSave::Saved(stamp) => {
            result.stamp = Some(stamp);
            Ok(result)
        }
        SourceSave::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    }
}

#[tauri::command]
pub fn narrative_world_restore(
    state: State<'_, AppState>,
    document: WorldDocument,
    expected: WorldDocument,
) -> CommandResult<WorldFile> {
    state.with(|project| restore(project, document, &expected))
}

fn restore(
    project: &mut Project,
    document: WorldDocument,
    expected: &WorldDocument,
) -> CommandResult<WorldFile> {
    let (on_disk, stamp) = project.world_document()?.map_or_else(
        || (WorldDocument::default(), None),
        |(document, stamp)| (document, Some(stamp)),
    );
    if &on_disk != expected {
        return Err(WobuError::new(
            Code::Invalid,
            "The world changed since this edit. Reload it before undoing or redoing; another writer's changes were kept.",
        ));
    }
    persist(project, document, stamp.as_ref())
}

#[cfg(test)]
mod tests;
