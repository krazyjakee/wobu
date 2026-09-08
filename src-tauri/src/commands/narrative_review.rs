//! Coarse review commands share one canonical storage transition boundary.
pub mod queue;
use super::narrative::SceneFileView;
use crate::{
    error::{Code, CommandResult, WobuError},
    state::{AppState, ProjectTicket},
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use wobu_narrative::{
    Scene, SceneId,
    review::{ReviewContext, ReviewTarget},
};
use wobu_store::project::narrative_review::{ReviewRequest, ReviewSceneView, ReviewSnapshot};
#[derive(Serialize)]
pub struct ReviewApplied {
    file: SceneFileView,
    review: ReviewSceneView,
}
#[tauri::command]
pub async fn narrative_review_get(
    app: AppHandle,
    scene_id: SceneId,
    state_json: Option<String>,
) -> CommandResult<ReviewSceneView> {
    let (ticket, ()) = app.state::<AppState>().ticket(|_| Ok(()))?;
    super::blocking("The review read thread stopped unexpectedly.", move || {
        let state = app.state::<AppState>();
        let snapshot = capture_read(&state, &ticket, scene_id, state_json.as_deref())?;
        let proposals = state.with_ticket(&ticket, |p| {
            Ok(p.review_proposals()?.remove(&scene_id).unwrap_or_default())
        })?;
        let view = snapshot.view_with_proposals(proposals)?;
        finish_read(&state, &ticket, &snapshot, view)
    })
    .await?
}
#[tauri::command]
pub async fn narrative_review_context(
    app: AppHandle,
    target: ReviewTarget,
    state_json: Option<String>,
) -> CommandResult<ReviewContext> {
    let (ticket, ()) = app.state::<AppState>().ticket(|_| Ok(()))?;
    super::blocking("The review context thread stopped unexpectedly.", move || {
        let state = app.state::<AppState>();
        let snapshot = capture_read(&state, &ticket, target.scene, state_json.as_deref())?;
        let context = snapshot.context(&target)?;
        finish_read(&state, &ticket, &snapshot, context)
    })
    .await?
}
pub(crate) fn capture_read(
    state: &AppState,
    ticket: &ProjectTicket,
    scene: SceneId,
    scenario: Option<&str>,
) -> CommandResult<ReviewSnapshot> {
    state.reconcile_ticket_now(ticket)?;
    state.with_ticket(ticket, |project| {
        project.review_snapshots(&[scene], scenario)?.pop().ok_or_else(|| {
            WobuError::new(Code::Invalid, "The requested review source was not captured.")
        })
    })
}
pub(crate) fn finish_read<T: Serialize>(
    state: &AppState,
    ticket: &ProjectTicket,
    snapshot: &ReviewSnapshot,
    value: T,
) -> CommandResult<T> {
    state.with_ticket(ticket, |project| Ok(snapshot.verify_current(project)?))?;
    bridge(value)
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
    let snapshots =
        project.review_snapshots(&scenes.iter().map(|scene| scene.id).collect::<Vec<_>>(), None)?;
    for (scene, snapshot) in scenes.iter().zip(&snapshots) {
        if snapshot.scene() != scene {
            return Err(WobuError::new(
                Code::Invalid,
                "Scene changed while verifying review history.",
            ));
        }
        evidence.extend(snapshot.evidence()?);
    }
    project.verify_review_snapshots(&snapshots)?;
    if project.narrative_fingerprint()? != fingerprint {
        return Err(WobuError::new(
            Code::Invalid,
            "Narrative inputs changed while verifying review history.",
        ));
    }
    Ok(evidence)
}
