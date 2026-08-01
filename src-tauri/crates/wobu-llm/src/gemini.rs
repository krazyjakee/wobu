//! [`TextProvider`] over Google's Gemini Interactions API.
//!
//! The second implementation of the trait, and written deliberately as a mirror
//! of `anthropic.rs`: same read loop, same cancellation, same shape of
//! `wire.rs`, so that the two can be read side by side and every difference
//! between them is a difference between the vendors rather than between two
//! authors. What is genuinely different, and why:
//!
//! 1. **Structured output is a first-class request field.** Where Anthropic has
//!    to be given a tool and forced to call it, Gemini takes a top-level
//!    `response_format` carrying a mime type and the schema, and the answer
//!    arrives as ordinary text that happens to be JSON. Same schema, both ways —
//!    `wobu_core::description_schema(kind)`, unedited.
//! 2. **The answer is one step among several.** An interaction is a numbered
//!    sequence of steps, and a thinking model emits its reasoning as
//!    `step.delta` events on the same event type as the answer. Only the
//!    `model_output` step's text is the document; see `wire.rs`.
//! 3. **Usage rides on any event, not on a fixed one.** Anthropic bills the
//!    prompt and says so in `message_start`; Google attaches a running
//!    `metadata.total_usage` to whatever it likes and restates the totals in
//!    `interaction.completed`. Both are read, because a call that is cancelled
//!    or dropped never reaches the completion and has still been paid for.
//!
//! Cancellation works exactly as it does for Anthropic: every read is raced
//! against [`Cancel::cancelled`], and losing that race returns immediately,
//! which drops the response body and closes the connection.
//!
//! What this file cannot be tested for is the HTTP itself: sending, headers, and
//! the drop that closes the socket. Everything downstream of "here are some
//! bytes" is driven from recorded payloads below and in `wire.rs`. Nothing in
//! this adapter has ever run against the real API — there is no key on this
//! machine — so those payloads are the documented ones, transcribed on
//! **2026-07-31**, and the first live call is where the remaining assumptions
//! get tested.

pub(crate) mod wire;

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use futures_core::Stream;
use wobu_core::NodeKind;

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::provider::{DeltaSink, EnhanceOutcome, EnhanceRequest, TextProvider};
use crate::stream::{Read, Sse, next_chunk, until_cancelled};

use wire::{Flow, Incoming};

/// The `provider` in `project.json` and the `wobu/<provider>` keychain entry.
pub const ID: &str = "gemini";

/// Also the name in error messages, which is why it is capitalised.
pub const LABEL: &str = "Gemini";

/// Model ids and prices verified against
/// <https://ai.google.dev/gemini-api/docs/models/gemini-3.6-flash> and
/// <https://ai.google.dev/gemini-api/docs/pricing> on **2026-07-31**. Both move,
/// so re-read them rather than trusting this comment, and never infer an id from
/// a pattern:
///
/// | id | in / out per Mtok | cached in | context | max output |
/// | --- | --- | --- | --- | --- |
/// | `gemini-3.6-flash` | $1.50 / $7.50 | $0.15 | 1,048,576 | 65,536 |
/// | `gemini-3.5-flash` | $1.50 / $9.00 | $0.15 | 1M | 64k |
/// | `gemini-3.5-flash-lite` | $0.30 / $2.50 | $0.03 | 1M | 64k |
/// | `gemini-3.1-pro-preview` | $2 / $12 to 200k, $4 / $18 beyond | $0.20 | — | — |
///
/// `gemini-3.6-flash` and `gemini-3.5-flash` both have a free tier — "Free of
/// charge" for input, output and caching on the Standard track — which is the
/// reason a Flash model is the default rather than Pro: someone who pastes a key
/// from AI Studio can enhance a character without a card on file.
///
/// 3.6 over 3.5 is a price decision, not a quality one. Google's own pages do
/// not rank them the way the version numbers suggest, and nobody has run
/// `docs/08-providers.md`'s benchmark on our actual Enhance prompt — so 3.6 is
/// picked for being the same input price and cheaper output, and that is all
/// this default claims. Nothing stops a project naming any of them:
/// `project.json` carries the id and this crate never checks it against a list,
/// which is the only way an id released next month works without a release of
/// ours.
pub const DEFAULT_MODEL: &str = "gemini-3.6-flash";

