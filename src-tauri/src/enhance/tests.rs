use super::*;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use serde_json::json;
use wobu_llm::{DeltaSink, EnhanceOutcome};

/// A one-thread executor, the same shape `wobu-llm`'s tests use. The shell
/// has no async test harness and adding one to run four tests would be a
/// dependency bought with very little.
fn block_on<F: Future>(future: F) -> F::Output {
    struct Unparker(std::thread::Thread);
    impl Wake for Unparker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(Unparker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        std::thread::park();
    }
}

/// How the fake's call ends. The same axis as `wobu-llm`'s fake provider,
/// because these are the endings a real one has.
#[derive(Clone, Copy, PartialEq)]
enum Ending {
    Whole,
    /// A `max_tokens` stop: most of a document arrived and was paid for.
    CutShort,
}

struct FakeProvider {
    ending: Ending,
    chunks_sent: Arc<AtomicUsize>,
}

#[async_trait]
impl TextProvider for FakeProvider {
    fn id(&self) -> &'static str {
        "fake"
    }
    fn label(&self) -> &'static str {
        "Fake"
    }
    fn default_model(&self) -> &'static str {
        "fake-1"
    }

    async fn enhance(
        &self,
        request: &EnhanceRequest,
        deltas: &mut dyn DeltaSink,
        cancel: &Cancel,
    ) -> EnhanceOutcome {
        let mut usage = Usage { input_tokens: 812, ..Usage::default() };
        let document = document_matching(request.schema());
        let chunks: Vec<String> =
            document.chars().collect::<Vec<_>>().chunks(9).map(|c| c.iter().collect()).collect();
        let deliver = match self.ending {
            Ending::Whole => chunks.len(),
            Ending::CutShort => chunks.len() * 2 / 3,
        };

        let mut streamed = String::new();
        for chunk in chunks.iter().take(deliver) {
            if cancel.is_cancelled() {
                return EnhanceOutcome::new(usage, Err(ProviderError::Cancelled));
            }
            deltas.delta(chunk);
            streamed.push_str(chunk);
            usage.output_tokens += 1;
            self.chunks_sent.fetch_add(1, Ordering::SeqCst);
        }

        let result = match self.ending {
            Ending::Whole => wobu_llm::parse_description(request.kind, &streamed),
            // Reported without being handed to the validator: a truncated
            // document that happened to parse would otherwise be written.
            Ending::CutShort => Err(ProviderError::Truncated),
        };
        EnhanceOutcome::new(usage, result)
    }
}

/// A response built from the schema it was handed, so a registry the
/// validator would reject fails here rather than on a paid call.
fn document_matching(schema: Value) -> String {
    let mut object = Map::new();
    for (key, property) in schema["properties"].as_object().unwrap() {
        let value = match property["type"].as_str().unwrap() {
            "string" => json!("Ash-glazed ceramic plate over oiled leather."),
            "array" if property["items"]["pattern"].is_string() => {
                json!(["#2b2118", "#c2703a"])
            }
            "array" if key == QUESTIONS_KEY => json!(["What is on the guild signet?"]),
            _ => json!(["Ember-lit throat vents"]),
        };
        object.insert(key.clone(), value);
    }
    serde_json::to_string(&Value::Object(object)).unwrap()
}

/// A task over a fake provider, with the deltas collected instead of
/// emitted. Returns the task, the frames it painted, and how many chunks the
/// provider produced — which is how "the work stopped" is told apart from
/// "the deltas stopped".
fn task(ending: Ending) -> (EnhanceTask, Arc<Mutex<Vec<EnhanceDelta>>>, Arc<AtomicUsize>) {
    let frames: Arc<Mutex<Vec<EnhanceDelta>>> = Arc::default();
    let seen = Arc::clone(&frames);
    let chunks_sent = Arc::new(AtomicUsize::new(0));
    let task = EnhanceTask {
        node_id: wobu_core::new_id(),
        kind: NodeKind::Character,
        label: "Enhance Kael Vantris".into(),
        project: wobu_core::new_id(),
        request: EnhanceRequest::new(NodeKind::Character, "fake-1", "Describe Kael.")
            .with_system(SYSTEM),
        sources: vec![wobu_core::new_id()],
        provider: Arc::new(FakeProvider { ending, chunks_sent: Arc::clone(&chunks_sent) }),
        pending: Pending::default(),
        emit: Arc::new(move |delta| seen.lock().push(delta)),
    };
    (task, frames, chunks_sent)
}

