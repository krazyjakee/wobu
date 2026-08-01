//! What we send Anthropic, what we make of what comes back, and how their
//! failures become ours.
//!
//! Everything here is a pure function of bytes, so the parts of the adapter
//! that decide whether a description is fit to save can be tested from recorded
//! payloads rather than from a paid call. The HTTP itself lives next door in
//! `anthropic.rs` and is the only part of this adapter a test cannot reach.

use std::time::Duration;

use serde_json::{Value, json};
use wobu_core::NodeKind;

use crate::error::Error;
use crate::provider::{DeltaSink, EnhanceOutcome, EnhanceRequest, Usage};
use crate::validate::parse_description;

use super::LABEL;

/// The tool the model is told to call, and forced to.
///
/// Anthropic has no "return JSON matching this schema" mode; it has tool use,
/// so a structured description is a tool call whose *input* is the description.
/// The name is this adapter's invention — Gemini's `response_format` needs no
/// such thing — which is why it is here and not on [`EnhanceRequest`].
pub(crate) const TOOL_NAME: &str = "record_description";

/// Why the model is being handed a tool it cannot actually run.
///
/// Written as an instruction rather than a description of a function, because
/// with `tool_choice` pinned this text is the only place left to say what a good
/// answer looks like.
const TOOL_DESCRIPTION: &str = "Record the finished visual description. Fill in every field with \
     concrete, drawable detail — what an artist would put on the page — and nothing about plot, \
     history, or personality except where it shows.";

/// The body of one Enhance request.
///
/// Deliberately small. Every parameter Anthropic accepts that is not here is
/// absent on purpose:
///
/// - No `temperature`, `top_p` or `top_k`. Claude 4.7 and later reject all three
///   with a 400, so sending them would break the models we default to in
///   exchange for a knob `docs/08-providers.md` already rules out for Gemini.
/// - No `thinking`. Omitting it is the one setting every current model accepts:
///   on Opus/Sonnet it means thinking is off, and on Fable 5 thinking is always
///   on and an explicit `disabled` is a 400. A description is a description; the
///   tokens are better spent on the answer.
/// - No `strict: true` on the tool. It would make the API enforce the schema for
///   us, but the schema puts a `pattern` on palette entries and structured
///   outputs documents string constraints as unsupported — so turning it on
///   risks trading a rare bad palette entry for every request failing. The
///   client-side validator catches the same thing for free.
pub(crate) fn request_body(request: &EnhanceRequest) -> Value {
    let mut body = json!({
        "model": request.model,
        "max_tokens": request.max_output_tokens,
        "stream": true,
        "messages": [{ "role": "user", "content": request.prompt }],
        "tools": [{
            "name": TOOL_NAME,
            "description": TOOL_DESCRIPTION,
            "input_schema": request.schema(),
        }],
        // `any` would also force a call, but naming the tool is what makes a
        // second tool added here later a compile-time decision rather than a
        // model's. `disable_parallel_tool_use` is what stops two half-filled
        // descriptions arriving on separate indices.
        "tool_choice": {
            "type": "tool",
            "name": TOOL_NAME,
            "disable_parallel_tool_use": true,
        },
    });
    if let Some(system) = &request.system {
        // Top level, not folded into the user turn: that is the field Anthropic
        // documents for standing instructions and it measurably outweighs the
        // same text in the prompt.
        body["system"] = Value::String(system.clone());
    }
    body
}

/// Whether the adapter should keep reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Continue,
    /// The API said the message is over, one way or the other. Returning here
    /// is what drops the response body and stops the meter.
    Done,
}

/// Everything the event stream has said so far.
///
/// Kept as one struct rather than as locals in the read loop so that the
/// decision at the end — description, truncation, or failure — is made in one
/// place from the whole picture, and so a test can build that picture from
/// recorded payloads without a socket.
#[derive(Debug, Default)]
pub(crate) struct Incoming {
    usage: Usage,
    /// Which content block is the tool call. The model may also emit prose, and
    /// prose is not part of the document — forwarding it would put commentary
    /// into the editor and into the text the validator is handed.
    tool_index: Option<u64>,
    json: String,
    tool_complete: bool,
    stop_reason: Option<String>,
    complete: bool,
    error: Option<Error>,
}

