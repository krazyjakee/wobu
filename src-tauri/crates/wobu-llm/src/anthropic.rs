//! [`TextProvider`] over Anthropic's Messages API.
//!
//! Three things this adapter does that a thinner one would not, each of them
//! costing the user money if it is skipped:
//!
//! 1. **Structured output is a tool call, not a request for JSON.** Anthropic
//!    has no `response_format`; it has tool use, and the description schema goes
//!    in as a tool's `input_schema` with `tool_choice` pinned to it. The tool
//!    name is this adapter's invention and stops here — see `wire.rs`.
//! 2. **Cancellation aborts the request.** Every read is raced against
//!    [`Cancel::cancelled`], and losing that race returns immediately, which
//!    drops the response body and closes the connection. A run that finishes
//!    and throws the answer away costs exactly as much as one nobody stopped.
//! 3. **Usage is reported on every path.** Anthropic bills the prompt before it
//!    emits a token and says so in `message_start`, so a 429 mid-stream, a
//!    dropped socket and a cancellation have all been paid for.
//!
//! What this file cannot be tested for is the HTTP itself: sending, headers, and
//! the drop that closes the socket. Everything downstream of "here are some
//! bytes" is driven from recorded payloads below and in `wire.rs`.

pub(crate) mod sse;
pub(crate) mod wire;

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use futures_core::Stream;
use wobu_core::NodeKind;

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::provider::{DeltaSink, EnhanceOutcome, EnhanceRequest, TextProvider};

use sse::Sse;
use wire::{Flow, Incoming};

/// The `provider` in `project.json` and the `wobu/<provider>` keychain entry.
pub const ID: &str = "anthropic";

/// Also the name in error messages, which is why it is capitalised.
pub const LABEL: &str = "Anthropic";

/// Model ids and prices verified against
/// <https://platform.claude.com/docs/en/about-claude/models/overview> and
/// <https://platform.claude.com/docs/en/about-claude/pricing> on **2026-07-31**.
/// Both move — `claude-sonnet-5` and `claude-opus-5` did not exist when this
/// crate's dependencies were pinned — so re-read them rather than trusting this
/// comment, and never infer an id from a pattern:
///
/// | id | in / out per Mtok | context | max output |
/// | --- | --- | --- | --- |
/// | `claude-opus-5` | $5 / $25 | 1M | 128k |
/// | `claude-sonnet-5` | $3 / $15 | 1M | 128k |
/// | `claude-haiku-4-5` | $1 / $5 | 200k | 64k |
/// | `claude-fable-5` | $10 / $50 | 1M | 128k |
///
/// Sonnet 5 is $2 / $10 under introductory pricing until 2026-08-31.
///
/// Sonnet is the default because Enhance is a few hundred output tokens of
/// visual invention: Haiku 4.5 is a fifth of the price but a generation older,
/// and Opus 5 costs five times as much for a paragraph about what a censer
/// looks like. Nothing stops a project naming any of them — `project.json`
/// carries the id and this crate never checks it against a list, which is the
/// only way an id released next month works without a release of ours.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5";

const MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";

/// The only API version this adapter has been written against. Anthropic
/// requires it on every request and uses it to keep old shapes working, so
/// bumping it is a deliberate act with a re-read of the response shapes
/// attached, not a version number to keep current.
const API_VERSION: &str = "2023-06-01";

/// Long enough for a slow network, short enough that a black hole is not
/// mistaken for a thinking model. Deliberately *only* a connect timeout: a
/// whole-request timeout would kill a long generation the user is paying for
/// and has not asked to stop.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// An Anthropic key and the client that uses it.
pub struct AnthropicProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl fmt::Debug for AnthropicProvider {
    /// Hand-written because the derived one prints the key, and a
    /// `{provider:?}` in a log line somewhere later is not a thing anyone would
    /// think to review.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl AnthropicProvider {
    /// The key comes from the keychain (`docs/08-providers.md`); this crate
    /// neither resolves nor stores it.
    pub fn new(api_key: impl Into<String>) -> Result<AnthropicProvider> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            // Building a client fails when the TLS backend will not start,
            // which is a machine problem and reads as one.
            .map_err(|e| Error::Unavailable { detail: e.to_string() })?;
        Ok(AnthropicProvider { api_key: api_key.into(), base_url: MESSAGES_URL.into(), client })
    }

    /// Point at something other than the real API — a local server standing in
    /// for it, or a gateway an organisation puts in front. Kept because the
    /// send path is otherwise the one part of this adapter nothing can reach.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> AnthropicProvider {
        self.base_url = base_url.into();
        self
    }
}

