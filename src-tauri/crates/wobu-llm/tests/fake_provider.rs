//! An implementation of [`TextProvider`] that never touches a network, and the
//! behaviour every real adapter has to reproduce.
//!
//! It lives outside the crate on purpose: an in-crate fake can reach for private
//! items, so it would prove the trait is implementable *here* rather than
//! implementable at all. Everything below uses only what `wobu-llm` exports,
//! which is the same position the Anthropic ([#34](https://github.com/krazyjakee/wobu/issues/34))
//! and Gemini ([#35](https://github.com/krazyjakee/wobu/issues/35)) adapters
//! will be in.
//!
//! The fake streams a document built from the schema it was handed rather than a
//! canned string, so a schema that drifts out of what the validator accepts
//! fails here rather than in front of a user with a paid call behind it.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use serde_json::{Map, Value, json};
use wobu_core::kind::NodeKind;
use wobu_core::schema::description_schema;
use wobu_llm::{
    Cancel, DeltaSink, Discard, EnhanceOutcome, EnhanceRequest, Error, TextProvider, Usage,
};

/// A one-thread executor, so the trait's async surface can be exercised without
/// a runtime dependency. `wobu-llm` names no runtime — it runs on Tauri's — and
/// pulling tokio in to prove that would undo the claim.
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

/// How the fake's call ends, which is the axis every interesting case sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// The whole document arrives and the provider says it is done.
    Whole,
    /// The provider stops early — a `max_tokens` cap, in practice.
    CutShort,
    /// The connection dies partway through.
    Dropped,
    /// Nothing arrives at all and the provider never says why. The case the
    /// [`Cancel`] wakeup exists for: a poll-only token leaves this call parked
    /// until the provider feels like talking, and the user is billed for the
    /// wait.
    Quiet,
}

struct FakeProvider {
    ending: Ending,
    /// What the provider reports having charged for the prompt before it
    /// generates anything — Anthropic sends this in `message_start`, before any
    /// output exists, which is exactly why usage has to survive a failure.
    input_tokens: u32,
    /// Observed from the test rather than inferred from the deltas, so
    /// "cancellation stopped the work" can be told apart from "cancellation
    /// stopped the deltas".
    chunks_sent: Arc<AtomicUsize>,
}

impl FakeProvider {
    fn new(ending: Ending) -> Self {
        FakeProvider { ending, input_tokens: 812, chunks_sent: Arc::new(AtomicUsize::new(0)) }
    }
}

#[async_trait::async_trait]
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
        let mut usage = Usage { input_tokens: self.input_tokens, ..Usage::default() };

        if self.ending == Ending::Quiet {
            // A real adapter races this against the next read. With nothing to
            // read, the race is just the wait.
            cancel.cancelled().await;
            return EnhanceOutcome::new(usage, Err(Error::Cancelled));
        }

        let document = document_matching(request.schema());
        let chunks = split(&document, 9);
        let deliver = match self.ending {
            Ending::Whole => chunks.len(),
            Ending::CutShort | Ending::Dropped => chunks.len() * 2 / 3,
            Ending::Quiet => unreachable!(),
        };

        let mut streamed = String::new();
        for chunk in chunks.iter().take(deliver) {
            // Checked before each chunk, which is where a real adapter checks
            // it: the point is that the loop stops, so the response body drops
            // and the provider stops generating.
            if cancel.is_cancelled() {
                return EnhanceOutcome::new(usage, Err(Error::Cancelled));
            }
            deltas.delta(chunk);
            streamed.push_str(chunk);
            usage.output_tokens += 1;
            self.chunks_sent.fetch_add(1, Ordering::SeqCst);
        }

        let result = match self.ending {
            // Validated on the way out no matter what the provider claims about
            // schema conformance.
            Ending::Whole => wobu_llm::parse_description(request.kind, &streamed),
            // Reported without being handed to the validator first: a truncated
            // document that happened to parse would otherwise be written down.
            Ending::CutShort => Err(Error::Truncated),
            Ending::Dropped => Err(Error::Unavailable { detail: "connection reset".into() }),
            Ending::Quiet => unreachable!(),
        };
        EnhanceOutcome::new(usage, result)
    }
}

