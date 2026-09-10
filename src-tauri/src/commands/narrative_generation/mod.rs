//! Provider jobs produce immutable evidence and review proposals, never accepted dialogue.
pub(super) mod freeze;
mod plan;
pub(super) mod records;
pub(super) mod task;

use crate::{
    error::{Code, CommandResult, WobuError},
    keys::Keys,
    state::{AppState, Jobs, ProjectTicket},
};
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tauri::{AppHandle, State};
use wobu_core::Id;
use wobu_narrative_generation::{AttemptStatus, FrozenRequest, Receipt};
use wobu_store::Project;

fn invalid(error: impl std::fmt::Display) -> WobuError {
    WobuError::new(Code::Invalid, error.to_string())
}

#[derive(Default)]
pub struct GenerationPlans {
    plans: Mutex<BTreeMap<Id, (ProjectTicket, plan::Plan)>>,
    active: Arc<Mutex<BTreeSet<(Id, Id)>>>,
}
pub struct GenerationPermit {
    active: Arc<Mutex<BTreeSet<(Id, Id)>>>,
    key: (Id, Id),
}
impl Drop for GenerationPermit {
    fn drop(&mut self) {
        self.active.lock().remove(&self.key);
    }
}
impl GenerationPlans {
    pub(super) fn reserve(&self, project: Id, request: Id) -> CommandResult<GenerationPermit> {
        let key = (project, request);
        if !self.active.lock().insert(key) {
            return Err(invalid("This request is already queued or running."));
        }
        Ok(GenerationPermit { active: self.active.clone(), key })
    }
}

#[tauri::command]
pub async fn narrative_generation_plan(
    state: State<'_, AppState>,
    plans: State<'_, GenerationPlans>,
    source: String,
) -> CommandResult<plan::Plan> {
    let input: plan::PlanInput = serde_json::from_str(&source).map_err(invalid)?;
    state.reconcile_now()?;
    let (ticket, selection) =
        state.ticket(|project| Ok(crate::enhance::selection(&project.meta().providers)))?;
    let model = crate::enhance::planning_model(&selection)?;
    let plan = state
        .with_ticket(&ticket, |project| plan::build(project, input, &selection.provider, &model))?;
    super::narrative_preview::bridge_integers(&plan)?;
    let mut retained = plans.plans.lock();
    retained.retain(|_, (existing, _)| *existing == ticket);
    if retained.len() >= 16 {
        retained.pop_first();
    }
    retained.insert(plan.id, (ticket, plan.clone()));
    Ok(plan)
}

#[derive(Serialize)]
pub struct Queued {
    pub request_id: Id,
    pub job_id: String,
}

#[tauri::command]
pub async fn narrative_generation_start(
    app: AppHandle,
    state: State<'_, AppState>,
    keys: State<'_, Keys>,
    jobs: State<'_, Jobs>,
    plans: State<'_, GenerationPlans>,
    plan_id: Id,
) -> CommandResult<Vec<Queued>> {
    if jobs.queue().is_closed() {
        return Err(invalid("The job queue is shutting down."));
    }
    let (ticket, plan) = plans
        .plans
        .lock()
        .get(&plan_id)
        .cloned()
        .ok_or_else(|| invalid("Generation plan expired; plan again."))?;
    let key = Arc::new(
        keys.secret(&plan.provider).await?.ok_or_else(|| crate::enhance::no_key(&plan.provider))?,
    );
    let provider = crate::enhance::text_provider(&plan.provider, &key)?;
    // The plan is consumed atomically before submission; double clicks cannot queue duplicates.
    let (_, plan) = plans
        .plans
        .lock()
        .remove(&plan_id)
        .ok_or_else(|| invalid("This generation plan was already queued."))?;
    let permits = plan
        .requests
        .iter()
        .map(|request| plans.reserve(ticket.project, request.request_id))
        .collect::<CommandResult<Vec<_>>>()?;
    state.with_ticket(&ticket,|project| {
        for request in &plan.requests {
            if !records::checks(project,request)?.current() {
                return Err(invalid("Source, context or policy changed after planning. Plan again before spending a request."));
            }
        }
        for request in &plan.requests {
            records::save_receipt(project,request.request_id,"Frozen narrative generation request",&Receipt::NarrativeGenerationRequest {request:Box::new(request.clone())})?;
        }
        Ok(())
    })?;
    Ok(plan
        .requests
        .into_iter()
        .zip(permits)
        .map(|(request, permit)| {
            submit(&app, &ticket, &jobs, request, provider.clone(), key.clone(), permit)
        })
        .collect())
}