#[async_trait]
impl TextProvider for AnthropicProvider {
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
            Err(e) => return EnhanceOutcome::unbilled(Error::SchemaRejected { detail: e.to_string() }),
        };

        let send = self
            .client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            // Streaming is asked for in the body; this only stops a proxy
            // deciding to buffer the response into one lump.
            .header("accept", "text/event-stream")
            .body(body)
            .send();

        let response = match until_cancelled(send, cancel).await {
            None => return EnhanceOutcome::unbilled(Error::Cancelled),
            Some(Err(e)) => {
                // DNS, TLS, refused connection, connect timeout. Nothing
                // reached a model, so nothing was charged.
                return EnhanceOutcome::unbilled(Error::Unavailable { detail: e.to_string() });
            }
            Some(Ok(response)) => response,
        };

        let status = response.status();
        if !status.is_success() {
            // Read before the body is consumed, and only the header — the
            // provider's own hint about how long to wait beats any backoff we
            // would invent (#49 weighs it against the rest of the queue).
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
            // The body ended without a `message_stop`; `outcome` calls that a
            // truncation rather than a description.
            Read::End => return incoming.outcome(kind, None),
            Read::Cancelled => return incoming.outcome(kind, Some(Error::Cancelled)),
        }
    }
}

enum Read<B, E> {
    Chunk(std::result::Result<B, E>),
    End,
    Cancelled,
}