/// A response built from the schema alone — no peeking at the kind registry — so
/// what the fake streams is what a provider honouring the schema would send.
fn document_matching(schema: Value) -> String {
    let mut object = Map::new();
    for (key, property) in schema["properties"].as_object().unwrap() {
        let value = match property["type"].as_str().unwrap() {
            "string" => json!("Ash-glazed ceramic plate over oiled leather."),
            "array" if property["items"]["pattern"].is_string() => json!(["#2b2118", "#c2703a"]),
            "array" => json!(["Ember-lit throat vents"]),
            other => panic!("the schema declares an unhandled property type {other}"),
        };
        object.insert(key.clone(), value);
    }
    serde_json::to_string(&Value::Object(object)).unwrap()
}

/// Character-wise so the split cannot land inside a multi-byte character. A real
/// SSE stream splits wherever the transport felt like it, including mid-word and
/// mid-token, which is the property being modelled.
fn split(text: &str, size: usize) -> Vec<String> {
    text.chars().collect::<Vec<_>>().chunks(size).map(|c| c.iter().collect()).collect()
}

fn request() -> EnhanceRequest {
    EnhanceRequest::new(NodeKind::Character, "fake-1", "Describe Vashk, an ashfall scout.")
        .with_system("You describe how things look.")
}

#[test]
fn a_completed_call_streams_exactly_the_document_it_then_validates() {
    // The boundary in one test: schema out, deltas in order, validated
    // description back. If the deltas and the validated answer could differ, the
    // editor would show one thing and the node would hold another.
    let provider = FakeProvider::new(Ending::Whole);
    let mut streamed = String::new();
    let mut sink = |json: &str| streamed.push_str(json);

    let outcome = block_on(provider.enhance(&request(), &mut sink, &Cancel::new()));

    let validated = outcome.result.expect("a whole response should validate");
    assert_eq!(
        validated.description.sections.keys().count(),
        description_schema(NodeKind::Character)["properties"].as_object().unwrap().len(),
    );
    assert!(validated.extra_sections.is_empty());
    assert_eq!(
        wobu_llm::parse_description(NodeKind::Character, &streamed).unwrap(),
        validated,
    );
    assert_eq!(outcome.usage.input_tokens, 812);
    assert!(outcome.usage.output_tokens > 0);
}

#[test]
fn a_truncated_response_is_a_failure_even_though_most_of_it_arrived() {
    // The regression this exists for: a description cut off mid-sentence being
    // saved as canon because the deltas looked like progress. The only route to
    // a description is `result`, and there is nothing partial in it.
    let provider = FakeProvider::new(Ending::CutShort);
    let mut streamed = String::new();
    let mut sink = |json: &str| streamed.push_str(json);

    let outcome = block_on(provider.enhance(&request(), &mut sink, &Cancel::new()));

    assert!(!streamed.is_empty(), "the test is worthless if nothing streamed");
    assert!(matches!(outcome.result, Err(Error::Truncated)));
    // And a caller that tried to salvage the streamed text gets an error too,
    // rather than a description with three of its nine sections.
    assert!(wobu_llm::parse_description(NodeKind::Character, &streamed).is_err());
}

#[test]
fn a_stream_that_dies_partway_still_reports_what_the_prompt_cost() {
    // The provider charged for the input the moment it read it. A failure that
    // reported zero would let the spend ceiling drift low exactly when a flaky
    // connection is making the user retry.
    let provider = FakeProvider::new(Ending::Dropped);
    let outcome = block_on(provider.enhance(&request(), &mut Discard, &Cancel::new()));

    assert!(matches!(outcome.result, Err(Error::Unavailable { .. })));
    assert_eq!(outcome.usage.input_tokens, 812);
    assert!(outcome.usage.output_tokens > 0, "the tokens it did produce were billed too");
}