impl Incoming {
    pub(crate) fn new() -> Incoming {
        Incoming::default()
    }

    /// Take one `data:` payload.
    pub(crate) fn accept(&mut self, data: &str, deltas: &mut dyn DeltaSink) -> Flow {
        // A payload that is not JSON is not something to fail the call over:
        // the versioning policy says new event types will appear, and the ones
        // that decide the outcome are all still to come. Failing here would
        // turn a forward-compatible addition into a broken Enhance.
        let Ok(event) = serde_json::from_str::<Value>(data) else {
            return Flow::Continue;
        };

        match event["type"].as_str().unwrap_or_default() {
            // Sent before a single token exists, which is exactly why usage has
            // to survive a failure: by this point the prompt is billed.
            "message_start" => self.read_usage(&event["message"]["usage"]),
            "content_block_start" => {
                let block = &event["content_block"];
                if block["type"] == "tool_use" && block["name"] == TOOL_NAME {
                    self.tool_index = event["index"].as_u64();
                }
            }
            "content_block_delta" if self.is_tool_block(&event) => {
                // `input_json_delta` carries a fragment of the tool's input
                // document — which is the description, as JSON text. That is
                // precisely what `DeltaSink` is documented to carry.
                if let Some(fragment) = event["delta"]["partial_json"].as_str() {
                    self.json.push_str(fragment);
                    deltas.delta(fragment);
                }
            }
            "content_block_stop" if self.is_tool_block(&event) => self.tool_complete = true,
            "message_delta" => {
                if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_string());
                }
                self.read_usage(&event["usage"]);
            }
            "message_stop" => {
                self.complete = true;
                return Flow::Done;
            }
            // A 200 that fails partway through. It arrives as an event rather
            // than a status code, so an adapter that only maps statuses reports
            // a truncated response for what is really an overload.
            "error" => {
                let error = &event["error"];
                self.error = Some(error_for_type(
                    error["type"].as_str().unwrap_or_default(),
                    error["message"].as_str().unwrap_or_default(),
                ));
                return Flow::Done;
            }
            _ => {}
        }
        Flow::Continue
    }

    /// What the call amounts to, given how the stream ended.
    ///
    /// `aborted` is the caller's own reason for stopping — a cancellation, a
    /// dead socket — and outranks anything inferred here, because those are the
    /// two cases where the stream stopped for a reason the events cannot state.
    pub(crate) fn outcome(self, kind: NodeKind, aborted: Option<Error>) -> EnhanceOutcome {
        let usage = self.usage;
        let result = if let Some(error) = aborted.or(self.error) {
            Err(error)
        } else if matches!(
            self.stop_reason.as_deref(),
            Some("max_tokens" | "model_context_window_exceeded")
        ) {
            // The document is cut off wherever the cap fell. Reported before
            // the validator sees it: a truncated object that happens to parse
            // would otherwise be written down as canon.
            Err(Error::Truncated)
        } else if !self.complete {
            // No `message_stop`, so the connection died mid-message.
            Err(Error::Truncated)
        } else if self.tool_complete {
            // Validated regardless of what tool use promises about schema
            // conformance — see `validate.rs`.
            parse_description(kind, &self.json)
        } else if self.stop_reason.as_deref() == Some("refusal") {
            // A safety classifier declined. It has no variant of its own
            // because the UI's taxonomy has no code for it, and the honest
            // summary is the same either way: what came back is not the
            // document. Retrying is likely to fail identically, which is the
            // one cost of folding it in here.
            Err(Error::NotJson(format!(
                "{LABEL} declined the request instead of calling `{TOOL_NAME}`"
            )))
        } else {
            Err(Error::NotJson(format!("the response contained no `{TOOL_NAME}` call")))
        };
        EnhanceOutcome::new(usage, result)
    }

    fn is_tool_block(&self, event: &Value) -> bool {
        self.tool_index.is_some() && event["index"].as_u64() == self.tool_index
    }

    /// Read whichever token counts this event carries.
    ///
    /// Fields are assigned only when present, because `message_delta` repeats
    /// the running totals and may name only the ones that changed. The counts
    /// are cumulative, so the last sighting is the answer.
    fn read_usage(&mut self, usage: &Value) {
        let count = |key: &str| usage[key].as_u64().map(|n| n.min(u64::from(u32::MAX)) as u32);

        // Cache *writes* are folded in with fresh input rather than with cache
        // reads. Anthropic bills a write above the base input rate and a read at
        // a tenth of it, so counting a write as cached input would understate a
        // long prompt's first call by an order of magnitude — and #55's spend
        // ceiling is only a ceiling if it errs the other way.
        let fresh = count("input_tokens");
        let written = count("cache_creation_input_tokens");
        if fresh.is_some() || written.is_some() {
            self.usage.input_tokens = fresh.unwrap_or(0).saturating_add(written.unwrap_or(0));
        }
        if let Some(read) = count("cache_read_input_tokens") {
            self.usage.cached_input_tokens = read;
        }
        if let Some(output) = count("output_tokens") {
            self.usage.output_tokens = output;
        }
    }
}

