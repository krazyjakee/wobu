//! The Text library's commands: supporting text assets on the same terms as
//! scenes (#167).
//!
//! Everything here is the scene surface in `commands::narrative` with the
//! scene-shaped parts removed, and that is deliberate rather than lazy. The
//! guard is the same [`Precondition`], the write is the same whole-document
//! guarded save, and the diagnostics are the same [`DiagnosticView`], because a
//! bark is authored, reviewed, merged and recovered by the same people under
//! the same rules. A second, gentler write path for supporting text would be a
//! second place a collaborator's work could be silently overwritten, and it
//! would be discovered by somebody losing a morning's barks.
//!
//! What is missing compared with scenes, and why:
//!
//! - **No layout.** A text asset has no canvas, which is the same absence that
//!   keeps `Destination` out of its model.
//! - **No rename-in-place command.** A scene has one because its title is shown
//!   in a dozen places; an asset's name is edited in the same form as the rest
//!   of it and arrives through the ordinary save.
//! - **No library index.** The catalog is read from the folder on every call.
//!   Indexing it is the same work as adding it to the scene library's search,
//!   and belongs with that rather than half-done here.

use serde::Serialize;
use tauri::State;
use wobu_narrative::{Name, TextAsset, TextAssetId, TextKind};
use wobu_store::atomic::Stamp;
use wobu_store::{Project, SourceSave, TextFile};

use super::narrative::{DiagnosticView, Precondition, UnreadableScene};
use crate::error::{CommandResult, WobuError};
use crate::state::AppState;

/// One supporting text asset as the library lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSummary {
    pub id: TextAssetId,
    pub kind: TextKind,
    pub name: String,
    pub slug: String,
    pub rel: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextCatalogView {
    pub assets: Vec<TextSummary>,
    pub unreadable: Vec<UnreadableScene>,
}

/// One asset, its file and the stamp a save has to present back.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFileView {
    pub asset: TextAsset,
    pub slug: String,
    pub rel: String,
    pub stamp: Option<Stamp>,
}

impl TextFileView {
    fn of(file: &TextFile) -> TextFileView {
        TextFileView {
            asset: file.asset.clone(),
            slug: file.slug().to_string(),
            rel: file.rel.clone(),
            stamp: file.stamp.clone(),
        }
    }
}

#[tauri::command]
pub fn narrative_texts(state: State<'_, AppState>) -> CommandResult<TextCatalogView> {
    state.with(|project| {
        let catalog = project.text_catalog()?;
        Ok(TextCatalogView {
            assets: catalog
                .assets
                .iter()
                .map(|entry| TextSummary {
                    id: entry.id,
                    kind: entry.kind,
                    name: entry.name.clone(),
                    slug: entry.slug.clone(),
                    rel: entry.rel.clone(),
                })
                .collect(),
            unreadable: catalog
                .unreadable
                .iter()
                .map(|entry| UnreadableScene {
                    rel: entry.rel.clone(),
                    reason: entry.reason.clone(),
                })
                .collect(),
        })
    })
}

#[tauri::command]
pub fn narrative_text_get(
    state: State<'_, AppState>,
    asset_id: TextAssetId,
) -> CommandResult<TextFileView> {
    state.with(|project| Ok(TextFileView::of(&project.load_text_asset(asset_id)?)))
}

/// Create an asset from a kind template.
///
/// `event` is required rather than defaulted, because an asset with no trigger
/// is an asset the host can never ask for, and a placeholder event name would
/// be a working-looking binding to a moment that does not exist. The kind
/// decides the starting repeat policy — see `TextKind::default_repeat`.
#[tauri::command]
pub fn narrative_text_create(
    state: State<'_, AppState>,
    kind: TextKind,
    name: String,
    event: String,
) -> CommandResult<TextFileView> {
    state.with(|project| {
        let event = Name::new(event).map_err(|error| {
            WobuError::new(crate::error::Code::Invalid, "That is not a valid trigger name.")
                .with_detail(error.to_string())
        })?;
        Ok(TextFileView::of(&project.create_text_asset(kind, &name, event)?))
    })
}

#[tauri::command]
pub fn narrative_text_save(
    state: State<'_, AppState>,
    asset: TextAsset,
    slug: Option<String>,
    expected: Precondition,
) -> CommandResult<TextFileView> {
    state.with(|project| save_text(project, asset, slug.as_deref(), &expected))
}

fn save_text(
    project: &mut Project,
    asset: TextAsset,
    slug: Option<&str>,
    expected: &Precondition,
) -> CommandResult<TextFileView> {
    if matches!(expected, Precondition::Current) {
        return Err(WobuError::new(
            crate::error::Code::Invalid,
            "Text saves require the original stamp, so a stale editor cannot overwrite a newer file.",
        ));
    }
    let catalog = project.text_catalog()?;
    let rel = match catalog.find(asset.id) {
        // The catalog is authoritative for an asset that exists: taking the path
        // from the caller would let a stale editor tab write to wherever it last
        // remembered the file being.
        Some(entry) => entry.rel.clone(),
        None => {
            let taken = catalog.slugs();
            let base = wobu_core::slugify(slug.unwrap_or(&asset.name))?;
            let unique = wobu_core::unique_slug(&base, &|candidate| taken.contains(candidate));
            wobu_store::narrative::text_rel(&unique)
        }
    };

    // Read without parsing: the precondition is a fact about the bytes, and a
    // file somebody broke in a text editor still has a stamp — otherwise the
    // person fixing it could not save over it.
    let path = wobu_store::paths::from_rel_string(project.root(), &rel);
    let on_disk = wobu_store::atomic::read_stamped(&path)?.map(|(_, stamp)| stamp);

    let mut file = TextFile { asset, rel, stamp: expected.against(on_disk) };
    match project.save_text_asset(&mut file)? {
        SourceSave::Saved(_) => Ok(TextFileView::of(&file)),
        SourceSave::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    }
}

#[tauri::command]
pub fn narrative_text_delete(
    state: State<'_, AppState>,
    asset_id: TextAssetId,
) -> CommandResult<()> {
    state.with(|project| Ok(project.delete_text_asset(asset_id)?))
}

/// What is wrong with one asset, keyed by the element responsible.
///
/// `asset` overrides what is on disk for the same reason the scene command
/// accepts one: the editor holds unsaved edits, and a diagnostics call that
/// could only read the saved file would report problems the writer has already
/// fixed. The state schema always comes from the project, so a condition is
/// checked against the variables that really exist.
#[tauri::command]
pub fn narrative_text_diagnostics(
    state: State<'_, AppState>,
    asset_id: TextAssetId,
    asset: Option<TextAsset>,
) -> CommandResult<Vec<DiagnosticView>> {
    state.with(|project| {
        let asset = match asset {
            Some(asset) => asset,
            None => project.load_text_asset(asset_id)?.asset,
        };
        let schema = project.state_schema()?;
        Ok(asset.diagnostics(&schema).iter().map(DiagnosticView::of).collect())
    })
}

#[cfg(test)]
mod tests;
