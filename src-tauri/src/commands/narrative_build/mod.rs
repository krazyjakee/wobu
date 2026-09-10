//! Explicit authoring builds reuse the generation queue and its canonical evidence.
pub(super) mod plan;
use super::narrative_generation::{self as generation, GenerationPlans, Queued, records};
use crate::{
    error::{Code, CommandResult, WobuError},
    keys::Keys,
    state::{AppState, Jobs},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tauri::{AppHandle, State};
use wobu_core::Id;
use wobu_narrative_build::{Action, Build, Scope};
use wobu_narrative_generation::{AttemptStatus, FrozenRequest, Receipt};
use wobu_store::Project;
fn invalid(error: impl std::fmt::Display) -> WobuError {
    WobuError::new(Code::Invalid, error.to_string())
}

#[tauri::command]
pub async fn narrative_build_plan(
    state: State<'_, AppState>,
    source: String,
) -> CommandResult<Build> {
    plan_saved(&state, &source, || {})
}
/// Bind authoring publication to the session that requested the plan. The hook
/// exercises the unlocked reconciliation/publication boundary deterministically.
pub(crate) fn plan_saved(
    state: &AppState,
    source: &str,
    after_reconcile: impl FnOnce(),
) -> CommandResult<Build> {
    let (ticket, ()) = state.ticket(|_| Ok(()))?;
    let input = serde_json::from_str(source).map_err(invalid)?;
    if state.reconcile_project_now(ticket.project)? {
        state.announce_local_change(ticket.project);
    }
    after_reconcile();
    state.with_ticket(&ticket, |project| {
        let selection = crate::enhance::selection(&project.meta().providers);
        let model = crate::enhance::planning_model(&selection)?;
        let build = plan::build(project, input, &selection.provider, &model)?;
        super::narrative_preview::bridge_integers(&build)?;
        Ok(build)
    })
}
#[derive(Serialize)]
pub struct Status {
    pub build: Build,
    pub history: Vec<generation::HistoryItem>,
    pub dispatched: BTreeSet<Id>,
    pub decided: BTreeSet<Id>,
}
#[tauri::command]
pub async fn narrative_build_status(
    state: State<'_, AppState>,
    build_id: Id,
) -> CommandResult<Status> {
    state.with(|project| status(project, build_id))
}
fn status(project: &Project, id: Id) -> CommandResult<Status> {
    let build = project.narrative_build(id)?;
    let requests: BTreeSet<_> = build.items.iter().filter_map(|i| i.request_id).collect();
    let history: Vec<_> = generation::history_for(project, Some(&requests))?
        .into_iter()
        .filter(|i| requests.contains(&i.request_id))
        .collect();
    let automatic: BTreeSet<_> = build
        .items
        .iter()
        .filter(|item| item.action == Action::Generate)
        .filter_map(|item| item.request_id)
        .collect();
    let mut scenes = BTreeMap::<_, BTreeSet<_>>::new();
    for item in history
        .iter()
        .filter(|item| item.proposal_published && automatic.contains(&item.request_id))
    {
        if let Some(receipt) = item.receipt_id {
            scenes.entry(item.target.scene).or_default().insert(receipt);
        }
    }
    let decided = project.narrative_decided_proposals(&scenes)?;
    Ok(Status { build, history, dispatched: project.narrative_build_dispatched(id)?, decided })
}
#[derive(Serialize)]
pub struct Summary {
    pub id: Id,
    pub scope: Scope,
    pub items: usize,
    pub provider: String,
    pub model: String,
}
#[tauri::command]
pub async fn narrative_build_list(state: State<'_, AppState>) -> CommandResult<Vec<Summary>> {
    state.with(|project| {
        project
            .narrative_build_ids()?
            .into_iter()
            .rev()
            .map(|id| {
                let b = project.narrative_build(id)?;
                Ok(Summary {
                    id,
                    scope: b.scope,
                    items: b.items.len(),
                    provider: b.provider,
                    model: b.model,
                })
            })
            .collect()
    })
}

/// Reuse succeeds through the original receipt/proposal path. No synthetic paid
/// attempt is invented, and no retained successful request can be resubmitted.
fn prepare(
    project: &mut Project,
    build_id: Id,
    items: &BTreeSet<Id>,
) -> CommandResult<Vec<FrozenRequest>> {
    let build = project.narrative_build(build_id)?;
    let known = build.items.iter().map(|i| i.id).collect::<BTreeSet<_>>();
    if items.is_empty() || !items.is_subset(&known) {
        return Err(invalid("Select current build items."));
    }
    let evidence = records::RecordSet::load(project)?;
    let mut queued = Vec::new();
    let mut recover = Vec::new();
    let mut fresh = Vec::new();
    for item in build.items.iter().filter(|i| items.contains(&i.id)) {
        let Some(request_id) = item.request_id else {
            return Err(invalid("Locked or blocked items cannot run."));
        };
        let request = evidence
            .requests
            .get(&request_id)
            .ok_or_else(|| invalid("Build request is missing."))?;
        if request.target != item.target
            || Some(request.candidate_variant_id) != item.candidate_variant_id
            || request.context.options.state != item.state
            || request.provider != build.provider
            || request.model != build.model
            || (!item.reusable && request.batch_id != build.id)
        {
            return Err(invalid("Build item differs from frozen request."));
        }
        let attempts = evidence.attempts(request)?;
        if let Some((id, receipt)) = attempts.iter().find(|(_, r)| {
            matches!(
                r,
                Receipt::NarrativeGenerationAttempt { status: AttemptStatus::Succeeded, .. }
            )
        }) {
            if item.reusable {
                fresh.push(request.clone());
            }
            recover.push((request, *id, receipt.clone(), item.action));
            continue;
        }
        queued.push(request.clone());
        fresh.push(request.clone());
    }
    check_batch(project, &fresh)?;
    for (request, id, receipt, action) in recover {
        records::publish(project, request, id, &receipt)?;
        if action == Action::Generate {
            project.apply_generated_proposal(id)?;
        }
    }
    Ok(queued)
}
#[tauri::command]
pub async fn narrative_build_start(
    app: AppHandle,
    state: State<'_, AppState>,
    keys: State<'_, Keys>,
    jobs: State<'_, Jobs>,
    plans: State<'_, GenerationPlans>,
    build_id: Id,
    items: BTreeSet<Id>,
) -> CommandResult<Vec<Queued>> {
    if jobs.queue().is_closed() {
        return Err(invalid("The job queue is shutting down."));
    }
    state.reconcile_now()?;
    let (ticket, ()) = state.ticket(|_| Ok(()))?;
    let (requests, selected_requests) = state.with_ticket(&ticket, |project| {
        let requests = prepare(project, build_id, &items)?;
        let selected_requests = project
            .narrative_build(build_id)?
            .items
            .iter()
            .filter(|i| items.contains(&i.id))
            .filter_map(|i| i.request_id)
            .collect::<Vec<_>>();
        Ok((requests, selected_requests))
    })?;
    if requests.is_empty() {
        state.with_ticket(&ticket, |project| {
            project.record_narrative_build_dispatch(build_id, selected_requests)?;
            Ok(())
        })?;
        return Ok(vec![]);
    }
    let provider_id = requests[0].provider.clone();
    if requests.iter().any(|r| r.provider != provider_id) {
        return Err(invalid("Build mixes provider identities."));
    }
    let permits = requests
        .iter()
        .map(|r| plans.reserve(ticket.project, r.request_id))
        .collect::<CommandResult<Vec<_>>>()?;
    let key = Arc::new(
        keys.secret(&provider_id).await?.ok_or_else(|| crate::enhance::no_key(&provider_id))?,
    );
    let provider = crate::enhance::text_provider(&provider_id, &key)?;
    if !provider.supports_structured() {
        return Err(invalid("Provider does not support structured narrative output."));
    }
    state.with_ticket(&ticket, |project| {
        check_batch(project, &requests)?;
        project.record_narrative_build_dispatch(build_id, selected_requests)?;
        Ok(())
    })?;
    Ok(requests
        .into_iter()
        .zip(permits)
        .map(|(request, permit)| {
            generation::submit(&app, &ticket, &jobs, request, provider.clone(), key.clone(), permit)
        })
        .collect())
}
#[cfg(test)]
mod tests;

fn check_batch(project: &Project, requests: &[FrozenRequest]) -> CommandResult<()> {
    if requests.is_empty() {
        return Ok(());
    }
    let snapshot = project.narrative_dependency_snapshot()?;
    let linked = snapshot.scenes.iter().cloned().map(|s| (s.id, s)).collect();
    let mut scenes: BTreeMap<_, _> = snapshot.scenes.iter().cloned().map(|s| (s.id, s)).collect();
    for asset in &snapshot.texts {
        let scene = asset.editorial_scene();
        if scenes.insert(scene.id, scene).is_some() {
            return Err(invalid("Conflicting source identities."));
        }
    }
    for request in requests {
        wobu_store::project::narrative_generation::check_analysis(project, request)?;
        let scene = scenes
            .get(&request.target.scene)
            .ok_or_else(|| invalid("Build target no longer exists."))?;
        let context = wobu_narrative_context::resolve_linked(
            wobu_narrative_context::Input {
                scene,
                world: &snapshot.world,
                schema: &snapshot.schema,
                characters: &snapshot.characters,
            },
            request.context.options.clone(),
            &linked,
        );
        if !wobu_store::project::narrative_generation::checks_captured(scene, &context, request)
            .current()
        {
            return Err(invalid(
                "Source, policy or text changed. Replan before spending a provider request.",
            ));
        }
    }
    snapshot.check_current(project)?;
    Ok(())
}