/// A non-2xx response, turned into something the UI has an answer for.
///
/// The statuses are Anthropic's documented set (`docs.claude.com/en/api/errors`,
/// read 2026-07-31): 400 `invalid_request_error`, 401 `authentication_error`,
/// 402 `billing_error`, 403 `permission_error`, 404 `not_found_error`, 409
/// `conflict_error`, 413 `request_too_large`, 429 `rate_limit_error`, 500
/// `api_error`, 504 `timeout_error`, 529 `overloaded_error`.
///
/// The status decides, not the body, with two exceptions where the status is
/// too coarse to act on and the message is the only thing that separates them.
pub(crate) fn error_for_status(status: u16, body: &str, retry_after: Option<Duration>) -> Error {
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let kind = parsed["error"]["type"].as_str().unwrap_or_default();
    let message = parsed["error"]["message"].as_str().unwrap_or(body).trim();

    match status {
        401 => Error::BadKey { provider: LABEL },
        402 => Error::BillingRequired { provider: LABEL },
        // 403 is one status for two different conversations: a key that is not
        // allowed near this model, and an account that has run out of money.
        // Only the message tells them apart, and sending a user to their
        // billing page over a workspace permission is a dead end.
        403 if mentions_billing(message) => Error::BillingRequired { provider: LABEL },
        403 => Error::BadKey { provider: LABEL },
        413 => Error::ContextTooLong,
        429 => Error::RateLimited { provider: LABEL, retry_after },
        // A 400 is either "your prompt does not fit" — which the user can act
        // on by trimming the influence stack — or a request this crate built
        // wrong, which they cannot.
        400 if mentions_context(message) => Error::ContextTooLong,
        // Every other 4xx is a request we should not have sent: a schema the
        // provider will not take, a model id `project.json` names that does not
        // exist. Retrying spends money to fail identically, and `SchemaRejected`
        // is this crate's word for a refusal that is ours to fix — which is why
        // it lands on `internal` rather than on a provider code.
        400..=499 => Error::SchemaRejected { detail: detail(status, kind, message) },
        // 500, 504 and 529, plus anything unrecognised. Waiting is the right
        // answer to all of them.
        _ => Error::Unavailable { detail: detail(status, kind, message) },
    }
}

/// A failure delivered as an SSE `error` event, which arrives after a 200 and
/// so has no status to read. Anthropic documents these as the same `type`
/// strings the bodies carry.
pub(crate) fn error_for_type(kind: &str, message: &str) -> Error {
    match kind {
        "authentication_error" => Error::BadKey { provider: LABEL },
        "billing_error" => Error::BillingRequired { provider: LABEL },
        "permission_error" if mentions_billing(message) => {
            Error::BillingRequired { provider: LABEL }
        }
        "permission_error" => Error::BadKey { provider: LABEL },
        // No `retry-after` to read: the header was on a response that already
        // succeeded. The job queue backs off on its own without one.
        "rate_limit_error" => Error::RateLimited { provider: LABEL, retry_after: None },
        "request_too_large" => Error::ContextTooLong,
        "invalid_request_error" if mentions_context(message) => Error::ContextTooLong,
        "invalid_request_error" => Error::SchemaRejected { detail: format!("{kind}: {message}") },
        // `overloaded_error`, `api_error`, `timeout_error`, and whatever gets
        // added next. All of them mean try again later.
        _ => Error::Unavailable {
            detail: if kind.is_empty() {
                message.to_string()
            } else {
                format!("{kind}: {message}")
            },
        },
    }
}