/// The next chunk, or the cancellation that beat it.
///
/// Cancellation is polled first so that a token set while a chunk was already
/// waiting still wins: the point is to stop, and a stream with a chunk ready is
/// a stream that will have another one ready in a moment.
async fn next_chunk<S, B, E>(mut body: Pin<&mut S>, cancel: &Cancel) -> Read<B, E>
where
    S: Stream<Item = std::result::Result<B, E>>,
{
    let mut cancelled = std::pin::pin!(cancel.cancelled());
    std::future::poll_fn(move |cx: &mut Context<'_>| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Read::Cancelled);
        }
        match body.as_mut().poll_next(cx) {
            Poll::Ready(Some(chunk)) => Poll::Ready(Read::Chunk(chunk)),
            Poll::Ready(None) => Poll::Ready(Read::End),
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

/// `None` when the cancellation won, in which case `future` is dropped — which
/// for an in-flight request is what abandons it.
async fn until_cancelled<F: Future>(future: F, cancel: &Cancel) -> Option<F::Output> {
    let mut future = std::pin::pin!(future);
    let mut cancelled = std::pin::pin!(cancel.cancelled());
    std::future::poll_fn(move |cx: &mut Context<'_>| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        future.as_mut().poll(cx).map(Some)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Wake, Waker};

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
    /// "the request stopped" and "the deltas stopped" are the difference
    /// between cancellation working and cancellation being a lie.
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
                // No waker registered on purpose: only the cancellation can
                // wake this task, which is the situation being modelled.
                None if this.quiet => Poll::Pending,
                None => Poll::Ready(None),
            }
        }
    }

    /// The documented SSE wire format, byte for byte, so the framing this
    /// adapter relies on is the framing Anthropic publishes.
    fn wire_bytes(document: &str, stop_reason: &str, closed: bool) -> Vec<u8> {
        let mut out = String::new();
        let mut frame = |name: &str, data: serde_json::Value| {
            out.push_str(&format!("event: {name}\ndata: {data}\n\n"));
        };
        frame(
            "message_start",
            json!({"type": "message_start", "message": {"id": "msg_01", "type": "message",
                   "role": "assistant", "content": [], "model": DEFAULT_MODEL,
                   "stop_reason": null, "usage": {"input_tokens": 812,
                   "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0,
                   "output_tokens": 2}}}),
        );
        frame("ping", json!({"type": "ping"}));
        frame(
            "content_block_start",
            json!({"type": "content_block_start", "index": 0, "content_block":
                   {"type": "tool_use", "id": "toolu_01", "name": wire::TOOL_NAME, "input": {}}}),
        );
        for fragment in document.as_bytes().chunks(13) {
            frame(
                "content_block_delta",
                json!({"type": "content_block_delta", "index": 0, "delta":
                       {"type": "input_json_delta",
                        "partial_json": std::str::from_utf8(fragment).unwrap()}}),
            );
        }
        frame("content_block_stop", json!({"type": "content_block_stop", "index": 0}));
        frame(
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": stop_reason,
                   "stop_sequence": null}, "usage": {"output_tokens": 289}}),
        );
        if closed {
            frame("message_stop", json!({"type": "message_stop"}));
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
        // what the node will hold have to be the same text, or the typing
        // effect is showing something that was never saved.
        let expected = document(NodeKind::Character);
        let body = Body::new(split(&wire_bytes(&expected, "tool_use", true), 37));
        let mut streamed = String::new();
        let mut sink = |json: &str| streamed.push_str(json);

        let outcome =
            block_on(read_body(NodeKind::Character, body, &mut sink, &Cancel::new()));

        let validated = outcome.result.expect("a whole tool call should validate");
        assert_eq!(streamed, expected);
        assert_eq!(
            crate::parse_description(NodeKind::Character, &streamed).unwrap(),
            validated,
        );
        assert_eq!(outcome.usage.input_tokens, 812);
        assert_eq!(outcome.usage.output_tokens, 289);
    }

    #[test]
    fn every_kind_round_trips_through_the_adapter() {
        // The schema is generated per kind, so a kind added to the registry
        // reaches this adapter with nobody wiring it up. This is where one
        // whose schema Anthropic could honour but the validator would reject
        // shows up.
        for def in wobu_core::kind::kind_registry() {
            let body = Body::new(split(&wire_bytes(&document(def.kind), "tool_use", true), 64));
            let outcome = block_on(read_body(def.kind, body, &mut Discard, &Cancel::new()));
            outcome
                .result
                .unwrap_or_else(|e| panic!("{} could not round-trip its own schema: {e}", def.kind));
        }
    }

    #[test]
    fn a_connection_that_dies_mid_response_reports_what_the_prompt_cost() {
        // The provider charged for the input the moment it read it. Reporting
        // zero would let #55's ceiling drift low exactly when a flaky
        // connection is making the user retry.
        let bytes = wire_bytes(&document(NodeKind::Character), "tool_use", true);
        let mut chunks = split(&bytes[..bytes.len() / 2], 37);
        chunks.push(Err("connection reset by peer".to_string()));
        let outcome =
            block_on(read_body(NodeKind::Character, Body::new(chunks), &mut Discard, &Cancel::new()));

        assert!(matches!(outcome.result, Err(Error::Unavailable { .. })));
        assert_eq!(outcome.usage.input_tokens, 812);
    }

    #[test]
    fn a_body_that_simply_stops_is_truncated_rather_than_parsed() {
        // No error, no `message_stop` — just an end. Handing the accumulated
        // text to the validator and hoping it notices is how half a description
        // becomes canon.
        let bytes = wire_bytes(&document(NodeKind::Character), "tool_use", false);
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
        let bytes = wire_bytes(&document(NodeKind::Character), "tool_use", true);
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
        // still generating billable output. Polling a flag between chunks would
        // leave this parked until it spoke again — which is the case worth
        // paying a `select` for.
        let bytes = wire_bytes(&document(NodeKind::Character), "tool_use", true);
        // Everything up to the end of `message_start`, so the prompt has been
        // billed and reported before the silence begins.
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
        let bytes = wire_bytes(&document(NodeKind::Character), "tool_use", true);
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
        let provider = AnthropicProvider::new("sk-ant-not-a-real-key").unwrap();
        let boxed: Box<dyn TextProvider> = Box::new(provider);
        assert_eq!(boxed.id(), ID);
        assert_eq!(boxed.label(), LABEL);
        assert_eq!(boxed.default_model(), DEFAULT_MODEL);
    }

    #[test]
    fn a_cancelled_job_never_opens_a_connection() {
        // The queue can cancel a job between queueing it and starting it. A
        // request sent and then abandoned is one the user is billed for.
        let provider = AnthropicProvider::new("sk-ant-not-a-real-key")
            .unwrap()
            // Unroutable by definition, so a request that did go out would fail
            // as `Unavailable` rather than as a cancellation.
            .with_base_url("http://127.0.0.1:1/v1/messages");
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
        let provider = AnthropicProvider::new("sk-ant-api03-secret").unwrap();
        let printed = format!("{provider:?}");
        assert!(!printed.contains("sk-ant"), "{printed}");
        assert!(printed.contains(MESSAGES_URL));
    }
}
