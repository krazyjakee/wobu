//! #209. Which wordings are stored in more than one place, and what an author
//! said about the ones they meant.
//!
//! Its own command group rather than a third scope on `narrative_diagnostics`,
//! because it answers a question no scene can answer about itself. Every other
//! narrative diagnostic is settled by reading one document against the declared
//! variables and a list of ids; this one is settled by reading all of them and
//! comparing digests, so it costs a pass over the project and must not be on the
//! path of a keystroke in the Script tab.

use serde::Serialize;
use tauri::{AppHandle, Manager};
use wobu_narrative::{DuplicatedWording, WordingSuppression};
use wobu_store::{Project, SourceSave};

use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};

/// The whole project's repeated wordings, and the answers already given.
///
/// The suppressions are returned beside the findings rather than silently
/// applied and forgotten, so a reader can see both what is repeated and what has
/// been signed off — a list that only showed what is left would make a
/// suppression indistinguishable from a line somebody deleted.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordingReport {
    pub duplicated: Vec<DuplicatedWording>,
    pub suppressions: Vec<WordingSuppression>,
    /// The guard to hand back to [`narrative_wording_suppress`]. Opaque.
    pub guard: String,
    /// Files that would not parse. Their wordings are in none of the counts
    /// above, and reporting nothing for them would read as "these are fine".
    pub unreadable: Vec<String>,
}

#[tauri::command]
pub async fn narrative_wording_report(app: AppHandle) -> CommandResult<WordingReport> {
    let (ticket, ()) = app.state::<AppState>().ticket(|_| Ok(()))?;
    super::blocking("The wording check thread stopped unexpectedly.", move || {
        app.state::<AppState>().with_ticket(&ticket, report)
    })
    .await?
}

pub(crate) fn report(project: &mut Project) -> CommandResult<WordingReport> {
    let catalog = project.scene_catalog()?;
    let texts = project.text_catalog()?;
    let scenes = catalog
        .scenes
        .iter()
        .map(|entry| project.load_scene(entry.id).map(|file| file.scene))
        .collect::<Result<Vec<_>, _>>()?;
    let assets = project.text_assets()?;
    let (suppressions, guard) = project.wording_suppressions()?;
    Ok(WordingReport {
        duplicated: wobu_narrative::duplicated_wording(&scenes, &assets, &suppressions),
        suppressions,
        guard,
        unreadable: catalog
            .unreadable
            .into_iter()
            .chain(texts.unreadable)
            .map(|source| source.rel)
            .collect(),
    })
}

/// Record, or withdraw, the whole set of signed-off repetitions.
///
/// The whole list rather than one entry, for the reason
/// [`Project::save_wording_suppressions`] gives: the guard is what makes two
/// writers on a shared folder safe, and a per-entry write would need a guard per
/// entry to say the same thing. Withdrawing one is sending the list without it.
#[tauri::command]
pub fn narrative_wording_suppress(
    state: tauri::State<'_, AppState>,
    suppressions: Vec<WordingSuppression>,
    expected: String,
) -> CommandResult<WordingReport> {
    for suppression in &suppressions {
        check_rationale(&suppression.rationale)?;
    }
    state.with(|project| {
        match project.save_wording_suppressions(suppressions.clone(), &expected)? {
            SourceSave::Saved(_) => report(project),
            // A conflict sibling rather than an overwrite, exactly as for source:
            // two people answering the same repetition differently is a
            // disagreement to read, not one to resolve by arrival order.
            SourceSave::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
        }
    })
}

/// Refuse a rationale that is only whitespace before it reaches the store, so
/// the message names the field the user is looking at.
pub(crate) fn check_rationale(rationale: &str) -> CommandResult<()> {
    if rationale.trim().is_empty() {
        return Err(WobuError::new(
            Code::Invalid,
            "Say why this repetition is deliberate. A suppression with no reason cannot be told \
             apart from a warning somebody dismissed.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