/// The Interactions API, not the legacy `:generateContent`. The usual reason to
/// prefer legacy is a CORS bug that does not apply to a native HTTP client.
///
/// **`/v1beta`, not `/v1beta2`** — `docs/08-providers.md` flagged the two as
/// contradictory, and they still are. On 2026-07-31 the structured output guide,
/// the breaking-changes migration guide (six curl examples), the streaming guide
/// and the API reference all wrote `/v1beta/interactions`; only
/// <https://ai.google.dev/gemini-api/docs/migrate-to-interactions> wrote
/// `/v1beta2`. That is four documents to one, so `/v1beta` is what this sends —
/// but it has *not* been confirmed by a live call, because there is no key on
/// this machine. If Enhance comes back as a 404 with nothing billed, this
/// constant is the first thing to try changing.
const INTERACTIONS_URL: &str = "https://generativelanguage.googleapis.com/v1beta/interactions";

/// The request and response schema this adapter was written against.
///
/// Google versions the Interactions API by date and lets a client pin one: the
/// shape here landed as `2026-05-20`, became the default on 2026-05-26, and the
/// revision before it was removed on 2026-06-08. Sending it explicitly means the
/// next revision — which will rename fields again, as this one turned `outputs`
/// into `steps` and folded `response_mime_type` into `response_format` — is a
/// deliberate act with a re-read of the response shapes attached, rather than an
/// Enhance that stops working on a Tuesday.
///
/// <https://ai.google.dev/gemini-api/docs/interactions-breaking-changes-may-2026>
const API_REVISION: &str = "2026-05-20";

/// Long enough for a slow network, short enough that a black hole is not
/// mistaken for a thinking model. Deliberately *only* a connect timeout: a
/// whole-request timeout would kill a long generation the user is paying for and
/// has not asked to stop.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// A Gemini key and the client that uses it.
pub struct GeminiProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl fmt::Debug for GeminiProvider {
    /// Hand-written because the derived one prints the key, and a
    /// `{provider:?}` in a log line somewhere later is not a thing anyone would
    /// think to review.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeminiProvider")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl GeminiProvider {
    /// The key comes from the keychain (`docs/08-providers.md`); this crate
    /// neither resolves nor stores it.
    pub fn new(api_key: impl Into<String>) -> Result<GeminiProvider> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            // Building a client fails when the TLS backend will not start,
            // which is a machine problem and reads as one.
            .map_err(|e| Error::Unavailable { detail: e.to_string() })?;
        Ok(GeminiProvider { api_key: api_key.into(), base_url: INTERACTIONS_URL.into(), client })
    }

    /// Point at something other than the real API — a local server standing in
    /// for it, or a gateway an organisation puts in front. Kept because the send
    /// path is otherwise the one part of this adapter nothing can reach.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> GeminiProvider {
        self.base_url = base_url.into();
        self
    }
}