/// The phrases Anthropic uses when the prompt itself is the problem. There is
/// no machine-readable subtype for this, so it is a string match — and it is
/// written to under-match: guessing wrong here costs a retry that fails, where
/// the fallback costs a clear "this is our bug" in the log.
fn mentions_context(message: &str) -> bool {
    let message = message.to_lowercase();
    message.contains("prompt is too long")
        || message.contains("context limit")
        || message.contains("context window")
        || message.contains("too many total text bytes")
}

fn mentions_billing(message: &str) -> bool {
    let message = message.to_lowercase();
    message.contains("credit")
        || message.contains("billing")
        || message.contains("payment")
        || message.contains("plans & billing")
}

/// Never includes anything from the request, because this string reaches a log.
fn detail(status: u16, kind: &str, message: &str) -> String {
    match (kind.is_empty(), message.is_empty()) {
        (true, true) => format!("HTTP {status}"),
        (true, false) => format!("HTTP {status}: {message}"),
        (false, _) => format!("HTTP {status} {kind}: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Discard;
    use wobu_core::schema::description_schema;

    fn request() -> EnhanceRequest {
        EnhanceRequest::new(NodeKind::Character, "claude-sonnet-5", "Describe Vashk.")
            .with_system("You describe how things look.")
    }

    #[test]
    fn the_request_asks_for_the_kinds_own_schema_as_a_tool_the_model_must_call() {
        // The whole reason this adapter uses tool use: a prompt asking politely
        // for JSON gets prose back often enough to matter, and prose cannot be
        // validated into a node.
        let body = request_body(&request());
        assert_eq!(body["tools"][0]["name"], TOOL_NAME);
        // Against the *request's* schema rather than the registry's, because
        // that is the one the validator applies on the way back and it is one
        // property wider — `questions`, which is per call and never a section.
        // Comparing against the registry here would let an adapter that quietly
        // dropped that property still pass.
        assert_eq!(body["tools"][0]["input_schema"], request().schema());
        assert_eq!(
            body["tools"][0]["input_schema"]["properties"]["palette"],
            description_schema(NodeKind::Character)["properties"]["palette"],
        );
        assert_eq!(body["tool_choice"]["type"], "tool");
        assert_eq!(body["tool_choice"]["name"], TOOL_NAME);
        assert_eq!(body["tool_choice"]["disable_parallel_tool_use"], true);
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], crate::provider::DEFAULT_MAX_OUTPUT_TOKENS);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "Describe Vashk.");
        assert_eq!(body["system"], "You describe how things look.");
    }

    #[test]
    fn the_request_carries_no_sampling_or_thinking_parameters() {
        // Claude 4.7 and later reject `temperature`, `top_p` and `top_k` with a
        // 400, and Fable 5 rejects an explicit `thinking: disabled`. Omitting
        // all four is the only shape every current model accepts, so a knob
        // added here later breaks Enhance on whichever models the user picked.
        let body = request_body(&request());
        for absent in ["temperature", "top_p", "top_k", "thinking", "output_config"] {
            assert!(body.get(absent).is_none(), "{absent} should not be sent");
        }
        assert!(body["tools"][0].get("strict").is_none());
    }

    #[test]
    fn a_request_without_standing_instructions_sends_no_system_field() {
        // `"system": null` is not the same as no system field, and only one of
        // them is a request Anthropic accepts.
        let body = request_body(&EnhanceRequest::new(NodeKind::Prop, "m", "A censer."));
        assert!(body.get("system").is_none());
    }

    /// The documented stream for a forced tool call, in the shape
    /// `docs.claude.com/en/build-with-claude/streaming` gives it: usage before
    /// any output, prose block first, then the tool input as partial JSON.
    fn tool_stream(document: &str) -> Vec<String> {
        let mut events = vec![
            json!({"type": "message_start", "message": {"usage": {
                "input_tokens": 812, "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0, "output_tokens": 2}}})
            .to_string(),
            json!({"type": "content_block_start", "index": 0,
                   "content_block": {"type": "text", "text": ""}})
            .to_string(),
            json!({"type": "content_block_delta", "index": 0,
                   "delta": {"type": "text_delta", "text": "Okay, recording that:"}})
            .to_string(),
            json!({"type": "content_block_stop", "index": 0}).to_string(),
            json!({"type": "content_block_start", "index": 1, "content_block":
                   {"type": "tool_use", "id": "toolu_01", "name": TOOL_NAME, "input": {}}})
            .to_string(),
        ];
        for fragment in document.as_bytes().chunks(11) {
            let fragment = std::str::from_utf8(fragment).unwrap();
            events.push(
                json!({"type": "content_block_delta", "index": 1,
                       "delta": {"type": "input_json_delta", "partial_json": fragment}})
                .to_string(),
            );
        }
        events.push(json!({"type": "content_block_stop", "index": 1}).to_string());
        events.push(
            json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"},
                   "usage": {"output_tokens": 289}})
            .to_string(),
        );
        events.push(json!({"type": "message_stop"}).to_string());
        events
    }

    /// A description built from the schema rather than hand-written, so a
    /// registry change that the validator would reject fails here.
    fn document(kind: NodeKind) -> String {
        let schema = description_schema(kind);
        let mut object = serde_json::Map::new();
        for (key, property) in schema["properties"].as_object().unwrap() {
            let value = match property["type"].as_str().unwrap() {
                "string" => json!("Ash-glazed plate over oiled leather."),
                "array" if property["items"]["pattern"].is_string() => {
                    json!(["#2b2118", "#c2703a"])
                }
                _ => json!(["Ember-lit throat vents"]),
            };
            object.insert(key.clone(), value);
        }
        serde_json::to_string(&Value::Object(object)).unwrap()
    }

    fn feed(events: &[String], deltas: &mut dyn DeltaSink) -> Incoming {
        let mut incoming = Incoming::new();
        for event in events {
            if incoming.accept(event, deltas) == Flow::Done {
                break;
            }
        }
        incoming
    }

    #[test]
    fn a_complete_tool_call_becomes_a_validated_description() {
        let events = tool_stream(&document(NodeKind::Character));
        let outcome = feed(&events, &mut Discard).outcome(NodeKind::Character, None);
        let validated = outcome.result.expect("a whole tool call should validate");
        assert!(validated.extra_sections.is_empty());
        assert_eq!(outcome.usage.input_tokens, 812);
        assert_eq!(outcome.usage.output_tokens, 289);
    }

    #[test]
    fn only_the_tool_blocks_json_reaches_the_sink() {
        // The regression: forwarding the model's prose preamble as well. The
        // sink is documented to carry the response document, and the editor
        // parses it incrementally — a sentence of commentary in front of the
        // opening brace makes every fragment unparseable.
        let expected = document(NodeKind::Character);
        let mut streamed = String::new();
        let mut sink = |json: &str| streamed.push_str(json);
        feed(&tool_stream(&expected), &mut sink);
        assert_eq!(streamed, expected);
    }

    #[test]
    fn a_stream_cut_off_at_max_tokens_is_a_failure_rather_than_a_short_description() {
        // A cap reached mid-object leaves JSON that may still parse if the cut
        // lands luckily. Believing it would save half a description as canon.
        let mut events = tool_stream(&document(NodeKind::Character));
        let stop = events.len() - 2;
        events[stop] = json!({"type": "message_delta", "delta": {"stop_reason": "max_tokens"},
                              "usage": {"output_tokens": 4096}})
        .to_string();
        let outcome = feed(&events, &mut Discard).outcome(NodeKind::Character, None);
        assert!(matches!(outcome.result, Err(Error::Truncated)));
        assert_eq!(outcome.usage.output_tokens, 4096, "the cap was reached, so it was billed");
    }

    #[test]
    fn a_stream_that_stops_before_message_stop_is_truncated_not_accepted() {
        // Everything arrived except the API's word that the message is over.
        // Without that there is no way to know whether more was coming.
        let mut events = tool_stream(&document(NodeKind::Character));
        events.pop();
        let outcome = feed(&events, &mut Discard).outcome(NodeKind::Character, None);
        assert!(matches!(outcome.result, Err(Error::Truncated)));
        assert_eq!(outcome.usage.input_tokens, 812);
    }

    #[test]
    fn a_refusal_or_a_prose_answer_is_reported_rather_than_parsed() {
        // With `tool_choice` pinned this should not happen, which is exactly
        // why it needs a defined answer: the alternative is an empty string
        // handed to the validator and a confusing "not JSON" about nothing.
        for (reason, expected) in [("refusal", "declined"), ("end_turn", "no `record_description`")]
        {
            let events = [
                json!({"type": "message_start", "message": {"usage": {"input_tokens": 5}}})
                    .to_string(),
                json!({"type": "message_delta", "delta": {"stop_reason": reason},
                       "usage": {"output_tokens": 9}})
                .to_string(),
                json!({"type": "message_stop"}).to_string(),
            ];
            let outcome = feed(&events, &mut Discard).outcome(NodeKind::Character, None);
            let error = outcome.result.expect_err("there is no description here");
            assert!(error.to_string().contains(expected), "{error}");
            assert!(error.is_retryable());
            assert_eq!(outcome.usage.output_tokens, 9);
        }
    }

    #[test]
    fn an_error_event_after_a_200_stops_the_read_and_keeps_the_usage() {
        // Anthropic sends overload failures mid-stream, after the status line
        // has already promised success. An adapter that only maps statuses
        // calls this a truncated response and tells the user their model is
        // babbling.
        let events = [
            json!({"type": "message_start", "message": {"usage": {"input_tokens": 812}}})
                .to_string(),
            json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}})
                .to_string(),
            json!({"type": "message_stop"}).to_string(),
        ];
        let incoming = feed(&events, &mut Discard);
        let outcome = incoming.outcome(NodeKind::Character, None);
        assert!(matches!(outcome.result, Err(Error::Unavailable { .. })));
        assert_eq!(outcome.usage.input_tokens, 812);
    }

    #[test]
    fn cache_writes_count_as_fresh_input_and_cache_reads_do_not() {
        // The two are priced an order of magnitude apart — a write above the
        // base rate, a read at a tenth of it — so putting a write in the cheap
        // column makes the spend ceiling read low on exactly the first call of
        // a long prompt.
        let events = [json!({"type": "message_start", "message": {"usage": {
            "input_tokens": 100, "cache_creation_input_tokens": 900,
            "cache_read_input_tokens": 4000, "output_tokens": 1}}})
        .to_string()];
        let usage =
            feed(&events, &mut Discard).outcome(NodeKind::Character, Some(Error::Cancelled)).usage;
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.cached_input_tokens, 4000);
    }

    #[test]
    fn an_unknown_event_type_is_ignored_rather_than_failing_the_call() {
        // Anthropic's versioning policy says new event types will appear.
        // Treating one as a failure would break Enhance on a change that was
        // announced as safe.
        let mut events = tool_stream(&document(NodeKind::Character));
        events.insert(1, json!({"type": "ping"}).to_string());
        events.insert(2, json!({"type": "something_from_2027", "detail": {}}).to_string());
        events.insert(3, "not json at all".to_string());
        let outcome = feed(&events, &mut Discard).outcome(NodeKind::Character, None);
        assert!(outcome.is_ok());
    }

    #[test]
    fn a_caller_that_gave_up_outranks_whatever_the_stream_was_saying() {
        // Cancellation must reach the queue as a cancellation. Anything else
        // gets retried, and the user is billed for pressing Stop.
        let events = tool_stream(&document(NodeKind::Character));
        let outcome =
            feed(&events[..3], &mut Discard).outcome(NodeKind::Character, Some(Error::Cancelled));
        assert!(matches!(outcome.result, Err(Error::Cancelled)));
        assert_eq!(outcome.usage.input_tokens, 812, "the prompt was billed before Stop");
    }

    #[test]
    fn every_documented_status_lands_on_the_variant_the_ui_can_answer() {
        // The regression this guards is a user staring at "the provider could
        // not be reached" over a key they could paste in Settings.
        let cases: &[(u16, &str, &str)] = &[
            (401, "authentication_error", "invalid x-api-key"),
            (402, "billing_error", "Your credit balance is too low"),
            (403, "permission_error", "Your credit balance is too low"),
            (403, "permission_error", "not allowed to use this workspace"),
            (404, "not_found_error", "model: claude-sonnet-9"),
            (413, "request_too_large", "too large"),
            (429, "rate_limit_error", "rate limit"),
            (400, "invalid_request_error", "prompt is too long: 1200000 tokens > 1000000 maximum"),
            (400, "invalid_request_error", "tools.0.input_schema: unsupported keyword"),
            (500, "api_error", "Internal server error"),
            (529, "overloaded_error", "Overloaded"),
        ];
        let expected: &[&str] = &[
            "provider.bad_key",
            "provider.billing_required",
            "provider.billing_required",
            "provider.bad_key",
            "internal",
            "provider.context_too_long",
            "provider.rate_limited",
            "provider.context_too_long",
            "internal",
            "provider.unavailable",
            "provider.unavailable",
        ];
        for ((status, kind, message), code) in cases.iter().zip(expected) {
            let body = json!({"type": "error", "error": {"type": kind, "message": message}});
            let error = error_for_status(*status, &body.to_string(), None);
            assert_eq!(error.code(), *code, "HTTP {status} {kind}: {message}");
        }
    }

    #[test]
    fn a_rate_limit_carries_the_providers_own_wait_rather_than_a_guess() {
        // The queue weighs this against everything else it has to run. A
        // hardcoded backoff either hammers the key or idles the app.
        let error = error_for_status(429, "{}", Some(Duration::from_secs(37)));
        assert!(matches!(
            error,
            Error::RateLimited { retry_after: Some(wait), .. } if wait == Duration::from_secs(37)
        ));
    }

    #[test]
    fn a_status_with_an_unreadable_body_still_reaches_the_right_variant() {
        // Cloudflare answers a 413 before the request reaches Anthropic, so the
        // body is HTML. Depending on the JSON shape would make that a mystery.
        assert!(matches!(error_for_status(413, "<html>...", None), Error::ContextTooLong));
        assert!(matches!(error_for_status(401, "", None), Error::BadKey { .. }));
        assert!(matches!(error_for_status(503, "bad gateway", None), Error::Unavailable { .. }));
    }

    #[test]
    fn a_mid_stream_error_maps_by_type_because_there_is_no_status_left_to_read() {
        assert!(matches!(
            error_for_type("overloaded_error", "Overloaded"),
            Error::Unavailable { .. }
        ));
        assert!(matches!(error_for_type("api_error", ""), Error::Unavailable { .. }));
        assert!(matches!(error_for_type("authentication_error", ""), Error::BadKey { .. }));
        assert!(matches!(error_for_type("billing_error", ""), Error::BillingRequired { .. }));
        assert!(matches!(
            error_for_type("rate_limit_error", ""),
            Error::RateLimited { retry_after: None, .. }
        ));
        assert!(matches!(
            error_for_type("invalid_request_error", "prompt is too long: 9 > 8"),
            Error::ContextTooLong
        ));
        assert!(matches!(error_for_type("an_error_type_from_2027", ""), Error::Unavailable { .. }));
    }

    #[test]
    fn a_detail_that_reaches_a_log_names_the_status_that_produced_it() {
        // `provider.unavailable` and `internal` both say almost nothing on
        // their own. Whoever reads the log needs to know whether they are
        // looking at a 529 to wait out or a 400 to go and fix.
        let body = json!({"error": {"type": "api_error", "message": "Internal server error"}});
        let error = error_for_status(500, &body.to_string(), None);
        let message = error.to_string();
        assert!(message.contains("500"), "{message}");
        assert!(message.contains("api_error"), "{message}");
        assert!(message.contains("Internal server error"), "{message}");
    }
}