#[test]
fn a_finished_call_leaves_a_description_waiting_rather_than_writing_one() {
    // Step 4 of the pipeline is "on accept", and this is the half before it.
    // The task holds no project handle at all, so there is no path from here
    // to a node file — a description becomes canon only when somebody says
    // so, through `enhance_accept`.
    let (task, _, _) = task(Ending::Whole);
    let job = JobId::new();

    let Outcome::Done(Some(ready)) = block_on(task.attempt(job, &Cancel::new())) else {
        panic!("a whole response should finish");
    };

    let waiting = task.pending.get(job).expect("the description is waiting");
    assert_eq!(waiting.node_id, task.node_id);
    assert_eq!(waiting.sources, task.sources, "the stamp is made of the walk it was built from");
    assert!(waiting.description.sections.contains_key("never"));

    // And the questions rode out beside the description rather than inside
    // it, so nothing ever asks an image model what is on the guild signet.
    assert_eq!(ready["questions"][0], "What is on the guild signet?");
    assert!(ready["description"]["sections"].get(QUESTIONS_KEY).is_none(), "{ready}");
    assert!(!waiting.description.sections.contains_key(QUESTIONS_KEY));
    assert_eq!(ready["nodeId"], task.node_id.to_string());
    assert_eq!(ready["jobId"], job.to_string());
}

#[test]
fn a_partial_stream_reaches_the_editor_and_never_the_waiting_room() {
    // The rule this module is built around. Most of a document arrived, the
    // pane drew it, and the provider billed for every token of it — and none
    // of that makes it a description. The only route to one is
    // `EnhanceOutcome::result`, and a truncation is not `Ok`.
    let (task, frames, _) = task(Ending::CutShort);
    let job = JobId::new();

    let outcome = block_on(task.attempt(job, &Cancel::new()));

    assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    assert!(task.pending.get(job).is_none(), "half a description was left to be accepted");
    let painted = frames.lock();
    assert!(painted.len() > 1, "the editor was shown nothing");
    assert!(
        painted.last().unwrap().description.sections.values().any(|v| !v.is_empty()),
        "the pane should still be showing what did arrive",
    );
}

#[test]
fn the_last_frame_is_the_whole_document_however_the_repaints_were_coalesced() {
    // The frames are throttled, so whatever arrives inside the last forty
    // milliseconds — which is the end of the description — would be
    // swallowed without the final emit. A pane that stops one sentence short
    // and stays there looks exactly like a truncated response.
    let (task, frames, _) = task(Ending::Whole);
    let job = JobId::new();

    let outcome = block_on(task.attempt(job, &Cancel::new()));
    assert!(matches!(outcome, Outcome::Done(_)));

    let painted = frames.lock();
    let last = painted.last().expect("at least one frame");
    let waiting = task.pending.get(job).unwrap();
    assert_eq!(
        last.description, waiting.description,
        "the editor and the node disagree about what was written",
    );
    assert_eq!(last.questions, ["What is on the guild signet?"]);
    assert_eq!(last.job_id, job);
    assert_eq!(last.node_id, task.node_id);
}

#[test]
fn stopping_an_enhance_is_a_cancellation_rather_than_a_failure() {
    // A cancellation reported as a failure would be run through the retry
    // rules, and a retryable one would start a fresh paid attempt for
    // somebody who pressed Stop.
    let (task, _, chunks) = task(Ending::Whole);
    let cancel = Cancel::new();
    cancel.cancel();

    let outcome = block_on(task.attempt(JobId::new(), &cancel));

    assert!(matches!(outcome, Outcome::Cancelled), "{outcome:?}");
    assert_eq!(chunks.load(Ordering::SeqCst), 0, "the provider was asked anyway");
}

#[test]
fn a_response_that_was_paid_for_and_came_back_broken_is_handed_up_not_repeated() {
    // The queue's rule, exercised through the failure this task actually
    // produces. `Truncated` is retryable — it could work — but the provider
    // billed for every token it generated, so the decision to spend again
    // belongs to the person paying. Reporting `Billed::Nothing` here is what
    // would quietly take it away from them.
    let (task, _, _) = task(Ending::CutShort);

    let Outcome::Failed(failed) = block_on(task.attempt(JobId::new(), &Cancel::new())) else {
        panic!("a truncated response should fail");
    };

    assert_eq!(failed.code, "provider.bad_response");
    assert!(failed.retryable);
    assert_eq!(failed.billed, wobu_jobs::Billed::Charged);
    assert!(failed.cost_note.is_some(), "the offer has to say what again would cost");
    assert_eq!(
        wobu_jobs::decide(
            &wobu_jobs::RetryPolicy::default(),
            wobu_jobs::Attempts { total: 1, paid: 0 },
            &failed,
        ),
        wobu_jobs::Verdict::Hold,
    );
}

