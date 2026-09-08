//! Coarse review commands share one canonical storage transition boundary.
use super::narrative::SceneFileView;
use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};
use serde::Serialize;
use tauri::State;
use wobu_narrative::{
    Scene, SceneId,
    review::{ReviewContext, ReviewTarget},
};
use wobu_store::project::narrative_review::{ReviewRequest, ReviewSceneView};
#[derive(Serialize)]
pub struct ReviewApplied {
    file: SceneFileView,
    review: ReviewSceneView,
}
#[tauri::command]
pub fn narrative_review_get(
    state: State<'_, AppState>,
    scene_id: SceneId,
    state_json: Option<String>,
) -> CommandResult<ReviewSceneView> {
    state.reconcile_now()?;
    state.with(|p| bridge(p.review_scene(scene_id, state_json.as_deref())?))
}
#[tauri::command]
pub fn narrative_review_context(
    state: State<'_, AppState>,
    target: ReviewTarget,
    state_json: Option<String>,
) -> CommandResult<ReviewContext> {
    state.reconcile_now()?;
    state.with(|p| bridge(p.review_context(&target, state_json.as_deref())?))
}
#[tauri::command]
pub fn narrative_review_apply(
    state: State<'_, AppState>,
    request: ReviewRequest,
) -> CommandResult<ReviewApplied> {
    state.reconcile_now()?;
    state.with(|p| {
        bridge(p.review_context(&request.target, Some(&request.state_json))?)?;
        let (file, review) = p.apply_review(&request)?;
        Ok(ReviewApplied { file: SceneFileView::of(&file), review })
    })
}
#[tauri::command]
pub fn narrative_scene_restore(
    state: State<'_, AppState>,
    scene: Scene,
    expected: Option<Scene>,
    slug: String,
) -> CommandResult<SceneFileView> {
    if wobu_core::slugify(&slug)? != slug {
        return Err(WobuError::new(Code::Invalid, "Expected the scene's original portable slug."));
    }
    state.with(|p| Ok(SceneFileView::of(&p.restore_scene(scene, expected.as_ref(), &slug)?)))
}

fn bridge<T: Serialize>(value: T) -> CommandResult<T> {
    if super::narrative_context::unsafe_integer(
        &serde_json::to_value(&value).map_err(|e| WobuError::new(Code::Invalid, e.to_string()))?,
    ) {
        return Err(WobuError::new(
            Code::Invalid,
            "Review inputs contain integers outside the exact webview range. Narrow the authored values before reviewing in the desktop UI.",
        ));
    }
    Ok(value)
}

pub(super) fn verified(
    project: &wobu_store::Project,
    scenes: &[Scene],
    fingerprint: &str,
) -> CommandResult<
    std::collections::BTreeMap<wobu_narrative::VariantId, wobu_narrative::review::ApprovalEvidence>,
> {
    let mut evidence = std::collections::BTreeMap::new();
    let mut snapshots = Vec::new();
    for scene in scenes {
        let snapshot = project.review_snapshot(scene.id, None)?;
        if snapshot.scene() != scene {
            return Err(WobuError::new(
                Code::Invalid,
                "Scene changed while verifying review history.",
            ));
        }
        evidence.extend(snapshot.evidence()?);
        snapshots.push(snapshot);
    }
    for snapshot in &snapshots {
        snapshot.verify_current(project)?;
    }
    if project.narrative_fingerprint()? != fingerprint {
        return Err(WobuError::new(
            Code::Invalid,
            "Narrative inputs changed while verifying review history.",
        ));
    }
    Ok(evidence)
}
