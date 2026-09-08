use super::*;
use async_trait::async_trait;
use std::sync::Arc;
use tauri::Manager;
use wobu_jobs::{Billed, Failure, JobContext, JobKind, Outcome, Progress, Task};
use wobu_llm::{Error as ProviderError, StructuredRequest, TextProvider};
use wobu_narrative_generation::{TokenUsage, VERSION};

pub trait GenerationStore: Send + 'static {
    fn prepare(&self, request: &FrozenRequest) -> CommandResult<u32>;
    fn persist(&self, request: &FrozenRequest, id: Id, receipt: &Receipt) -> CommandResult<()>;
}
pub struct DesktopStore {
    app: AppHandle,
    ticket: ProjectTicket,
}
impl GenerationStore for DesktopStore {
    fn prepare(&self, request: &FrozenRequest) -> CommandResult<u32> {
        self.app.state::<AppState>().with_ticket(&self.ticket, |project| prepare(project, request))
    }
    fn persist(&self, request: &FrozenRequest, id: Id, receipt: &Receipt) -> CommandResult<()> {
        self.app
            .state::<AppState>()
            .with_ticket(&self.ticket, |project| persist(project, request, id, receipt))
    }
}
pub fn prepare(project: &Project, request: &FrozenRequest) -> CommandResult<u32> {
    if project.is_read_only() {
        return Err(wobu_store::Error::ReadOnly.into());
    }
    request.validate().map_err(invalid)?;
    let recorded = records::request(project, request.request_id)?;
    if recorded != *request {
        return Err(invalid("Frozen request changed before execution."));
    }
    let attempts = records::attempts(project, request)?;
    if attempts.iter().any(|(_, r)| {
        matches!(r, Receipt::NarrativeGenerationAttempt { status: AttemptStatus::Succeeded, .. })
    }) {
        return Err(invalid(
            "This request already succeeded; no additional provider call was made.",
        ));
    }
    if !records::checks(project, request)?.current() {
        return Err(invalid("Source, text or policy changed before the provider call."));
    }
    attempts
        .last()
        .map_or(Some(1), |(_, r)| match r {
            Receipt::NarrativeGenerationAttempt { attempt, .. } => attempt.checked_add(1),
            _ => None,
        })
        .ok_or_else(|| invalid("Attempt counter is exhausted."))
}
pub fn persist(
    project: &mut Project,
    request: &FrozenRequest,
    id: Id,
    receipt: &Receipt,
) -> CommandResult<()> {
    records::save_receipt(project, id, "Narrative generation attempt", receipt)?;
    if matches!(
        receipt,
        Receipt::NarrativeGenerationAttempt { status: AttemptStatus::Succeeded, .. }
    ) {
        records::publish(project, request, id, receipt)?;
    }
    Ok(())
}