#[test]
fn a_failure_that_cost_nothing_says_so_and_is_retried_for_free() {
    // The other half: a rate limit generated nothing, so the queue should
    // ride it out on its own rather than putting a dialog in front of a blip.
    let free = failure(
        &ProviderError::RateLimited { provider: "Anthropic", retry_after: None },
        Usage::default(),
    );
    assert_eq!(free.billed, wobu_jobs::Billed::Nothing);
    assert!(free.cost_note.is_none(), "nothing was spent, so there is nothing to quote");
    assert!(matches!(
        wobu_jobs::decide(
            &wobu_jobs::RetryPolicy::default(),
            wobu_jobs::Attempts { total: 1, paid: 0 },
            &free,
        ),
        wobu_jobs::Verdict::Free(_),
    ));
}

#[test]
fn an_answered_description_is_forgotten_and_an_old_unanswered_one_is_dropped() {
    // Nothing but an answer clears this, so without the bound a long session
    // would hold every description it ever produced.
    let pending = Pending::default();
    let ready = |job| Ready {
        job,
        project: wobu_core::new_id(),
        node_id: wobu_core::new_id(),
        description: Description::default(),
        questions: vec![],
        sources: vec![],
    };

    let first = JobId::new();
    pending.remember(ready(first));
    assert!(pending.get(first).is_some());
    // Read, not taken: a `RefusedEdit` is answered by calling accept again
    // with `force`, and there is no second copy of the answer anywhere.
    assert!(pending.get(first).is_some(), "reading it consumed it");
    pending.forget(first);
    assert!(pending.get(first).is_none());

    let mut ids = Vec::new();
    for _ in 0..KEPT + 3 {
        let id = JobId::new();
        pending.remember(ready(id));
        ids.push(id);
    }
    assert!(pending.get(ids[0]).is_none(), "the oldest should have been dropped");
    assert!(pending.get(*ids.last().unwrap()).is_some(), "the newest is still there");

    pending.clear();
    assert!(pending.get(*ids.last().unwrap()).is_none());
}

#[test]
fn a_reloaded_pane_can_ask_for_the_answer_this_process_is_still_holding() {
    // The one failure in this pipeline that costs money to recover from. A
    // webview reload loses the `job:done` that carried the description, and
    // without a way to ask for it again the only way back is running the
    // call — and paying for it — a second time.
    let pending = Pending::default();
    let (ashfall, other) = (wobu_core::new_id(), wobu_core::new_id());
    let waiting = |project, questions: &[&str]| Ready {
        job: JobId::new(),
        project,
        node_id: wobu_core::new_id(),
        description: Description::from_sections([(
            "silhouette".to_string(),
            SectionValue::Text("Tall, narrow, hooded".into()),
        )]),
        questions: questions.iter().map(|q| (*q).to_string()).collect(),
        sources: vec![wobu_core::new_id()],
    };

    let first = waiting(ashfall, &["What is on the guild signet?"]);
    let (job, node) = (first.job, first.node_id);
    pending.remember(first);
    pending.remember(waiting(ashfall, &[]));
    pending.remember(waiting(other, &[]));

    let listed = pending.list(ashfall);
    assert_eq!(listed.len(), 2, "the other project's description is not this one's");
    let found = listed.iter().find(|r| r.node_id == node).expect("matched by node");
    // Both halves come back, because losing the questions would leave the
    // user re-running the call to find out what the model could not settle.
    assert_eq!(found.description.text("silhouette"), Some("Tall, narrow, hooded"));
    assert_eq!(found.questions, ["What is on the guild signet?"]);
    // And the id to answer with, which a reloaded pane no longer has.
    assert_eq!(found.job_id, job);

    // Reading the list does not consume it — the pane may reload twice.
    assert_eq!(pending.list(ashfall).len(), 2);
    assert!(pending.list(wobu_core::new_id()).is_empty(), "a world nobody is in");
}

#[test]
fn a_pending_entry_matches_the_enhanceready_interface() {
    // The same shape rides `job:done` and comes back from `enhance_pending`,
    // so the pane renders one component either way. A rename noticed by
    // neither side arrives as `undefined`.
    let pending = Pending::default();
    let project = wobu_core::new_id();
    pending.remember(Ready {
        job: JobId::new(),
        project,
        node_id: wobu_core::new_id(),
        description: Description::default(),
        questions: vec!["What is on the guild signet?".into()],
        sources: vec![],
    });

    let json = serde_json::to_value(pending.list(project)).unwrap();
    for key in ["jobId", "nodeId", "description", "questions"] {
        assert!(json[0].get(key).is_some(), "`{key}` is missing from EnhanceReady");
    }
    assert!(json[0]["jobId"].is_string(), "a job id must cross as a string");
}

/* ── the provider selection ───────────────────────────────────────────── */