#[derive(Serialize)]
pub struct HistoryItem {
    pub request_id: Id,
    pub batch_id: Id,
    pub target: wobu_narrative_context::Selection,
    pub provider: String,
    pub model: String,
    pub attempts: u32,
    pub status: String,
    pub receipt_id: Option<Id>,
    pub candidate: Option<wobu_narrative_generation::Candidate>,
    pub usage: wobu_narrative_generation::TokenUsage,
    pub billing_unknown: bool,
    pub error_code: Option<String>,
    pub proposal_published: bool,
    pub proposal_current_at_publication: Option<bool>,
}
pub(super) fn history(project: &Project) -> CommandResult<Vec<HistoryItem>> {
    history_for(project, None)
}
pub(super) fn history_for(
    project: &Project,
    only: Option<&BTreeSet<Id>>,
) -> CommandResult<Vec<HistoryItem>> {
    let mut history = Vec::new();
    let receipts = records::RecordSet::load(project)?;
    for request in
        receipts.requests.values().filter(|r| only.is_none_or(|ids| ids.contains(&r.request_id)))
    {
        let attempts = receipts.attempts(request)?;
        let mut item = HistoryItem {
            request_id: request.request_id,
            batch_id: request.batch_id,
            target: request.target.clone(),
            provider: request.provider.clone(),
            model: request.model.clone(),
            attempts: 0,
            status: "interrupted".into(),
            receipt_id: None,
            candidate: None,
            usage: Default::default(),
            billing_unknown: true,
            error_code: None,
            proposal_published: false,
            proposal_current_at_publication: None,
        };
        for (id, receipt) in attempts {
            let recorded = receipt.clone();
            if let Receipt::NarrativeGenerationAttempt {
                attempt,
                status,
                usage,
                billing_unknown,
                error_code,
                candidate,
                ..
            } = receipt
            {
                item.attempts = attempt;
                item.receipt_id = Some(id);
                item.usage = usage;
                item.billing_unknown = billing_unknown;
                item.error_code = error_code;
                item.candidate = candidate;
                item.status = match status {
                    AttemptStatus::Succeeded => "succeeded",
                    AttemptStatus::InvalidOutput => "invalid_output",
                    AttemptStatus::Failed => "failed",
                    AttemptStatus::Cancelled => "cancelled",
                }
                .into();
                if status == AttemptStatus::Succeeded {
                    if let Some(checks) = records::published(project, request, id, &recorded)? {
                        item.proposal_published = true;
                        item.proposal_current_at_publication = Some(checks.current());
                    }
                    break;
                }
            }
        }
        history.push(item);
    }
    Ok(history)
}
#[tauri::command]
pub fn narrative_generation_history(state: State<'_, AppState>) -> CommandResult<Vec<HistoryItem>> {
    state.with(|project| {
        let result = history(project)?;
        super::narrative_preview::bridge_integers(&result)?;
        Ok(result)
    })
}

#[tauri::command]
pub async fn narrative_generation_retry(
    app: AppHandle,
    state: State<'_, AppState>,
    keys: State<'_, Keys>,
    jobs: State<'_, Jobs>,
    plans: State<'_, GenerationPlans>,
    request_id: Id,
) -> CommandResult<Queued> {
    if jobs.queue().is_closed() {
        return Err(invalid("The job queue is shutting down."));
    }
    let (ticket,request)=state.ticket(|project| {
        if project.is_read_only() { return Err(wobu_store::Error::ReadOnly.into()); }
        let request=records::request(project,request_id)?;
        if records::attempts(project,&request)?.iter().any(|(_,r)| matches!(r,Receipt::NarrativeGenerationAttempt {status:AttemptStatus::Succeeded,..})) {
            return Err(invalid("A successful result is already retained. Repair its publication instead of spending another request."));
        }
        if records::checks(project,&request)?.locked_now {return Err(invalid("This dialogue is now locked; retry is excluded."));}
        Ok(request)
    })?;
    let permit = plans.reserve(ticket.project, request_id)?;
    let key = Arc::new(
        keys.secret(&request.provider)
            .await?
            .ok_or_else(|| crate::enhance::no_key(&request.provider))?,
    );
    let provider = crate::enhance::text_provider(&request.provider, &key)?;
    state.with_ticket(&ticket, |_| Ok(()))?;
    let job_id = jobs
        .queue()
        .submit(task::GenerationTask::new(app, ticket, request, provider, key, permit))
        .to_string();
    Ok(Queued { request_id, job_id })
}

#[tauri::command]
pub fn narrative_generation_recover(
    state: State<'_, AppState>,
    request_id: Id,
) -> CommandResult<()> {
    state.with(|project| {
        let request = records::request(project, request_id)?;
        let (id, receipt) = records::attempts(project, &request)?
            .into_iter()
            .find(|(_, r)| {
                matches!(
                    r,
                    Receipt::NarrativeGenerationAttempt { status: AttemptStatus::Succeeded, .. }
                )
            })
            .ok_or_else(|| invalid("No successful result is available to publish."))?;
        records::publish(project, &request, id, &receipt)
    })
}

#[cfg(test)]
mod tests;

pub(super) fn submit(
    app: &AppHandle,
    ticket: &ProjectTicket,
    jobs: &Jobs,
    request: FrozenRequest,
    provider: Arc<dyn wobu_llm::TextProvider>,
    key: Arc<crate::keys::Secret>,
    permit: GenerationPermit,
) -> Queued {
    let request_id = request.request_id;
    let job_id = jobs
        .queue()
        .submit(task::GenerationTask::new(
            app.clone(),
            ticket.clone(),
            request,
            provider,
            key,
            permit,
        ))
        .to_string();
    Queued { request_id, job_id }
}