pub struct GenerationTask<S = DesktopStore> {
    pub store: S,
    pub request: FrozenRequest,
    pub provider: Arc<dyn TextProvider>,
    /// Only in process memory. Never Debug/Serialize or sent in a prompt.
    pub secret: Arc<crate::keys::Secret>,
    pub _permit: Option<GenerationPermit>,
}
impl GenerationTask {
    pub fn new(
        app: AppHandle,
        ticket: ProjectTicket,
        request: FrozenRequest,
        provider: Arc<dyn TextProvider>,
        secret: Arc<crate::keys::Secret>,
        permit: GenerationPermit,
    ) -> Self {
        Self {
            store: DesktopStore { app, ticket },
            request,
            provider,
            secret,
            _permit: Some(permit),
        }
    }
}
#[async_trait]
impl<S: GenerationStore> Task for GenerationTask<S> {
    fn kind(&self) -> JobKind {
        JobKind::Narrative
    }
    fn subject_id(&self) -> Option<String> {
        Some(self.request.request_id.to_string())
    }
    fn label(&self) -> String {
        format!("Dialogue · {} · {}", self.request.provider, self.request.target.slot)
    }
    async fn run(&mut self, ctx: &JobContext) -> Outcome {
        let attempt = match self.store.prepare(&self.request) {
            Ok(attempt) => attempt,
            Err(_) => {
                return Outcome::Failed(Failure::new(
                    "narrative.preflight",
                    "The request is no longer eligible or its project session changed. No provider call was made.",
                ));
            }
        };
        if ctx.is_cancelled() {
            return Outcome::Cancelled;
        }
        if serde_json::to_string(&self.request)
            .is_ok_and(|text| text.contains(self.secret.expose()))
        {
            return Outcome::Failed(Failure::new(
                "narrative.credential",
                "A configured credential appeared in narrative context. No provider call was made.",
            ));
        }
        let provider_request = StructuredRequest {
            model: self.request.model.clone(),
            system: Some(self.request.system.clone()),
            prompt: self.request.prompt.clone(),
            schema: self.request.output_schema.clone(),
            max_output_tokens: self.request.settings.max_output_tokens,
        };
        ctx.progress(Progress::new(0, 1).with_note("Requesting one dialogue line"));
        let mut got_output = false;
        let mut deltas = |part: &str| {
            got_output |= !part.is_empty();
        };
        let outcome = self.provider.structured(&provider_request, &mut deltas, ctx.cancel()).await;
        let usage = outcome.usage;
        let mut failure = None;
        let (status, raw, candidate, error_code, billing_unknown) = match outcome.result {
            Ok(raw) => {
                let validated = if raw.contains(self.secret.expose()) {
                    Err("Provider output contained a credential.".into())
                } else {
                    self.request.validate_output(&raw)
                };
                match validated {
                    Ok(candidate) if !candidate.text.contains(self.secret.expose()) => (
                        AttemptStatus::Succeeded,
                        Some(raw),
                        Some(candidate),
                        None,
                        usage.total_tokens() == 0,
                    ),
                    Ok(_) | Err(_) => {
                        failure=Some(Failure::new("narrative.invalid_output","Provider output did not match the authorised dialogue contract. Accepted text was preserved.").billed(Billed::Unknown));
                        (
                            AttemptStatus::InvalidOutput,
                            None,
                            None,
                            Some("narrative.invalid_output".into()),
                            true,
                        )
                    }
                }
            }
            Err(error) => {
                let cancelled = matches!(error, ProviderError::Cancelled);
                // Only an explicit rejection before any output can retry automatically.
                let rejected = matches!(error, ProviderError::RateLimited { .. })
                    && !got_output
                    && usage.total_tokens() == 0;
                let code = if cancelled {
                    "job.cancelled"
                } else if rejected {
                    "provider.rate_limited"
                } else if matches!(error, ProviderError::RateLimited { .. }) {
                    "provider.unavailable"
                } else {
                    error.code()
                };
                if !cancelled {
                    let mut one = Failure::new(
                        code,
                        "The dialogue provider request failed. No accepted text was changed.",
                    )
                    .billed(if rejected { Billed::Nothing } else { Billed::Unknown })
                    .retryable(rejected);
                    if let ProviderError::RateLimited { retry_after: Some(delay), .. } = error {
                        one = one.after(delay);
                    }
                    failure = Some(one);
                }
                (
                    if cancelled { AttemptStatus::Cancelled } else { AttemptStatus::Failed },
                    None,
                    None,
                    Some(code.into()),
                    !rejected,
                )
            }
        };
        let cancelled = status == AttemptStatus::Cancelled;
        let receipt = Receipt::NarrativeGenerationAttempt {
            version: VERSION,
            request_id: self.request.request_id,
            request_hash: self.request.hash(),
            attempt,
            status,
            usage: TokenUsage {
                input: usage.input_tokens,
                cached_input: usage.cached_input_tokens,
                output: usage.output_tokens,
            },
            billing_unknown,
            error_code,
            raw_accepted_output: raw,
            candidate,
        };
        if self.store.persist(&self.request, wobu_core::new_id(), &receipt).is_err() {
            return Outcome::Failed(Failure::new("narrative.persistence","The provider attempt finished, but its result could not be fully published. Inspect retained history before retrying; billing may have occurred.").billed(Billed::Unknown));
        }
        if cancelled {
            return Outcome::Cancelled;
        }
        if let Some(failure) = failure {
            return Outcome::Failed(failure);
        }
        ctx.progress(Progress::new(1, 1).with_note("Proposal retained for review"));
        Outcome::Done(Some(serde_json::json!({"request_id":self.request.request_id})))
    }
}
