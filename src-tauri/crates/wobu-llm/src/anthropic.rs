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

pub(crate) mod wire;

use std::fmt;

use async_trait::async_trait;

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::provider::{DeltaSink, EnhanceOutcome, EnhanceRequest, TextProvider};
use crate::stream::read_sse;
use crate::transport;

use wire::Incoming;

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
        let client =
            transport::client().map_err(|e| Error::Unavailable { detail: e.to_string() })?;
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

        let body = match transport::json_body(&wire::request_body(request)) {
            Ok(body) => body,
            // Only reachable if the generated schema is not serialisable, which
            // is our bug in the same way a rejected schema is.
            Err(error) => return EnhanceOutcome::unbilled(error),
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
            .body(body);

        let response = match transport::text_stream(send, cancel, wire::error_for_status).await {
            Ok(response) => response,
            Err(error) => return EnhanceOutcome::unbilled(error),
        };

        read_sse(request.kind, response.bytes_stream(), deltas, cancel, Incoming::new()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;
    use wobu_core::NodeKind;

    use crate::provider::Discard;
    use crate::stream::testing::{
        assert_billed_disconnect, assert_complete, assert_every_kind, assert_mid_stream_cancel,
        assert_pre_cancel, assert_quiet_cancel, assert_truncated, block_on, document,
    };

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

    #[test]
    fn a_whole_response_streams_exactly_the_document_it_then_validates() {
        let expected = document(NodeKind::Character);
        assert_complete(wire_bytes(&expected, "tool_use", true), Incoming::new());
    }

    #[test]
    fn every_kind_round_trips_through_the_adapter() {
        assert_every_kind("Anthropic", Incoming::new, |document| {
            wire_bytes(document, "tool_use", true)
        });
    }

    #[test]
    fn a_connection_that_dies_mid_response_reports_what_the_prompt_cost() {
        let bytes = wire_bytes(&document(NodeKind::Character), "tool_use", true);
        assert_billed_disconnect(&bytes, Incoming::new());
    }

    #[test]
    fn a_body_that_simply_stops_is_truncated_rather_than_parsed() {
        let bytes = wire_bytes(&document(NodeKind::Character), "tool_use", false);
        assert_truncated(&bytes, Incoming::new());
    }

    #[test]
    fn cancelling_mid_stream_stops_reading_rather_than_discarding_the_answer() {
        let bytes = wire_bytes(&document(NodeKind::Character), "tool_use", true);
        assert_mid_stream_cancel(&bytes, Incoming::new());
    }

    #[test]
    fn a_call_waiting_on_a_quiet_provider_is_woken_by_the_cancellation() {
        let bytes = wire_bytes(&document(NodeKind::Character), "tool_use", true);
        let billed = bytes.windows(2).position(|pair| pair == b"\n\n").unwrap() + 2;
        assert_quiet_cancel(&bytes[..billed], Incoming::new());
    }

    #[test]
    fn a_cancelled_call_is_never_retried_by_the_queue_or_the_button() {
        let bytes = wire_bytes(&document(NodeKind::Character), "tool_use", true);
        assert_pre_cancel(&bytes, Incoming::new());
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