#[async_trait]
impl TextProvider for GeminiProvider {
    fn id(&self) -> &'static str {
        ID
    }

    fn label(&self) -> &'static str {
        LABEL
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    async fn enhance(
        &self,
        request: &EnhanceRequest,
        deltas: &mut dyn DeltaSink,
        cancel: &Cancel,
    ) -> EnhanceOutcome {
        // A job cancelled while it was queued should not open a connection at
        // all. Everything before the first byte of response is unbilled.
        if cancel.is_cancelled() {
            return EnhanceOutcome::unbilled(Error::Cancelled);
        }

        let body = match serde_json::to_vec(&wire::request_body(request)) {
            Ok(body) => body,
            // Only reachable if the generated schema is not serialisable, which
            // is our bug in the same way a rejected schema is.
            Err(e) => {
                return EnhanceOutcome::unbilled(Error::SchemaRejected { detail: e.to_string() });
            }
        };

        let send = self
            .client
            .post(&self.base_url)
            // A header rather than the `?key=` query parameter the quickstarts
            // use. Both authenticate; only one of them keeps the key out of
            // proxy logs and out of anything that prints a URL.
            .header("x-goog-api-key", &self.api_key)
            .header("api-revision", API_REVISION)
            .header("content-type", "application/json")
            // Streaming is asked for in the body; this only stops a proxy
            // deciding to buffer the response into one lump.
            .header("accept", "text/event-stream")
            .body(body)
            .send();

        let response = match until_cancelled(send, cancel).await {
            None => return EnhanceOutcome::unbilled(Error::Cancelled),
            Some(Err(e)) => {
                // DNS, TLS, refused connection, connect timeout. Nothing reached
                // a model, so nothing was charged.
                return EnhanceOutcome::unbilled(Error::Unavailable { detail: e.to_string() });
            }
            Some(Ok(response)) => response,
        };

        let status = response.status();
        if !status.is_success() {
            // Read before the body is consumed, and only the header. Google's
            // own answer to "how long should I wait" is usually in the body
            // instead, as a `RetryInfo` detail — `wire::error_for_status` reads
            // that when this is absent — and either beats a backoff we would
            // invent (#49 weighs it against the rest of the queue).
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            let body = match until_cancelled(response.text(), cancel).await {
                None => return EnhanceOutcome::unbilled(Error::Cancelled),
                // A body we could not read still leaves a status worth mapping.
                Some(body) => body.unwrap_or_default(),
            };
            // Unbilled: the request was refused before it generated anything.
            return EnhanceOutcome::unbilled(wire::error_for_status(
                status.as_u16(),
                &body,
                retry_after,
            ));
        }

        read_body(request.kind, response.bytes_stream(), deltas, cancel).await
    }
}

