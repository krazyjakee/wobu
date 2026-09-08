use super::*;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use wobu_jobs::{Config, JobState, Queue, Silent};
use wobu_llm::{
    Cancel, DeltaSink, EnhanceOutcome, EnhanceRequest, Error as ProviderError, StructuredOutcome,
    StructuredRequest, TextProvider, Usage,
};
use wobu_narrative::{Beat, DialogueSlot, GenerationPolicy, Speaker, Text, Variant};
use wobu_store::NarrativeRecordKind;

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("wobu-generation-{}", wobu_core::new_id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(temp: &Temp) -> (Project, plan::PlanInput) {
    let mut project = Project::create(&temp.0, "Harbour").unwrap();
    let mut file = project.create_scene("Rumour at the harbour").unwrap();
    let mut beat = Beat::new("Ask about the beacon");
    beat.dialogue.push(DialogueSlot::new(Speaker::Narrator));
    beat.outcomes.push(wobu_narrative::Outcome::new(wobu_narrative::Destination::End {
        label: "Rumour recorded".into(),
    }));
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();
    let input = plan::PlanInput {
        scene: file.scene.id,
        selection: None,
        state: BTreeMap::new(),
        commands: BTreeMap::new(),
        token_budget: 4000,
        max_output_tokens: 512,
    };
    (project, input)
}
fn freeze(project: &mut Project, input: plan::PlanInput) -> FrozenRequest {
    let request = plan::build(project, input, "fixture", "local-mock").unwrap().requests.remove(0);
    records::save_receipt(
        project,
        request.request_id,
        "Frozen narrative generation request",
        &Receipt::NarrativeGenerationRequest { request: Box::new(request.clone()) },
    )
    .unwrap();
    request
}
fn raw(request: &FrozenRequest) -> String {
    serde_json::json!({"lines":[{"slot_id":request.target.slot,"variant_id":request.candidate_variant_id,"speaker":request.speaker,"text":"They say the beacon went dark."}]}).to_string()
}

#[test]
fn planner_excludes_slot_and_variant_locks_and_refuses_appending_branches() {
    let temp = Temp::new();
    let (mut project, mut input) = fixture(&temp);
    let mut file = project.load_scene(input.scene).unwrap();
    file.scene.beats[0].dialogue[0].policy = GenerationPolicy::Locked;
    project.save_scene(&mut file).unwrap();
    let plan = plan::build(&project, input, "fixture", "model").unwrap();
    assert!(plan.requests.is_empty());
    assert_eq!(plan.skipped.len(), 1);
    file.scene.beats[0].dialogue[0].policy = GenerationPolicy::Edited;
    file.scene.beats[0].dialogue[0].variants.push(Variant::new(Text::written("Existing line")));
    project.save_scene(&mut file).unwrap();
    input = plan::PlanInput {
        scene: file.scene.id,
        selection: Some(wobu_narrative_context::Selection {
            scene: file.scene.id,
            beat: file.scene.beats[0].id,
            slot: file.scene.beats[0].dialogue[0].id,
            variant: None,
        }),
        state: BTreeMap::new(),
        commands: BTreeMap::new(),
        token_budget: 4000,
        max_output_tokens: 512,
    };
    assert!(plan::build(&project, input.clone(), "fixture", "model").is_err());
    let variant = &mut file.scene.beats[0].dialogue[0].variants[0];
    variant.text.lifecycle.policy = GenerationPolicy::Locked;
    input.selection.as_mut().unwrap().variant = Some(variant.id);
    project.save_scene(&mut file).unwrap();
    let locked = plan::build(&project, input, "fixture", "model").unwrap();
    assert!(locked.requests.is_empty());
    assert_eq!(locked.skipped.len(), 1);
}

#[derive(Clone)]
struct Store(Arc<Mutex<Project>>);
impl task::GenerationStore for Store {
    fn prepare(&self, request: &FrozenRequest) -> CommandResult<u32> {
        task::prepare(&self.0.lock(), request)
    }
    fn persist(&self, request: &FrozenRequest, id: Id, receipt: &Receipt) -> CommandResult<()> {
        task::persist(&mut self.0.lock(), request, id, receipt)
    }
}
enum Reply {
    Good(String),
    Bad,
    RateLimit,
    Wait,
    PartialRateLimit,
}
struct Provider {
    reply: Mutex<std::collections::VecDeque<Reply>>,
    calls: AtomicUsize,
    hook: Option<Box<dyn Fn() + Send + Sync>>,
}
#[async_trait]
impl TextProvider for Provider {
    fn id(&self) -> &'static str {
        "fixture"
    }
    fn label(&self) -> &'static str {
        "Fixture"
    }
    fn default_model(&self) -> &'static str {
        "local-mock"
    }
    fn supports_structured(&self) -> bool {
        true
    }
    async fn enhance(
        &self,
        _: &EnhanceRequest,
        _: &mut dyn DeltaSink,
        _: &Cancel,
    ) -> EnhanceOutcome {
        EnhanceOutcome::unbilled(ProviderError::Cancelled)
    }
    async fn structured(
        &self,
        _: &StructuredRequest,
        sink: &mut dyn DeltaSink,
        cancel: &Cancel,
    ) -> StructuredOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = &self.hook {
            hook();
        }
        let reply = self.reply.lock().pop_front().expect("Unexpected implicit provider retry");
        let result = match reply {
            Reply::Good(raw) => Ok(raw),
            Reply::Bad => Ok("{\"effects\":[{\"set\":\"win\"}]}".into()),
            Reply::RateLimit => Err(ProviderError::RateLimited {
                provider: "fixture",
                retry_after: Some(Duration::from_millis(1)),
            }),
            Reply::PartialRateLimit => {
                sink.delta("partial");
                Err(ProviderError::RateLimited {
                    provider: "fixture",
                    retry_after: Some(Duration::from_millis(1)),
                })
            }
            Reply::Wait => {
                sink.delta("partial");
                cancel.cancelled().await;
                Err(ProviderError::Cancelled)
            }
        };
        StructuredOutcome { usage: Usage::default(), result }
    }
}
fn provider(replies: Vec<Reply>) -> Arc<Provider> {
    Arc::new(Provider { reply: Mutex::new(replies.into()), calls: AtomicUsize::new(0), hook: None })
}
fn submit(
    queue: &Queue,
    store: &Store,
    request: &FrozenRequest,
    provider: Arc<Provider>,
) -> wobu_jobs::JobId {
    queue.submit(task::GenerationTask {
        store: store.clone(),
        request: request.clone(),
        provider,
        secret: Arc::new(crate::keys::Secret::new("not-a-real-provider-secret")),
        _permit: None,
    })
}
async fn terminal(queue: &Queue, id: wobu_jobs::JobId) -> JobState {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(job) = queue.snapshot().jobs.iter().find(|j| j.id == id)
                && job.state.is_terminal()
            {
                return job.state.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn partial_batch_retry_never_repeats_success_and_malformed_logic_is_discarded() {
    let temp = Temp::new();
    let (mut project, input) = fixture(&temp);
    let first = freeze(&mut project, input);
    let mut second = first.clone();
    second.request_id = wobu_core::new_id();
    records::save_receipt(
        &mut project,
        second.request_id,
        "Frozen narrative generation request",
        &Receipt::NarrativeGenerationRequest { request: Box::new(second.clone()) },
    )
    .unwrap();
    let before = project.narrative_fingerprint().unwrap();
    let store = Store(Arc::new(Mutex::new(project)));
    let queue = Queue::new(Config::default(), Silent);
    let good = provider(vec![Reply::Good(raw(&first))]);
    let bad = provider(vec![Reply::Bad]);
    assert!(matches!(
        terminal(&queue, submit(&queue, &store, &first, good.clone())).await,
        JobState::Done
    ));
    assert!(matches!(
        terminal(&queue, submit(&queue, &store, &second, bad.clone())).await,
        JobState::Failed { .. }
    ));
    assert_eq!(good.calls.load(Ordering::SeqCst), 1);
    assert_eq!(bad.calls.load(Ordering::SeqCst), 1);
    let history = history(&store.0.lock()).unwrap();
    assert_eq!(history.len(), 2);
    assert!(history.iter().any(|h| h.status == "invalid_output" && h.candidate.is_none()));
    assert!(matches!(
        terminal(&queue, submit(&queue, &store, &first, good.clone())).await,
        JobState::Failed { .. }
    ));
    assert_eq!(good.calls.load(Ordering::SeqCst), 1);
    let retry = provider(vec![Reply::Good(raw(&second))]);
    assert!(matches!(
        terminal(&queue, submit(&queue, &store, &second, retry)).await,
        JobState::Done
    ));
    assert_eq!(store.0.lock().narrative_fingerprint().unwrap(), before);
    assert_eq!(records::attempts(&store.0.lock(), &second).unwrap().len(), 2);
}

#[tokio::test]
async fn only_explicit_rate_limit_rejection_retries_and_shutdown_retains_interrupted_evidence() {
    let temp = Temp::new();
    let (mut project, input) = fixture(&temp);
    let request = freeze(&mut project, input);
    let store = Store(Arc::new(Mutex::new(project)));
    let queue = Queue::new(Config::default(), Silent);
    let rejected = provider(vec![Reply::RateLimit, Reply::Good(raw(&request))]);
    assert!(matches!(
        terminal(&queue, submit(&queue, &store, &request, rejected.clone())).await,
        JobState::Done
    ));
    assert_eq!(rejected.calls.load(Ordering::SeqCst), 2);
    let mut next = request.clone();
    next.request_id = wobu_core::new_id();
    records::save_receipt(
        &mut store.0.lock(),
        next.request_id,
        "Frozen narrative generation request",
        &Receipt::NarrativeGenerationRequest { request: Box::new(next.clone()) },
    )
    .unwrap();
    let partial = provider(vec![Reply::PartialRateLimit]);
    assert!(matches!(
        terminal(&queue, submit(&queue, &store, &next, partial.clone())).await,
        JobState::Failed { .. }
    ));
    assert_eq!(partial.calls.load(Ordering::SeqCst), 1);
    let waiting = provider(vec![Reply::Wait]);
    let job = submit(&queue, &store, &next, waiting.clone());
    tokio::time::timeout(Duration::from_secs(2), async {
        while waiting.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    queue.close();
    assert!(queue.quiesce(Duration::from_secs(2)).await);
    assert!(matches!(terminal(&queue, job).await, JobState::Cancelled));
    let history = history(&store.0.lock()).unwrap();
    let row = history.iter().find(|h| h.request_id == next.request_id).unwrap();
    assert_eq!(row.status, "cancelled");
    assert!(row.billing_unknown);
}

#[tokio::test]
async fn edits_and_locks_during_provider_call_become_conflicted_proposals() {
    for locked in [false, true] {
        let temp = Temp::new();
        let (mut project, input) = fixture(&temp);
        let request = freeze(&mut project, input);
        let store = Store(Arc::new(Mutex::new(project)));
        let changed = store.clone();
        let scene = request.target.scene;
        let provider = Arc::new(Provider {
            reply: Mutex::new(vec![Reply::Good(raw(&request))].into()),
            calls: AtomicUsize::new(0),
            hook: Some(Box::new(move || {
                let mut project = changed.0.lock();
                let mut file = project.load_scene(scene).unwrap();
                if locked {
                    file.scene.beats[0].dialogue[0].policy = GenerationPolicy::Locked;
                } else {
                    file.scene.beats[0].dialogue[0]
                        .variants
                        .push(Variant::new(Text::written("Human correction")));
                }
                project.save_scene(&mut file).unwrap();
            })),
        });
        let queue = Queue::new(Config::default(), Silent);
        assert!(matches!(
            terminal(&queue, submit(&queue, &store, &request, provider)).await,
            JobState::Done
        ));
        let project = store.0.lock();
        let attempts = records::attempts(&project, &request).unwrap();
        let publication = project.narrative_publication(attempts[0].0).unwrap().unwrap();
        let proposal =
            publication.records.iter().find(|r| r.kind == NarrativeRecordKind::Proposal).unwrap();
        assert_eq!(proposal.payload["publication_checks"]["scene_unchanged"], false);
        if locked {
            assert_eq!(proposal.payload["publication_checks"]["locked_now"], true);
        } else {
            assert_eq!(
                project.load_scene(scene).unwrap().scene.beats[0].dialogue[0].variants[0].text.body,
                "Human correction"
            );
        }
    }
}

#[test]
fn successful_receipt_survives_unpublished_restart_without_paid_retry() {
    let temp = Temp::new();
    let (mut project, input) = fixture(&temp);
    let request = freeze(&mut project, input);
    let id = wobu_core::new_id();
    let receipt = success_receipt(&request);
    records::save_receipt(&mut project, id, "Narrative generation attempt", &receipt).unwrap();
    assert!(task::prepare(&project, &request).is_err());
    assert!(!history(&project).unwrap()[0].proposal_published);
    let root = project.root().to_path_buf();
    drop(project);
    let mut project = Project::open(&root).unwrap();
    records::publish(&mut project, &request, id, &receipt).unwrap();
    let stamp = project.narrative_publication(id).unwrap().unwrap().stamp;
    records::publish(&mut project, &request, id, &receipt).unwrap();
    assert_eq!(stamp, project.narrative_publication(id).unwrap().unwrap().stamp);
    assert!(history(&project).unwrap()[0].proposal_published);
}

#[tokio::test]
async fn queued_cancellation_spends_no_request_and_keeps_recoverable_intent() {
    let temp = Temp::new();
    let (mut project, input) = fixture(&temp);
    let first = freeze(&mut project, input);
    let mut queued = first.clone();
    queued.request_id = wobu_core::new_id();
    records::save_receipt(
        &mut project,
        queued.request_id,
        "Frozen narrative generation request",
        &Receipt::NarrativeGenerationRequest { request: Box::new(queued.clone()) },
    )
    .unwrap();
    let store = Store(Arc::new(Mutex::new(project)));
    let queue = Queue::new(Config { concurrency: 1, ..Config::default() }, Silent);
    let first_provider = provider(vec![Reply::Wait]);
    submit(&queue, &store, &first, first_provider.clone());
    tokio::time::timeout(Duration::from_secs(2), async {
        while first_provider.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    let unused = provider(vec![]);
    let pending = submit(&queue, &store, &queued, unused.clone());
    assert!(queue.cancel(pending));
    assert!(matches!(terminal(&queue, pending).await, JobState::Cancelled));
    assert_eq!(unused.calls.load(Ordering::SeqCst), 0);
    assert!(records::attempts(&store.0.lock(), &queued).unwrap().is_empty());
    assert_eq!(
        history(&store.0.lock())
            .unwrap()
            .iter()
            .find(|h| h.request_id == queued.request_id)
            .unwrap()
            .status,
        "interrupted"
    );
    queue.close();
    assert!(queue.quiesce(Duration::from_secs(2)).await);
}

struct SessionStore {
    store: Store,
    valid: Arc<std::sync::atomic::AtomicBool>,
}
impl task::GenerationStore for SessionStore {
    fn prepare(&self, request: &FrozenRequest) -> CommandResult<u32> {
        task::prepare(&self.store.0.lock(), request)
    }
    fn persist(&self, request: &FrozenRequest, id: Id, receipt: &Receipt) -> CommandResult<()> {
        if !self.valid.load(Ordering::SeqCst) {
            return Err(WobuError::no_project_open());
        }
        task::persist(&mut self.store.0.lock(), request, id, receipt)
    }
}
#[tokio::test]
async fn lost_project_session_rejects_late_publication_without_retrying_provider() {
    let temp = Temp::new();
    let (mut project, input) = fixture(&temp);
    let request = freeze(&mut project, input);
    let before = project.narrative_fingerprint().unwrap();
    let store = Store(Arc::new(Mutex::new(project)));
    let valid = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let invalidate = valid.clone();
    let provider = Arc::new(Provider {
        reply: Mutex::new(vec![Reply::Good(raw(&request))].into()),
        calls: AtomicUsize::new(0),
        hook: Some(Box::new(move || invalidate.store(false, Ordering::SeqCst))),
    });
    let queue = Queue::new(Config::default(), Silent);
    let job = queue.submit(task::GenerationTask {
        store: SessionStore { store: store.clone(), valid },
        request: request.clone(),
        provider: provider.clone(),
        secret: Arc::new(crate::keys::Secret::new("not-a-real-provider-secret")),
        _permit: None,
    });
    assert!(matches!(terminal(&queue, job).await, JobState::Failed { .. }));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let project = store.0.lock();
    assert!(records::attempts(&project, &request).unwrap().is_empty());
    assert_eq!(project.narrative_fingerprint().unwrap(), before);
}

#[test]
fn incomplete_or_wrong_proposal_pair_is_never_reported_as_published_or_regenerated() {
    let temp = Temp::new();
    let (mut project, input) = fixture(&temp);
    let request = freeze(&mut project, input);
    let id = wobu_core::new_id();
    let receipt = success_receipt(&request);
    task::persist(&mut project, &request, id, &receipt).unwrap();
    let complete = project.narrative_publication(id).unwrap().unwrap();
    let receipt_only = complete
        .records
        .iter()
        .filter(|r| r.kind == NarrativeRecordKind::Receipt)
        .cloned()
        .collect::<Vec<_>>();
    project
        .publish_narrative_records(
            id,
            "Incomplete generation result",
            &receipt_only,
            Some(&complete.stamp),
        )
        .unwrap();
    let incomplete = project.narrative_publication(id).unwrap().unwrap();
    assert!(records::publish(&mut project, &request, id, &receipt).is_err());
    assert!(history(&project).is_err());
    assert!(task::prepare(&project, &request).is_err());
    assert_eq!(incomplete.stamp, project.narrative_publication(id).unwrap().unwrap().stamp);
    let mut mismatched = complete.records;
    let proposal = mismatched.iter_mut().find(|r| r.kind == NarrativeRecordKind::Proposal).unwrap();
    proposal.payload["request_id"] = serde_json::json!(wobu_core::new_id());
    project
        .publish_narrative_records(
            id,
            "Unrelated generation proposal",
            &mismatched,
            Some(&incomplete.stamp),
        )
        .unwrap();
    assert!(records::publish(&mut project, &request, id, &receipt).is_err());
    assert!(history(&project).is_err());
}

fn success_receipt(request: &FrozenRequest) -> Receipt {
    let raw = raw(request);
    Receipt::NarrativeGenerationAttempt {
        version: 1,
        request_id: request.request_id,
        request_hash: request.hash(),
        attempt: 1,
        status: AttemptStatus::Succeeded,
        usage: Default::default(),
        billing_unknown: false,
        error_code: None,
        candidate: Some(request.validate_output(&raw).unwrap()),
        raw_accepted_output: Some(raw),
    }
}