#[test]
fn a_project_that_names_a_provider_is_not_quietly_given_a_different_one() {
    // The regression worth having: hardcoding Anthropic here would bill the
    // wrong vendor, read the wrong keychain entry, and be invisible until
    // somebody wondered why their Gemini key was never used.
    let providers = json!({
        "text": { "provider": "gemini", "model": "gemini-3.6-flash" },
        "image": { "provider": "comfyui" },
    });
    assert_eq!(
        selection(providers.as_object().unwrap()),
        Selection { provider: "gemini".into(), model: Some("gemini-3.6-flash".into()) }
    );
}

#[test]
fn a_project_that_names_nothing_gets_the_default_text_provider_and_no_model() {
    // Every project created before there is a settings pane for this is in
    // this state, and Enhance has to work in it. An absent model is the
    // adapter's own default rather than a string spelled out here, because
    // model ids move faster than anything else in `docs/08-providers.md`.
    for empty in [
        json!({}),
        json!({ "image": { "provider": "comfyui" } }),
        json!({
            "text": { "model": "  " }
        }),
    ] {
        let selection = selection(empty.as_object().unwrap());
        assert_eq!(selection.provider, anthropic::ID);
        assert_eq!(selection.model, None, "{empty}");
    }
}

#[test]
fn every_adapter_this_build_has_can_be_built_and_named_without_a_network() {
    // Both halves of the two-table split: a provider that can be constructed
    // but not named would leave the "no key on this machine" message saying
    // nothing useful, and one that can be named but not constructed would
    // fail after the key had already been read out of the keychain.
    for id in [anthropic::ID, gemini::ID] {
        let provider = text_provider(id, &Secret::new("not-a-real-key")).unwrap();
        assert_eq!(provider.id(), id);
        assert_eq!(label_of(id), provider.label());
        assert!(!provider.default_model().is_empty());
    }

    let Err(unknown) = text_provider("openai", &Secret::new("k")) else {
        panic!("a provider this build does not have cannot be built");
    };
    assert_eq!(serde_json::to_value(&unknown).unwrap()["code"], "node.invalid");
    assert!(no_key("gemini").message.contains("Gemini"), "the message names the vendor");
    assert_eq!(
        serde_json::to_value(no_key("gemini")).unwrap()["retryable"],
        false,
        "trying again without pasting a key fails identically",
    );
}

/* ── the bridge ───────────────────────────────────────────────────────── */

#[test]
fn a_delta_matches_the_enhancedelta_interface() {
    // Hand-written TypeScript on the far side, so a serde rename nothing
    // noticed arrives in the pane as `undefined` rather than as an error.
    let (task, _, _) = task(Ending::Whole);
    let delta = task.delta(JobId::new(), r#"{"silhouette":"Tall, narrow-should"#);
    let json = serde_json::to_value(&delta).unwrap();

    for key in ["jobId", "nodeId", "description", "questions"] {
        assert!(json.get(key).is_some(), "`{key}` is missing from EnhanceDelta");
    }
    assert!(json["jobId"].is_string(), "a job id must cross as a string");
    // The same tagged `SectionValue` shape a node's description crosses in,
    // so the pane renders a half-written description with the component it
    // already has rather than a second one.
    assert_eq!(json["description"]["sections"]["silhouette"]["type"], "text");
    assert_eq!(json["description"]["sections"]["silhouette"]["value"], "Tall, narrow-should",);
}

#[test]
fn a_refused_edit_reaches_the_webview_as_the_question_it_is() {
    // `edited` must never be silently overwritten, and the shape is how the
    // UI can tell "your hand-written description is about to go" from
    // "something failed". A failure is what an `Err` here would look like,
    // and the answer to it would be a dismissed dialog rather than a choice.
    let node = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
    let refused = Accepted::RefusedEdit { node: Box::new(node.clone()) };
    let json = serde_json::to_value(&refused).unwrap();

    assert_eq!(json["outcome"], "refusedEdit");
    assert_eq!(json["node"]["name"], "Kael Vantris");
    assert_eq!(
        serde_json::to_value(Accepted::Saved { node: Box::new(node) }).unwrap()["outcome"],
        "saved",
    );
}

#[test]
fn a_job_id_survives_the_round_trip_the_webview_puts_it_through() {
    // `enhance_start` returns a string and `enhance_accept` takes one back,
    // because that is what JSON has. An id that could not be parsed back
    // would make every description unacceptable.
    let id = JobId::new();
    assert_eq!(job_id_of(&id.to_string()).unwrap(), id);
    assert_eq!(
        serde_json::to_value(job_id_of("not-a-job").unwrap_err()).unwrap()["code"],
        "internal",
    );
}