/// Read the SSE body to its end, to a failure, or to a cancellation.
///
/// Generic over the byte stream rather than taking a [`reqwest::Response`] so
/// that the loop below — which is where cancellation either works or silently
/// does not — can be driven from recorded chunks without a socket or a runtime.
async fn read_body<S, B, E>(
    kind: NodeKind,
    body: S,
    deltas: &mut dyn DeltaSink,
    cancel: &Cancel,
) -> EnhanceOutcome
where
    S: Stream<Item = std::result::Result<B, E>>,
    B: AsRef<[u8]>,
    E: fmt::Display,
{
    let mut body = std::pin::pin!(body);
    let mut sse = Sse::new();
    let mut incoming = Incoming::new();

    loop {
        // Every return below drops `body`, which drops the response, which
        // closes the connection and stops the model generating. That is the
        // whole mechanism: there is no "cancel" call to make, only a body to
        // stop holding.
        match next_chunk(body.as_mut(), cancel).await {
            Read::Chunk(Ok(bytes)) => {
                sse.push(bytes.as_ref());
                while let Some(event) = sse.next_event() {
                    if incoming.accept(&event, deltas) == Flow::Done {
                        return incoming.outcome(kind, None);
                    }
                }
            }
            Read::Chunk(Err(e)) => {
                let detail = e.to_string();
                return incoming.outcome(kind, Some(Error::Unavailable { detail }));
            }
            // The body ended without an `interaction.completed`; `outcome` calls
            // that a truncation rather than a description.
            Read::End => return incoming.outcome(kind, None),
            Read::Cancelled => return incoming.outcome(kind, Some(Error::Cancelled)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll, Wake, Waker};

    use serde_json::json;
    use wobu_core::schema::description_schema;

    use crate::provider::Discard;

    /// A one-thread executor, so the adapter's async surface is exercised
    /// without a runtime. `wobu-llm` names none — it runs on Tauri's — and
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

    /// Stands in for `Response::bytes_stream`. Counts its own polls, because
    /// "the request stopped" and "the deltas stopped" are the difference between
    /// cancellation working and cancellation being a lie.
    struct Body {
        chunks: VecDeque<std::result::Result<Vec<u8>, String>>,
        polls: Arc<AtomicUsize>,
        /// What happens once the chunks run out. `true` models a provider that
        /// has gone quiet mid-response — the case a poll-only cancellation
        /// leaves the user paying for.
        quiet: bool,
    }

    impl Body {
        fn new(chunks: Vec<std::result::Result<Vec<u8>, String>>) -> Body {
            Body { chunks: chunks.into(), polls: Arc::new(AtomicUsize::new(0)), quiet: false }
        }

        fn quiet_after(mut self) -> Body {
            self.quiet = true;
            self
        }
    }

    impl Stream for Body {
        type Item = std::result::Result<Vec<u8>, String>;

        fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let this = self.get_mut();
            this.polls.fetch_add(1, Ordering::SeqCst);
            match this.chunks.pop_front() {
                Some(chunk) => Poll::Ready(Some(chunk)),
                // No waker registered on purpose: only the cancellation can wake
                // this task, which is the situation being modelled.
                None if this.quiet => Poll::Pending,
                None => Poll::Ready(None),
            }
        }
    }

    /// The documented SSE wire format, byte for byte, as
    /// `ai.google.dev/gemini-api/docs/interactions/streaming` prints it — down
    /// to the `event: done` / `data: [DONE]` sentinel, so the framing this
    /// adapter relies on is the framing Google publishes.
    fn wire_bytes(document: &str, status: &str, closed: bool) -> Vec<u8> {
        let mut out = String::new();
        let mut frame = |name: &str, data: serde_json::Value| {
            out.push_str(&format!("event: {name}\ndata: {data}\n\n"));
        };
        frame(
            "interaction.created",
            json!({"event_type": "interaction.created", "interaction": {"id": "v1_abc",
                   "status": "in_progress", "object": "interaction", "model": DEFAULT_MODEL},
                   "metadata": {"total_usage": {"total_input_tokens": 812,
                   "total_output_tokens": 0}}}),
        );
        frame(
            "interaction.status_update",
            json!({"event_type": "interaction.status_update", "interaction_id": "v1_abc",
                   "status": "in_progress"}),
        );
        frame(
            "step.start",
            json!({"event_type": "step.start", "index": 0, "step": {"type": "thought"}}),
        );
        frame(
            "step.delta",
            json!({"event_type": "step.delta", "index": 0,
                   "delta": {"type": "thought_signature", "signature": "Cg0Ktz"}}),
        );
        frame("step.stop", json!({"event_type": "step.stop", "index": 0}));
        frame(
            "step.start",
            json!({"event_type": "step.start", "index": 1, "step": {"type": "model_output"}}),
        );
        for fragment in document.as_bytes().chunks(13) {
            frame(
                "step.delta",
                json!({"event_type": "step.delta", "index": 1, "delta":
                       {"type": "text", "text": std::str::from_utf8(fragment).unwrap()}}),
            );
        }
        frame("step.stop", json!({"event_type": "step.stop", "index": 1}));
        if closed {
            frame(
                "interaction.completed",
                json!({"event_type": "interaction.completed", "interaction": {"id": "v1_abc",
                       "status": status, "usage": {"total_input_tokens": 812,
                       "total_output_tokens": 289, "total_tokens": 1101}}}),
            );
            out.push_str("event: done\ndata: [DONE]\n\n");
        }
        out.into_bytes()
    }

    /// A description built from the schema, so a registry change the validator
    /// would reject fails here rather than on a paid call.
    fn document(kind: NodeKind) -> String {
        let schema = description_schema(kind);
        let mut object = serde_json::Map::new();
        for (key, property) in schema["properties"].as_object().unwrap() {
            let value = match property["type"].as_str().unwrap() {
                "string" => json!("A scout in ash-glazed plate, shoulders canted forward."),
                "array" if property["items"]["pattern"].is_string() => {
                    json!(["#2b2118", "#c2703a"])
                }
                _ => json!(["Ember-lit throat vents"]),
            };
            object.insert(key.clone(), value);
        }
        serde_json::to_string(&serde_json::Value::Object(object)).unwrap()
    }

    /// Split at a size that has nothing to do with the frame boundaries — which
    /// is the only realistic assumption about how a socket hands bytes over.
    fn split(bytes: &[u8], size: usize) -> Vec<std::result::Result<Vec<u8>, String>> {
        bytes.chunks(size).map(|chunk| Ok(chunk.to_vec())).collect()
    }

    #[test]
    fn a_whole_response_streams_exactly_the_document_it_then_validates() {
        // End to end over the real wire format: what the editor was shown and
        // what the node will hold have to be the same text, or the typing effect
        // is showing something that was never saved.
        let expected = document(NodeKind::Character);
        let body = Body::new(split(&wire_bytes(&expected, "completed", true), 37));
        let mut streamed = String::new();
        let mut sink = |json: &str| streamed.push_str(json);

        let outcome = block_on(read_body(NodeKind::Character, body, &mut sink, &Cancel::new()));

        let validated = outcome.result.expect("a whole interaction should validate");
        assert_eq!(streamed, expected);
        assert_eq!(crate::parse_description(NodeKind::Character, &streamed).unwrap(), validated);
        assert_eq!(outcome.usage.input_tokens, 812);
        assert_eq!(outcome.usage.output_tokens, 289);
    }

    #[test]
    fn every_kind_round_trips_through_the_adapter() {
        // The schema is generated per kind, so a kind added to the registry
        // reaches this adapter with nobody wiring it up. This is where one whose
        // schema Google could honour but the validator would reject shows up.
        for def in wobu_core::kind::kind_registry() {
            let body = Body::new(split(&wire_bytes(&document(def.kind), "completed", true), 64));
            let outcome = block_on(read_body(def.kind, body, &mut Discard, &Cancel::new()));
            outcome.result.unwrap_or_else(|e| {
                panic!("{} could not round-trip its own schema: {e}", def.kind)
            });
        }
    }

    #[test]
    fn a_connection_that_dies_mid_response_reports_what_the_prompt_cost() {
        // The provider charged for the input the moment it read it, and said so
        // in the `metadata.total_usage` riding on the very first event.
        // Reporting zero would let #55's ceiling drift low exactly when a flaky
        // connection is making the user retry.
        let bytes = wire_bytes(&document(NodeKind::Character), "completed", true);
        let mut chunks = split(&bytes[..bytes.len() / 2], 37);
        chunks.push(Err("connection reset by peer".to_string()));
        let outcome = block_on(read_body(
            NodeKind::Character,
            Body::new(chunks),
            &mut Discard,
            &Cancel::new(),
        ));

        assert!(matches!(outcome.result, Err(Error::Unavailable { .. })));
        assert_eq!(outcome.usage.input_tokens, 812);
    }

    #[test]
    fn a_body_that_simply_stops_is_truncated_rather_than_parsed() {
        // No error, no `interaction.completed` — just an end. Handing the
        // accumulated text to the validator and hoping it notices is how half a
        // description becomes canon.
        let bytes = wire_bytes(&document(NodeKind::Character), "completed", false);
        let outcome = block_on(read_body(
            NodeKind::Character,
            Body::new(split(&bytes, 37)),
            &mut Discard,
            &Cancel::new(),
        ));
        assert!(matches!(outcome.result, Err(Error::Truncated)));
    }

    #[test]
    fn cancelling_mid_stream_stops_reading_rather_than_discarding_the_answer() {
        // The expensive regression. A run that reads to the end and throws the
        // result away costs exactly what an uncancelled one costs; the thing
        // that stops the meter is nobody holding the body. Counted in polls
        // because that is the observable difference.
        let bytes = wire_bytes(&document(NodeKind::Character), "completed", true);
        let body = Body::new(split(&bytes, 24));
        let polls = Arc::clone(&body.polls);
        let total = body.chunks.len();
        let cancel = Cancel::new();

        let mut seen = 0usize;
        let mut sink = |_: &str| {
            seen += 1;
            if seen == 2 {
                cancel.cancel();
            }
        };
        let outcome = block_on(read_body(NodeKind::Character, body, &mut sink, &cancel));

        assert!(matches!(outcome.result, Err(Error::Cancelled)));
        assert!(
            polls.load(Ordering::SeqCst) < total,
            "the loop read {} of {total} chunks after Stop",
            polls.load(Ordering::SeqCst),
        );
        assert_eq!(outcome.usage.input_tokens, 812, "the prompt was billed before Stop");
    }

    #[test]
    fn a_call_waiting_on_a_quiet_provider_is_woken_by_the_cancellation() {
        // Between two tokens a provider can be silent for tens of seconds while
        // still generating billable output — and a thinking model is silent for
        // longer than most. Polling a flag between chunks would leave this
        // parked until it spoke again, which is the case worth paying a `select`
        // for.
        let bytes = wire_bytes(&document(NodeKind::Character), "completed", true);
        // Everything up to the end of `interaction.created`, so the prompt has
        // been billed and reported before the silence begins.
        let billed = bytes.windows(2).position(|pair| pair == b"\n\n").unwrap() + 2;
        let body = Body::new(split(&bytes[..billed], 64)).quiet_after();
        let cancel = Cancel::new();
        let stop = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            stop.cancel();
        });

        let outcome = block_on(read_body(NodeKind::Character, body, &mut Discard, &cancel));

        assert!(matches!(outcome.result, Err(Error::Cancelled)));
        assert_eq!(outcome.usage.input_tokens, 812);
    }

    #[test]
    fn a_cancelled_call_is_never_retried_by_the_queue_or_the_button() {
        let bytes = wire_bytes(&document(NodeKind::Character), "completed", true);
        let cancel = Cancel::new();
        cancel.cancel();
        let outcome = block_on(read_body(
            NodeKind::Character,
            Body::new(split(&bytes, 24)),
            &mut Discard,
            &cancel,
        ));
        let error = outcome.result.expect_err("a cancelled call has no description");
        assert!(!error.is_retryable());
        assert_eq!(error.code(), "cancelled");
    }

    #[test]
    fn the_provider_works_through_a_box_dyn_because_the_project_picks_one() {
        // `project.json` names the provider, so the enhance path holds a
        // `Box<dyn TextProvider>`. Constructing one must also not need a
        // network, or nothing can be wired up offline.
        let provider = GeminiProvider::new("AIza-not-a-real-key").unwrap();
        let boxed: Box<dyn TextProvider> = Box::new(provider);
        assert_eq!(boxed.id(), ID);
        assert_eq!(boxed.label(), LABEL);
        assert_eq!(boxed.default_model(), DEFAULT_MODEL);
    }

    #[test]
    fn the_two_adapters_do_not_share_an_id_a_label_or_a_default_model() {
        // `wobu/<id>` is the keychain entry and `provider` in `project.json`.
        // Two adapters answering the same id would hand one vendor's key to the
        // other, which fails as a rejected key and reads as a bad paste.
        assert_ne!(ID, crate::anthropic::ID);
        assert_ne!(LABEL, crate::anthropic::LABEL);
        assert_ne!(DEFAULT_MODEL, crate::anthropic::DEFAULT_MODEL);
    }

    #[test]
    fn a_cancelled_job_never_opens_a_connection() {
        // The queue can cancel a job between queueing it and starting it. A
        // request sent and then abandoned is one the user is billed for.
        let provider = GeminiProvider::new("AIza-not-a-real-key")
            .unwrap()
            // Unroutable by definition, so a request that did go out would fail
            // as `Unavailable` rather than as a cancellation.
            .with_base_url("http://127.0.0.1:1/v1beta/interactions");
        let cancel = Cancel::new();
        cancel.cancel();
        let request = EnhanceRequest::new(NodeKind::Character, DEFAULT_MODEL, "Describe Vashk.");

        let outcome = block_on(provider.enhance(&request, &mut Discard, &cancel));

        assert!(matches!(outcome.result, Err(Error::Cancelled)));
        assert_eq!(outcome.usage, crate::Usage::default());
    }

    #[test]
    fn debug_output_does_not_carry_the_key() {
        // `redact::scrub` at the command boundary is the real guard, but a
        // `{provider:?}` in a log line has already leaked by the time it gets
        // there.
        let provider = GeminiProvider::new("AIzaSyD-secret-key-material").unwrap();
        let printed = format!("{provider:?}");
        assert!(!printed.contains("AIza"), "{printed}");
        assert!(printed.contains(INTERACTIONS_URL));
    }

    #[test]
    fn the_endpoint_is_the_interactions_api_at_the_revision_this_was_written_against() {
        // Two failures this pins, both of which look like a broken key from the
        // UI: the legacy `:generateContent` path, whose request shape this
        // adapter does not speak; and `/v1beta2`, which one of Google's own
        // pages prints and four others contradict.
        assert!(INTERACTIONS_URL.ends_with("/v1beta/interactions"), "{INTERACTIONS_URL}");
        assert!(!INTERACTIONS_URL.contains("generateContent"));
        assert_eq!(API_REVISION, "2026-05-20");
    }
}