#[test]
fn cancelling_mid_stream_stops_the_provider_rather_than_discarding_its_answer() {
    // "Cancelled" has to mean the work stopped. A run that finishes and throws
    // the result away costs the same as one nobody cancelled, which is the
    // difference this asserts: fewer chunks were produced, not just fewer shown.
    let provider = FakeProvider::new(Ending::Whole);
    let sent = Arc::clone(&provider.chunks_sent);
    let cancel = Cancel::new();

    let mut seen = 0usize;
    let mut sink = |_: &str| {
        seen += 1;
        if seen == 2 {
            cancel.cancel();
        }
    };
    let outcome = block_on(provider.enhance(&request(), &mut sink, &cancel));

    assert!(matches!(outcome.result, Err(Error::Cancelled)));
    assert_eq!(sent.load(Ordering::SeqCst), 2, "the loop kept going after Stop");
    let whole = split(&document_matching(description_schema(NodeKind::Character)), 9).len();
    assert!(whole > 2, "the document has to be longer than what was streamed");
}

#[test]
fn cancelling_a_call_that_is_cancelled_is_not_retried_by_anyone() {
    // Both halves of the contract in one place: the queue reads `is_retryable`
    // and the UI reads the code, and a cancellation that either one treated as
    // transient would bill the user for pressing Stop.
    let provider = FakeProvider::new(Ending::Whole);
    let cancel = Cancel::new();
    cancel.cancel();

    let outcome = block_on(provider.enhance(&request(), &mut Discard, &cancel));
    let error = outcome.result.expect_err("a cancelled call has no description");
    assert!(!error.is_retryable());
    assert_eq!(error.code(), "cancelled");
}

#[test]
fn a_call_waiting_on_a_quiet_provider_is_woken_by_the_cancellation() {
    // The expensive case. Between two tokens a provider can be silent for tens
    // of seconds while still generating billable output, so a token that can
    // only be polled leaves the user paying for a request they stopped wanting.
    let provider = FakeProvider::new(Ending::Quiet);
    let cancel = Cancel::new();
    let stop = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        stop.cancel();
    });

    let outcome = block_on(provider.enhance(&request(), &mut Discard, &cancel));

    assert!(matches!(outcome.result, Err(Error::Cancelled)));
    // Still billed for the prompt it had already read.
    assert_eq!(outcome.usage.input_tokens, 812);
}

#[test]
fn a_provider_works_through_a_box_dyn_because_the_project_picks_one_at_runtime() {
    // `project.json` names the provider, so the enhance path holds a
    // `Box<dyn TextProvider>`. A trait that stopped being object safe — a
    // generic parameter on `enhance`, a native `async fn` — would break that at
    // the call site rather than here.
    let providers: Vec<Box<dyn TextProvider>> = vec![Box::new(FakeProvider::new(Ending::Whole))];
    for provider in &providers {
        assert_eq!(provider.id(), "fake");
        assert_eq!(provider.label(), "Fake");
        let request = EnhanceRequest::new(NodeKind::Prop, provider.default_model(), "A censer.");
        let outcome = block_on(provider.enhance(&request, &mut Discard, &Cancel::new()));
        assert!(outcome.is_ok());
    }
}

#[test]
fn every_kind_can_be_asked_for_and_answered_through_the_trait() {
    // The schema is per kind and generated, so a kind added to the registry
    // reaches a provider without anyone wiring it up — and this is where a kind
    // whose schema a provider could honour but the validator would reject shows
    // up.
    for def in wobu_core::kind::kind_registry() {
        let provider = FakeProvider::new(Ending::Whole);
        let request = EnhanceRequest::new(def.kind, "fake-1", "…");
        let outcome = block_on(provider.enhance(&request, &mut Discard, &Cancel::new()));
        let validated = outcome
            .result
            .unwrap_or_else(|e| panic!("{} could not round-trip its own schema: {e}", def.kind));
        assert!(!validated.description.is_empty(), "{}", def.kind);
    }
}
