//! Which authored lines an edit affected, and why (#168).
//!
//! Reports after reconciling external edits. Source saves and reconciliation
//! propagate freshness automatically; deciding what to rebuild or generating
//! replacement wording remains the separate #169 workflow.
//!
//! The wire shape is flattened on purpose. `Affected` is keyed by a typed
//! [`TargetRef`](wobu_narrative_deps::TargetRef) whose two variants carry
//! different containers, and a webview that had to switch on the variant to find
//! out which line a row is about would grow a second copy of that knowledge. The
//! rows here name the scene or the asset, the slot and the variant directly, and
//! the reasons arrive already rendered into the backend's own words — the pane's
//! job is to display them, not to invent a vocabulary for staleness.

use serde::Serialize;
use tauri::State;
use wobu_narrative_deps::{Affected, TargetRef};

use crate::{error::CommandResult, state::AppState};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffectedLine {
    /// The scene this line lives in, or `null` when `asset` names a supporting
    /// text document instead — the same split
    /// [`SourceRef`](wobu_narrative_compiler::SourceRef) makes, for the same
    /// reason.
    scene: Option<String>,
    asset: Option<String>,
    slot: String,
    variant: String,
    /// `changed`, `untracked` or `absent`.
    kind: String,
    /// The fingerprint the line's existing results were produced under, kept so
    /// a receipt recorded against it is still findable.
    before: Option<String>,
    after: Option<String>,
    /// Why affected: source field → context or variant → line.
    explanations: Vec<wobu_narrative_deps::Explanation>,
}

fn row(item: &Affected) -> AffectedLine {
    let (scene, asset) = match &item.target {
        TargetRef::SceneLine { scene, .. } => (Some(scene.to_string()), None),
        TargetRef::TextLine { asset, .. } => (None, Some(asset.to_string())),
    };
    AffectedLine {
        scene,
        asset,
        slot: item.target.slot().to_string(),
        variant: item.target.variant().to_string(),
        kind: serde_json::to_value(item.kind)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default(),
        before: item.before.clone(),
        after: item.after.clone(),
        explanations: item.explanations(),
    }
}

/// Every line whose recorded dependencies no longer match the project.
#[tauri::command]
pub fn narrative_affected(state: State<'_, AppState>) -> CommandResult<Vec<AffectedLine>> {
    state.reconcile_now()?;
    state.with(|project| Ok(project.narrative_affected()?.iter().map(row).collect()))
}

#[cfg(test)]
mod tests;
