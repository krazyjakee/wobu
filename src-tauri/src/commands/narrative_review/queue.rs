//! Bounded project reads and grouped decisions use the same guarded store boundary as Script.
use super::bridge;
use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tauri::{AppHandle, Manager, State};
use wobu_narrative::{SceneId, review::ReviewTarget};
use wobu_store::{
    Project,
    project::narrative_review::{ReviewRequest, ReviewSceneView, ReviewSnapshot},
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
pub async fn narrative_review_list(
    app: AppHandle,
    state_json: Option<String>,
    offset: usize,
    expected_catalog: Option<String>,
) -> CommandResult<ReviewList> {
    let (ticket, ()) = app.state::<AppState>().ticket(|_| Ok(()))?;
    super::super::blocking("The review queue thread stopped unexpectedly.", move || {
        let state = app.state::<AppState>();
        state.reconcile_ticket_now(&ticket)?;
        let captured = state.with_ticket(&ticket, |project| {
            capture_list(project, state_json.as_deref(), offset, expected_catalog.as_deref())
        })?;
        let rendered = captured.render()?;
        state.with_ticket(&ticket, |project| rendered.finish(project))
    })
    .await?
}
struct CapturedList {
    output: ReviewList,
    snapshots: Vec<(SceneId, wobu_store::Result<ReviewSnapshot>)>,
    proposals: BTreeMap<SceneId, Vec<wobu_store::project::narrative_review::ReviewProposal>>,
    proposal_bytes: Vec<u8>,
    scene_ids: BTreeSet<SceneId>,
    text_ids: Vec<wobu_narrative::TextAssetId>,
}
struct RenderedList {
    captured: CapturedList,
    views: Vec<(ReviewSnapshot, ReviewSceneView)>,
}
#[cfg(test)]
fn list(
    project: &Project,
    state: Option<&str>,
    offset: usize,
    expected_catalog: Option<&str>,
) -> CommandResult<ReviewList> {
    capture_list(project, state, offset, expected_catalog)?.render()?.finish(project)
}
fn capture_list(
    project: &Project,
    state: Option<&str>,
    offset: usize,
    expected_catalog: Option<&str>,
) -> CommandResult<CapturedList> {
    let catalog = project.scene_catalog()?;
    let texts = project.text_catalog()?;
    let mut ids = catalog
        .ids()
        .into_iter()
        .chain(texts.assets.iter().map(|asset| SceneId::from_raw(asset.id.raw())))
        .collect::<Vec<_>>();
    ids.sort();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid("Scene and supporting text identities collide."));
    }
    let catalog_revision =
        blake3::hash(&serde_json::to_vec(&ids).map_err(invalid)?).to_hex().to_string();
    if (offset > 0 && expected_catalog != Some(catalog_revision.as_str())) || offset > ids.len() {
        return Err(invalid(
            "Scene listing changed. Refresh the queue before loading another page.",
        ));
    }
    let proposals = project.review_proposals()?;
    let proposal_bytes = serde_json::to_vec(&proposals).map_err(invalid)?;
    let output = ReviewList {
        scenes: vec![],
        errors: catalog
            .unreadable
            .iter()
            .chain(&texts.unreadable)
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
    let page = ids.into_iter().skip(offset).take(SCENES_PER_PAGE).collect::<Vec<_>>();
    // The common path captures membership/fingerprints once. A malformed source
    // retains the queue's per-container errors and other reviewable results.
    let snapshots = match project.review_snapshots(&page, state) {
        Ok(snapshots) => page.into_iter().zip(snapshots.into_iter().map(Ok)).collect(),
        Err(_) => page.into_iter().map(|id| (id, project.review_snapshot(id, state))).collect(),
    };
    Ok(CapturedList {
        output,
        snapshots,
        proposals,
        proposal_bytes,
        scene_ids: catalog.ids(),
        text_ids: texts.assets.iter().map(|asset| asset.id).collect(),
    })
}
impl CapturedList {
    /// Pure context/evidence work uses only frozen values, outside the slot mutex.
    fn render(mut self) -> CommandResult<RenderedList> {
        let mut views = Vec::new();
        let mut lines = 0;
        for (id, snapshot) in std::mem::take(&mut self.snapshots) {
            let result = snapshot.and_then(|snapshot| {
                let view =
                    snapshot.view_with_proposals(self.proposals.remove(&id).unwrap_or_default())?;
                Ok((snapshot, view))
            });
            match result {
                Ok((snapshot, view)) if lines + view.lines.len() <= MAX_PAGE_LINES => {
                    bridge(&view)?;
                    lines += view.lines.len();
                    views.push((snapshot, view));
                }
                Ok(_) => self.output.errors.push(ListError { scene_id: Some(id), reason: "This page exceeds 10,000 lines. Open the scene in Script to review it directly.".into() }),
                Err(error) => self.output.errors.push(ListError { scene_id: Some(id), reason: error.to_string() }),
            }
        }
        Ok(RenderedList { captured: self, views })
    }
}
impl RenderedList {
    fn finish(mut self, project: &Project) -> CommandResult<ReviewList> {
        let batch_current = project
            .verify_review_snapshots(self.views.iter().map(|(snapshot, _)| snapshot))
            .is_ok();
        for (snapshot, view) in self.views {
            let checked = if batch_current { Ok(()) } else { snapshot.verify_current(project) };
            match checked {
                Ok(()) => self.captured.output.scenes.push(view),
                Err(error) => self
                    .captured
                    .output
                    .errors
                    .push(ListError { scene_id: Some(view.scene_id), reason: error.to_string() }),
            }
        }
        if self.captured.text_ids
            != project.text_catalog()?.assets.iter().map(|asset| asset.id).collect::<Vec<_>>()
            || self.captured.scene_ids != project.scene_catalog()?.ids()
            || self.captured.proposal_bytes
                != serde_json::to_vec(&project.review_proposals()?).map_err(invalid)?
        {
            return Err(invalid(
                "Narrative sources or proposals changed during review listing. Refresh and compare before deciding.",
            ));
        }
        bridge(self.captured.output)
    }
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
