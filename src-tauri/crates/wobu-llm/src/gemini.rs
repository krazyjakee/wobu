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

use async_trait::async_trait;

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::provider::{DeltaSink, EnhanceOutcome, EnhanceRequest, TextProvider};
use crate::stream::read_sse;
use crate::transport;

use wire::Incoming;

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
        let client =
            transport::client().map_err(|e| Error::Unavailable { detail: e.to_string() })?;
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

        let body = match transport::json_body(&wire::request_body(request)) {
            Ok(body) => body,
            // Only reachable if the generated schema is not serialisable, which
            // is our bug in the same way a rejected schema is.
            Err(error) => return EnhanceOutcome::unbilled(error),
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

    #[test]
    fn a_whole_response_streams_exactly_the_document_it_then_validates() {
        let expected = document(NodeKind::Character);
        assert_complete(wire_bytes(&expected, "completed", true), Incoming::new());
    }

    #[test]
    fn every_kind_round_trips_through_the_adapter() {
        assert_every_kind("Gemini", Incoming::new, |document| {
            wire_bytes(document, "completed", true)
        });
    }

    #[test]
    fn a_connection_that_dies_mid_response_reports_what_the_prompt_cost() {
        let bytes = wire_bytes(&document(NodeKind::Character), "completed", true);
        assert_billed_disconnect(&bytes, Incoming::new());
    }

    #[test]
    fn a_body_that_simply_stops_is_truncated_rather_than_parsed() {
        let bytes = wire_bytes(&document(NodeKind::Character), "completed", false);
        assert_truncated(&bytes, Incoming::new());
    }

    #[test]
    fn cancelling_mid_stream_stops_reading_rather_than_discarding_the_answer() {
        let bytes = wire_bytes(&document(NodeKind::Character), "completed", true);
        assert_mid_stream_cancel(&bytes, Incoming::new());
    }

    #[test]
    fn a_call_waiting_on_a_quiet_provider_is_woken_by_the_cancellation() {
        let bytes = wire_bytes(&document(NodeKind::Character), "completed", true);
        let billed = bytes.windows(2).position(|pair| pair == b"\n\n").unwrap() + 2;
        assert_quiet_cancel(&bytes[..billed], Incoming::new());
    }

    #[test]
    fn a_cancelled_call_is_never_retried_by_the_queue_or_the_button() {
        let bytes = wire_bytes(&document(NodeKind::Character), "completed", true);
        assert_pre_cancel(&bytes, Incoming::new());
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
