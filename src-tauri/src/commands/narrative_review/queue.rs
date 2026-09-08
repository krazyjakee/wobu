//! Bounded project reads and grouped decisions use the same guarded store boundary as Script.
use super::bridge;
use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tauri::State;
use wobu_narrative::{SceneId, review::ReviewTarget};
use wobu_store::{
    Project,
    project::narrative_review::{ReviewRequest, ReviewSceneView},
};

const SCENES_PER_PAGE: usize = 32;
const MAX_PAGE_LINES: usize = 10_000;
const MAX_BATCH: usize = 128;
#[derive(Serialize)]
pub struct ReviewList {
    scenes: Vec<ReviewSceneView>,
    errors: Vec<ListError>,
    next_offset: Option<usize>,
    total_scenes: usize,
    catalog_revision: String,
}
#[derive(Serialize)]
struct ListError {
    scene_id: Option<SceneId>,
    reason: String,
}
#[tauri::command]
pub fn narrative_review_list(
    state: State<'_, AppState>,
    state_json: Option<String>,
    offset: usize,
    expected_catalog: Option<String>,
) -> CommandResult<ReviewList> {
    state.reconcile_now()?;
    state.with(|p| list(p, state_json.as_deref(), offset, expected_catalog.as_deref()))
}
fn list(
    project: &Project,
    state: Option<&str>,
    offset: usize,
    expected_catalog: Option<&str>,
) -> CommandResult<ReviewList> {
    let catalog = project.scene_catalog()?;
    let ids = catalog.ids().into_iter().collect::<Vec<_>>();
    let catalog_revision =
        blake3::hash(&serde_json::to_vec(&ids).map_err(invalid)?).to_hex().to_string();
    if (offset > 0 && expected_catalog != Some(catalog_revision.as_str())) || offset > ids.len() {
        return Err(invalid(
            "Scene listing changed. Refresh the queue before loading another page.",
        ));
    }
    let proposals = project.review_proposals()?;
    let proposal_bytes = serde_json::to_vec(&proposals).map_err(invalid)?;
    let mut output = ReviewList {
        scenes: vec![],
        errors: catalog
            .unreadable
            .iter()
            .map(|source| ListError {
                scene_id: None,
                reason: format!("{}: {}", source.rel, source.reason),
            })
            .collect(),
        next_offset: (offset.saturating_add(SCENES_PER_PAGE) < ids.len())
            .then_some(offset.saturating_add(SCENES_PER_PAGE)),
        total_scenes: ids.len(),
        catalog_revision,
    };
    let mut captures = Vec::new();
    let mut lines = 0;
    for id in ids.iter().skip(offset).take(SCENES_PER_PAGE) {
        let result = (|| {
            let snapshot = project.review_snapshot(*id, state)?;
            let view =
                snapshot.view_with_proposals(proposals.get(id).cloned().unwrap_or_default())?;
            Ok::<_, wobu_store::Error>((snapshot, view))
        })();
        match result {
            Ok((snapshot,view)) if lines + view.lines.len() <= MAX_PAGE_LINES => {
                bridge(&view)?;
                lines += view.lines.len();
                captures.push((snapshot, view));
            }
            Ok(_) => output.errors.push(ListError {scene_id:Some(*id),reason:"This page exceeds 10,000 lines. Open the scene in Script to review it directly.".into()}),
            Err(error) => output.errors.push(ListError {scene_id:Some(*id),reason:error.to_string()}),
        }
    }
    for (snapshot, view) in captures {
        match snapshot.verify_current(project) {
            Ok(()) => output.scenes.push(view),
            Err(error) => output
                .errors
                .push(ListError { scene_id: Some(view.scene_id), reason: error.to_string() }),
        }
    }
    if catalog.ids() != project.scene_catalog()?.ids()
        || proposal_bytes != serde_json::to_vec(&project.review_proposals()?).map_err(invalid)?
    {
        return Err(invalid(
            "Narrative sources or proposals changed during review listing. Refresh and compare before deciding.",
        ));
    }
    bridge(output)
}
#[derive(Serialize)]
pub struct BatchResult {
    items: Vec<BatchItem>,
}
#[derive(Serialize)]
struct BatchItem {
    index: usize,
    target: ReviewTarget,
    status: &'static str,
    reason: String,
}
#[tauri::command]
pub fn narrative_review_batch(
    state: State<'_, AppState>,
    requests: Vec<ReviewRequest>,
    commit: bool,
) -> CommandResult<BatchResult> {
    state.reconcile_now()?;
    state.with(|p| batch(p, &requests, commit))
}
fn batch(
    project: &mut Project,
    requests: &[ReviewRequest],
    commit: bool,
) -> CommandResult<BatchResult> {
    if requests.is_empty() || requests.len() > MAX_BATCH {
        return Err(invalid("Review batches must contain between 1 and 128 decisions."));
    }
    let mut grouped: BTreeMap<SceneId, Vec<(usize, &ReviewRequest)>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    for (index, request) in requests.iter().enumerate() {
        bridge(request)?;
        let key = serde_json::to_string(&request.target).map_err(invalid)?;
        if !seen.insert(key) {
            items.push(item(index, request, "skipped", "Duplicate target in this batch."));
        } else {
            grouped.entry(request.target.scene).or_default().push((index, request));
        }
    }
    for group in grouped.values() {
        let mut transaction = match project.begin_review(group[0].1) {
            Ok(tx) => tx,
            Err(error) => {
                items.extend(
                    group
                        .iter()
                        .map(|(index, r)| item(*index, r, "conflicting", error.to_string())),
                );
                continue;
            }
        };
        let mut eligible = Vec::new();
        for (index, request) in group {
            let result = project
                .review_context(&request.target, Some(&request.state_json))
                .map_err(WobuError::from)
                .and_then(bridge)
                .and_then(|_| project.stage_review(&mut transaction, request).map_err(Into::into));
            match result {
                Ok(()) => eligible.push((*index, *request)),
                Err(error) => items.push(item(*index, request, "conflicting", error.message)),
            }
        }
        if !commit {
            items.extend(eligible.into_iter().map(|(index, r)| {
                item(index, r, "eligible", "Eligible against the reviewed scene and context.")
            }));
        } else if !eligible.is_empty() {
            match project.commit_review(transaction) {
                Ok(_) => items.extend(eligible.into_iter().map(|(index, r)| {
                    item(index, r, "applied", "Decision saved in the scene's canonical history.")
                })),
                Err(error) => items.extend(
                    eligible
                        .into_iter()
                        .map(|(index, r)| item(index, r, "conflicting", error.to_string())),
                ),
            }
        }
    }
    items.sort_by_key(|item| item.index);
    Ok(BatchResult { items })
}
fn item(
    index: usize,
    request: &ReviewRequest,
    status: &'static str,
    reason: impl Into<String>,
) -> BatchItem {
    BatchItem { index, target: request.target.clone(), status, reason: reason.into() }
}
fn invalid(reason: impl std::fmt::Display) -> WobuError {
    WobuError::new(Code::Invalid, reason.to_string())
}

#[cfg(test)]
mod tests;
