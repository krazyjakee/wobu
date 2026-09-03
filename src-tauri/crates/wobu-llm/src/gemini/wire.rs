//! What we send Google, what we make of what comes back, and how their failures
//! become ours.
//!
//! Everything here is a pure function of bytes, so the parts of the adapter that
//! decide whether a description is fit to save can be tested from recorded
//! payloads rather than from a paid call. The HTTP itself lives next door in
//! `gemini.rs` and is the only part of this adapter a test cannot reach.
//!
//! Every shape below was read off Google's own documentation on **2026-07-31**,
//! not remembered. The pages, so the next person can re-read rather than trust:
//!
//! - Request and streaming:
//!   <https://ai.google.dev/gemini-api/docs/interactions/text-generation>,
//!   <https://ai.google.dev/gemini-api/docs/interactions/streaming>
//! - Structured output and the schema subset:
//!   <https://ai.google.dev/gemini-api/docs/structured-output>
//! - Field-by-field reference: <https://ai.google.dev/api/interactions-api>
//! - Error codes: <https://ai.google.dev/gemini-api/docs/api-errors>

use std::time::Duration;

use serde_json::{Value, json};
use wobu_core::NodeKind;

use crate::error::Error;
use crate::provider::{DeltaSink, EnhanceOutcome, EnhanceRequest, Usage};
use crate::stream::SseConsumer;
use crate::validate::parse_description;

use super::LABEL;

/// The step type carrying the answer.
///
/// Gemini streams an interaction as a numbered sequence of *steps*, and only one
/// of them is the response document. The others are real and arrive on the same
/// event type: a `thought` step emits `step.delta` payloads too, carrying
/// thought signatures and — with `thinking_summaries` on — prose. Forwarding
/// those would put the model's reasoning in front of the opening brace of a
/// document the editor parses incrementally, so the index of this step is
/// tracked and everything else is dropped.
pub(crate) const MODEL_OUTPUT_STEP: &str = "model_output";

/// The body of one Enhance request.
///
/// Deliberately small. Every parameter the Interactions API accepts that is not
/// here is absent on purpose:
///
/// - No `temperature`, `top_p` or `top_k`. `docs/08-providers.md` records these
///   as deprecated on 2026-07-21, which is also why Settings has no sampling
///   sliders to feed them from.
/// - No `generation_config.thinking_level`. Omitting it takes the model's own
///   default. Pinning `minimal` would save thought tokens on every call, but it
///   is a quality decision about the Enhance prompt that nobody has measured,
///   and an adapter is the wrong place to make one silently.
/// - No `previous_interaction_id`. Enhance is one question with one answer; the
///   trait has no history for the same reason.
///
/// `store: false` is the one addition Anthropic has no counterpart for. The
/// Interactions API keeps the request and the response server-side by default so
/// a later call can chain onto them, which for a local-first worldbuilding tool
/// means every character's canon sitting in someone's Google project until it is
/// deleted. We never chain, so there is nothing to trade for it.
pub(crate) fn request_body(request: &EnhanceRequest) -> Value {
    let mut body = json!({
        "model": request.model,
        // One turn, as a bare string. `input` also accepts an array of typed
        // parts, which is what the image adapter will need — but a text-only
        // request written that way is a shape with no reader.
        "input": request.prompt,
        "stream": true,
        "store": false,
        "generation_config": { "max_output_tokens": request.max_output_tokens },
        // Top level, and *not* inside `generation_config`. The legacy
        // `:generateContent` API put the schema under
        // `generationConfig.responseSchema` alongside `responseMimeType`; the
        // Interactions API replaced both with this one polymorphic object, and
        // the legacy request schema was removed on 2026-06-08. Sending the old
        // shape now is a 400 with nothing generated.
        "response_format": {
            "type": "text",
            "mime_type": "application/json",
            "schema": request.schema(),
        },
    });
    if let Some(system) = &request.system {
        // A plain string at the top level, which is the shape the Interactions
        // API documents — not the legacy `system_instruction: {parts: [...]}`
        // object from `:generateContent`.
        body["system_instruction"] = Value::String(system.clone());
    }
    body
}

/// Whether the adapter should keep reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Continue,
    /// The API said the interaction is over, one way or the other. Returning
    /// here is what drops the response body and stops the meter.
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
    /// Which step is the answer. See [`MODEL_OUTPUT_STEP`] for why a step index
    /// has to be tracked at all.
    output_index: Option<u64>,
    json: String,
    output_complete: bool,
    /// The interaction's own last word on how it went: `completed`,
    /// `incomplete`, `failed`, `cancelled`, `budget_exceeded`. This is where a
    /// response cut off at `max_output_tokens` is announced — there is no
    /// separate finish-reason field.
    status: Option<String>,
    complete: bool,
    error: Option<Error>,
}

impl Incoming {
    pub(crate) fn new() -> Incoming {
        Incoming::default()
    }

    /// Take one `data:` payload.
    pub(crate) fn accept(&mut self, data: &str, deltas: &mut dyn DeltaSink) -> Flow {
        // A payload that is not JSON is not something to fail the call over.
        // Google closes every stream with `event: done` / `data: [DONE]`, and
        // the streaming guide asks explicitly that unknown events be handled
        // gracefully rather than thrown on — so failing here would turn a
        // documented sentinel, and every future addition, into a broken Enhance.
        let Ok(event) = serde_json::from_str::<Value>(data) else {
            return Flow::Continue;
        };

        // `metadata.total_usage` is documented as optional metadata that may
        // accompany *any* streamed event, so it is read before the match rather
        // than in one arm of it. This is the only figure available while the
        // response is still arriving, which is exactly the window in which a
        // cancellation or a dropped socket lands.
        self.read_usage(&event["metadata"]["total_usage"]);

        match event["event_type"].as_str().unwrap_or_default() {
            "interaction.created" => self.read_usage(&event["interaction"]["usage"]),
            "interaction.status_update" => self.read_status(&event["status"]),
            "step.start" => {
                if event["step"]["type"] == MODEL_OUTPUT_STEP {
                    self.output_index = event["index"].as_u64();
                }
            }
            "step.delta" if self.is_output_step(&event) => {
                // The document arrives as ordinary text deltas: with
                // `response_format.mime_type` set to `application/json` the
                // model's text *is* the JSON. The delta type is checked anyway
                // because the same step can carry `image` and `audio` deltas,
                // and one of those concatenated into the accumulator would be a
                // response that fails to parse for a reason nothing explains.
                if event["delta"]["type"] == "text"
                    && let Some(fragment) = event["delta"]["text"].as_str()
                {
                    self.json.push_str(fragment);
                    deltas.delta(fragment);
                }
            }
            "step.stop" if self.is_output_step(&event) => self.output_complete = true,
            "interaction.completed" => {
                self.read_usage(&event["interaction"]["usage"]);
                self.read_status(&event["interaction"]["status"]);
                self.complete = true;
                return Flow::Done;
            }
            // A 200 that fails partway through. It arrives as an event rather
            // than a status code, so an adapter that only maps statuses reports
            // a truncated response for what is really an overload.
            "error" => {
                let error = &event["error"];
                self.error = Some(error_for_code(
                    error["code"].as_str().unwrap_or_default(),
                    error["message"].as_str().unwrap_or_default(),
                    // No `retry-after` to read: the header was on a response
                    // that already succeeded. The job queue backs off on its own
                    // without one.
                    None,
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
        } else {
            match self.status.as_deref() {
                // `incomplete` is documented as "completed, but contains
                // incomplete results (e.g. hitting max_tokens)", and
                // `budget_exceeded` is the same thing with a different meter.
                // Reported before the validator sees it: a truncated object that
                // happens to parse would otherwise be written down as canon.
                Some("incomplete" | "budget_exceeded") => Err(Error::Truncated),
                // Server-side cancellation — a 499, or a stream the far end gave
                // up on. It has to reach the queue as a cancellation, or the
                // queue will retry it.
                Some("cancelled") => Err(Error::Cancelled),
                Some("failed") => Err(Error::Unavailable {
                    detail: format!("{LABEL} reported the interaction as failed"),
                }),
                // No `interaction.completed`, so the connection died mid-stream.
                _ if !self.complete => Err(Error::Truncated),
                // Validated regardless of what structured output promises about
                // schema conformance — see `validate.rs`, and the note on
                // `pattern` in `gemini.rs`.
                _ if self.output_complete => parse_description(kind, &self.json),
                // The interaction finished without ever opening a model output
                // step. A safety classifier declining is the likely cause, and
                // it has no variant of its own because the UI's taxonomy has no
                // code for it and the honest summary is the same either way:
                // what came back is not the document.
                _ if self.output_index.is_none() => Err(Error::NotJson(format!(
                    "the response contained no `{MODEL_OUTPUT_STEP}` step"
                ))),
                // The step opened and never closed, yet the interaction claims
                // it completed. Whatever accumulated is not known to be whole.
                _ => Err(Error::Truncated),
            }
        };
        EnhanceOutcome::new(usage, result)
    }

    fn is_output_step(&self, event: &Value) -> bool {
        self.output_index.is_some() && event["index"].as_u64() == self.output_index
    }

    fn read_status(&mut self, status: &Value) {
        if let Some(status) = status.as_str() {
            self.status = Some(status.to_string());
        }
    }

    /// Read whichever token counts this event carries.
    ///
    /// Fields are assigned only when present, because the totals are repeated on
    /// every event that carries them and an event may name only the ones it
    /// knows. They are cumulative, so the last sighting is the answer.
    fn read_usage(&mut self, usage: &Value) {
        let count = |key: &str| usage[key].as_u64().map(|n| n.min(u64::from(u32::MAX)) as u32);

        // `total_cached_tokens` is documented as "the cached part of the
        // prompt", so it is a *subset* of `total_input_tokens` rather than a
        // figure beside it — the opposite of Anthropic, where the two are
        // disjoint. Subtracting keeps `Usage::total_tokens` equal to Google's
        // own `total_tokens` instead of billing the cached part twice, and keeps
        // the split honest: cached input is a tenth of the price of fresh, so
        // folding the two together overstates what a call cost by an order of
        // magnitude. Saturating because a provider figure we cannot make sense of
        // must not panic a running job.
        let cached = count("total_cached_tokens");
        if let Some(cached) = cached {
            self.usage.cached_input_tokens = cached;
        }
        if let Some(input) = count("total_input_tokens") {
            self.usage.input_tokens = input.saturating_sub(cached.unwrap_or(0));
        }

        // Thoughts are billed at the output rate — Google's pricing table calls
        // the output column "including thinking tokens" — and are reported in a
        // field of their own. Adding them is the reading that cannot understate
        // the bill: if `total_output_tokens` already contains them the meter
        // runs high and someone notices, where the other way round a thinking
        // model spends most of its output invisibly.
        let output = count("total_output_tokens");
        let thoughts = count("total_thought_tokens");
        if output.is_some() || thoughts.is_some() {
            self.usage.output_tokens = output.unwrap_or(0).saturating_add(thoughts.unwrap_or(0));
        }
    }
}

impl SseConsumer for Incoming {
    fn event(&mut self, payload: &str, deltas: &mut dyn DeltaSink) -> bool {
        self.accept(payload, deltas) == Flow::Done
    }

    fn finish(self, kind: NodeKind, aborted: Option<Error>) -> EnhanceOutcome {
        self.outcome(kind, aborted)
    }
}

/// A non-2xx response, turned into something the UI has an answer for.
///
/// Two body shapes reach this and both are real. The Interactions API documents
/// `{"error": {"code": "<snake_case>", "message": "..."}}`; Google's older
/// `google.rpc.Status` envelope, which the edge still emits for quota and auth
/// refusals raised before the request reaches the API, puts an integer in `code`
/// and the name in `status` — `RESOURCE_EXHAUSTED`, `PERMISSION_DENIED`. Reading
/// both means a 429 from a load balancer maps like a 429 from the API.
///
/// The code decides wherever it is recognised, because it is finer than the
/// status: 429 alone cannot tell a per-minute rate limit from a daily quota, and
/// 400 alone cannot tell a prompt that does not fit from a schema we should not
/// have sent. The status is the fallback for a code nobody here has seen.
pub(crate) fn error_for_status(status: u16, body: &str, retry_after: Option<Duration>) -> Error {
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let error = &parsed["error"];
    let code = error["code"].as_str().or_else(|| error["status"].as_str()).unwrap_or_default();
    let message = error["message"].as_str().unwrap_or(body).trim();
    // The header first, then Google's own `RetryInfo` detail. Either beats a
    // backoff we would invent (#49 weighs it against the rest of the queue).
    let retry_after = retry_after.or_else(|| retry_info(error));

    match known_error(code, message, retry_after) {
        Some(error) => error,
        None => match status {
            401 => Error::BadKey { provider: LABEL },
            403 if mentions_billing(message) => Error::BillingRequired { provider: LABEL },
            403 => Error::BadKey { provider: LABEL },
            429 => Error::RateLimited { provider: LABEL, retry_after },
            400 if mentions_context(message) => Error::ContextTooLong,
            // Every other 4xx is a request we should not have sent: a schema the
            // provider will not take, a model id `project.json` names that does
            // not exist. Retrying spends money to fail identically, and
            // `SchemaRejected` is this crate's word for a refusal that is ours
            // to fix — which is why it lands on `internal` rather than on a
            // provider code.
            400..=499 => Error::SchemaRejected {
                detail: crate::transport::status_detail(status, code, message),
            },
            // 500, 503, 504, and anything unrecognised. Waiting is the right
            // answer to all of them.
            _ => Error::Unavailable {
                detail: crate::transport::status_detail(status, code, message),
            },
        },
    }
}

/// A failure delivered as an SSE `error` event, which arrives after a 200 and so
/// has no status to read. Google documents these as carrying the same `code`
/// strings the bodies carry.
pub(crate) fn error_for_code(code: &str, message: &str, retry_after: Option<Duration>) -> Error {
    known_error(code, message, retry_after).unwrap_or_else(|| Error::Unavailable {
        detail: if code.is_empty() { message.to_string() } else { format!("{code}: {message}") },
    })
}

/// The documented codes, and `None` for anything else.
///
/// The snake_case names are the Interactions API's own table
/// (<https://ai.google.dev/gemini-api/docs/api-errors>, read 2026-07-31); the
/// SCREAMING_CASE ones are `google.rpc.Code` names from the older envelope.
fn known_error(code: &str, message: &str, retry_after: Option<Duration>) -> Option<Error> {
    let error = match code {
        "authentication" | "UNAUTHENTICATED" => Error::BadKey { provider: LABEL },
        // One code for two different conversations: a key that is not allowed
        // near this model, and a project that has run out of money. Only the
        // message tells them apart, and sending a user to their billing page
        // over a project permission is a dead end.
        "permission_denied" | "PERMISSION_DENIED" if mentions_billing(message) => {
            Error::BillingRequired { provider: LABEL }
        }
        "permission_denied" | "PERMISSION_DENIED" => Error::BadKey { provider: LABEL },
        // `rate_limit_exceeded` is per-minute, `quota_exceeded` is the day's
        // allowance. Both are a wait, and the queue decides how long a wait it
        // is willing to sit through.
        "rate_limit_exceeded" | "quota_exceeded" | "RESOURCE_EXHAUSTED" => {
            Error::RateLimited { provider: LABEL, retry_after }
        }
        // The documented answer for "the free tier is not available in your
        // country and billing is not enabled" — which is a working key that will
        // never generate anything until the user acts, so telling them the key
        // is bad would send them to change the one thing that is fine.
        "FAILED_PRECONDITION" => Error::BillingRequired { provider: LABEL },
        // A model id from `project.json` that does not exist, or a path this
        // adapter built wrong. Both are ours to fix.
        "not_found" | "model_not_found" | "NOT_FOUND" => {
            Error::SchemaRejected { detail: format!("{code}: {message}") }
        }
        "invalid_request" | "parameter_unknown" | "INVALID_ARGUMENT"
            if mentions_context(message) =>
        {
            Error::ContextTooLong
        }
        "invalid_request" | "parameter_unknown" | "INVALID_ARGUMENT" => {
            Error::SchemaRejected { detail: format!("{code}: {message}") }
        }
        // 499: the far end decided the client had gone. Reaching the queue as
        // anything else would have it retried.
        "cancelled" | "CANCELLED" => Error::Cancelled,
        "api_error" | "service_unavailable" | "INTERNAL" | "UNAVAILABLE" | "DEADLINE_EXCEEDED" => {
            Error::Unavailable { detail: format!("{code}: {message}") }
        }
        _ if is_blocked(code) => Error::NotJson(format!(
            "{LABEL} blocked the response ({code}) instead of returning a description"
        )),
        _ => return None,
    };
    Some(error)
}

/// The codes for a response the model would not or could not produce, as opposed
/// to a call that failed.
///
/// Folded onto [`Error::NotJson`] rather than given variants of their own: the
/// UI's taxonomy has one code for "the model answered badly" and these are all
/// it, so splitting them here would produce error codes nothing switches on. The
/// one cost is that a safety block is retryable and will most likely fail
/// identically — which is still better than telling the user their network is
/// down.
fn is_blocked(code: &str) -> bool {
    matches!(
        code,
        "safety"
            | "recitation"
            | "language"
            | "prohibited_content"
            | "spii"
            | "blocklist"
            | "content_blocked"
            | "image_safety"
            | "image_prohibited_content"
            | "image_recitation"
            | "image_other"
            | "malformed_function_call"
            | "malformed_tool_call"
            | "unexpected_tool_call"
            | "too_many_tool_calls"
            | "missing_thought_signature"
            | "no_image"
    )
}

/// The wait Google itself asks for, out of the `google.rpc.RetryInfo` detail a
/// 429 body carries. Written to under-match: a delay we fail to find costs the
/// queue its own backoff, where one we invent costs a hammered key.
fn retry_info(error: &Value) -> Option<Duration> {
    let details = error["details"].as_array()?;
    details.iter().find_map(|detail| {
        if !detail["@type"].as_str()?.ends_with("google.rpc.RetryInfo") {
            return None;
        }
        // `google.protobuf.Duration` in JSON: seconds with an optional
        // fractional part and a trailing `s`, as in "42s" or "1.500s".
        let delay = detail["retryDelay"].as_str().or_else(|| detail["retry_delay"].as_str())?;
        let seconds: f64 = delay.trim_end_matches('s').parse().ok()?;
        (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
    })
}

/// The phrases Google uses when the prompt itself is the problem. There is no
/// machine-readable subtype for this, so it is a string match — and it is
/// written to under-match: guessing wrong here costs a retry that fails, where
/// the fallback costs a clear "this is our bug" in the log.
fn mentions_context(message: &str) -> bool {
    let message = message.to_lowercase();
    message.contains("token count")
        || message.contains("context limit")
        || message.contains("context window")
        || message.contains("input token limit")
        || message.contains("exceeds the maximum number of tokens")
}

fn mentions_billing(message: &str) -> bool {
    let message = message.to_lowercase();
    message.contains("billing")
        || message.contains("credit")
        || message.contains("payment")
        || message.contains("free tier")
        || message.contains("paid tier")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Discard;
    use crate::stream::testing::document;
    use wobu_core::schema::description_schema;

    fn request() -> EnhanceRequest {
        EnhanceRequest::new(NodeKind::Character, "gemini-3.6-flash", "Describe Vashk.")
            .with_system("You describe how things look.")
    }

    #[test]
    fn the_request_carries_the_kinds_own_schema_in_a_top_level_response_format() {
        // The two mistakes that would silently produce prose instead of a
        // document: the schema going in under `generation_config`, where the
        // legacy API wanted it and where the Interactions API ignores it; and
        // the mime type being left off, so `application/json` is never asked
        // for.
        let body = request_body(&request());
        assert_eq!(body["response_format"]["type"], "text");
        assert_eq!(body["response_format"]["mime_type"], "application/json");
        assert_eq!(
            body["response_format"]["schema"],
            request().schema(),
            "the schema sent must be the one the validator applies, unedited",
        );
        assert_eq!(
            body["response_format"]["schema"]["properties"]["palette"],
            description_schema(NodeKind::Character)["properties"]["palette"],
        );
        assert!(body["generation_config"].get("response_schema").is_none());
        assert!(body["generation_config"].get("response_mime_type").is_none());
        assert_eq!(body["model"], "gemini-3.6-flash");
        assert_eq!(body["input"], "Describe Vashk.");
        assert_eq!(body["stream"], true);
        assert_eq!(body["system_instruction"], "You describe how things look.");
        assert_eq!(
            body["generation_config"]["max_output_tokens"],
            crate::provider::DEFAULT_MAX_OUTPUT_TOKENS,
        );
    }

    #[test]
    fn the_request_sends_the_same_schema_anthropic_gets_for_every_kind() {
        // The point of one generated schema. If either adapter edited it — to
        // drop the palette's `pattern`, say, which Google does not list among
        // the keywords it supports — the two providers would be answering
        // different questions, and the difference would show up as one of them
        // "ignoring" a schema it was never sent.
        for def in wobu_core::kind::kind_registry() {
            let request = EnhanceRequest::new(def.kind, "gemini-3.6-flash", "…");
            let ours = request_body(&request)["response_format"]["schema"].clone();
            let theirs =
                crate::anthropic::wire::request_body(&request)["tools"][0]["input_schema"].clone();
            assert_eq!(ours, theirs, "{} is sent two different schemas", def.kind);
        }
    }

    #[test]
    fn the_request_opts_out_of_server_side_storage() {
        // The Interactions API keeps the request and the response by default so
        // a later call can chain onto them. We never chain, and the payload is
        // somebody's unpublished world.
        assert_eq!(request_body(&request())["store"], false);
    }

    #[test]
    fn the_request_carries_no_sampling_parameters() {
        // `temperature`, `top_p` and `top_k` were deprecated on 2026-07-21, and
        // a knob added here later would be one Settings has no slider for.
        let body = request_body(&request());
        for absent in ["temperature", "top_p", "top_k"] {
            assert!(body.get(absent).is_none(), "{absent} should not be sent");
            assert!(
                body["generation_config"].get(absent).is_none(),
                "{absent} should not be sent under generation_config",
            );
        }
    }

    #[test]
    fn a_request_without_standing_instructions_sends_no_system_instruction() {
        // `"system_instruction": null` is not the same as no field, and only one
        // of them is a request Google accepts.
        let body = request_body(&EnhanceRequest::new(NodeKind::Prop, "m", "A censer."));
        assert!(body.get("system_instruction").is_none());
    }

    /// The documented stream for a structured response, in the shape
    /// `ai.google.dev/gemini-api/docs/interactions/streaming` gives it: a
    /// thought step first, on its own index and with deltas of its own, then the
    /// model output step, then the completion that carries the usage.
    fn interaction_stream(document: &str, status: &str, closed: bool) -> Vec<String> {
        let mut events = vec![
            json!({"event_type": "interaction.created", "interaction": {"id": "v1_abc",
                   "status": "in_progress", "object": "interaction",
                   "model": "gemini-3.6-flash"}})
            .to_string(),
            json!({"event_type": "interaction.status_update", "interaction_id": "v1_abc",
                   "status": "in_progress"})
            .to_string(),
            json!({"event_type": "step.start", "index": 0, "step": {"type": "thought"}})
                .to_string(),
            json!({"event_type": "step.delta", "index": 0,
                   "delta": {"type": "thought_signature", "signature": "Cg0Ktz"}})
            .to_string(),
            json!({"event_type": "step.delta", "index": 0,
                   "delta": {"type": "text", "text": "The user wants a censer, so "}})
            .to_string(),
            json!({"event_type": "step.stop", "index": 0}).to_string(),
            json!({"event_type": "step.start", "index": 1, "step": {"type": "model_output"}})
                .to_string(),
        ];
        for fragment in document.as_bytes().chunks(11) {
            let fragment = std::str::from_utf8(fragment).unwrap();
            events.push(
                json!({"event_type": "step.delta", "index": 1,
                       "delta": {"type": "text", "text": fragment}})
                .to_string(),
            );
        }
        events.push(json!({"event_type": "step.stop", "index": 1}).to_string());
        if closed {
            events.push(
                json!({"event_type": "interaction.completed", "interaction": {"id": "v1_abc",
                       "status": status, "usage": {"total_input_tokens": 812,
                       "total_output_tokens": 289, "total_tokens": 1101}}})
                .to_string(),
            );
            events.push("[DONE]".to_string());
        }
        events
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
    fn a_complete_interaction_becomes_a_validated_description() {
        let events = interaction_stream(&document(NodeKind::Character), "completed", true);
        let outcome = feed(&events, &mut Discard).outcome(NodeKind::Character, None);
        let validated = outcome.result.expect("a whole interaction should validate");
        assert!(validated.extra_sections.is_empty());
        assert_eq!(outcome.usage.input_tokens, 812);
        assert_eq!(outcome.usage.output_tokens, 289);
    }

    #[test]
    fn only_the_model_output_steps_text_reaches_the_sink() {
        // The regression, and the one unique to this vendor: a thought step
        // emits `step.delta` on the same event type as the answer, and with
        // thinking summaries on it emits prose. A sentence of reasoning in front
        // of the opening brace makes every fragment the editor parses
        // unparseable, and makes the accumulated text fail validation for a
        // reason no error message would explain.
        let expected = document(NodeKind::Character);
        let mut streamed = String::new();
        let mut sink = |json: &str| streamed.push_str(json);
        feed(&interaction_stream(&expected, "completed", true), &mut sink);
        assert_eq!(streamed, expected);
    }

    #[test]
    fn an_interaction_that_ends_incomplete_is_a_failure_rather_than_a_short_description() {
        // `incomplete` is how hitting `max_output_tokens` is announced — there
        // is no finish-reason field. A cap reached mid-object leaves JSON that
        // may still parse if the cut lands luckily, and believing it would save
        // half a description as canon.
        let events = interaction_stream(&document(NodeKind::Character), "incomplete", true);
        let outcome = feed(&events, &mut Discard).outcome(NodeKind::Character, None);
        assert!(matches!(outcome.result, Err(Error::Truncated)));
        assert_eq!(outcome.usage.output_tokens, 289, "the cap was reached, so it was billed");
    }

    #[test]
    fn a_stream_that_stops_before_interaction_completed_is_truncated_not_accepted() {
        // Everything arrived except the API's word that the interaction is over.
        // Without that there is no way to know whether more was coming.
        let events = interaction_stream(&document(NodeKind::Character), "completed", false);
        let outcome = feed(&events, &mut Discard).outcome(NodeKind::Character, None);
        assert!(matches!(outcome.result, Err(Error::Truncated)));
    }

    #[test]
    fn an_interaction_with_no_model_output_step_is_reported_rather_than_parsed() {
        // A safety classifier declining leaves a completed interaction with
        // nothing in it. The alternative is an empty string handed to the
        // validator and a confusing "not JSON" about nothing.
        let events = [
            json!({"event_type": "interaction.created", "interaction": {"status": "in_progress"}})
                .to_string(),
            json!({"event_type": "interaction.completed", "interaction": {"status": "completed",
                   "usage": {"total_input_tokens": 5, "total_output_tokens": 9}}})
            .to_string(),
        ];
        let outcome = feed(&events, &mut Discard).outcome(NodeKind::Character, None);
        let error = outcome.result.expect_err("there is no description here");
        assert!(error.to_string().contains("model_output"), "{error}");
        assert!(error.is_retryable());
        assert_eq!(outcome.usage.output_tokens, 9);
    }

    #[test]
    fn a_failed_or_cancelled_interaction_lands_on_the_variant_that_matches_it() {
        // `failed` is worth waiting out and `cancelled` must never be retried,
        // so folding the two together either bills the user for pressing Stop or
        // gives up on a transient failure.
        for (status, retryable) in [("failed", true), ("cancelled", false)] {
            let events = [json!({"event_type": "interaction.completed",
                       "interaction": {"status": status, "usage": {"total_input_tokens": 3}}})
            .to_string()];
            let outcome = feed(&events, &mut Discard).outcome(NodeKind::Character, None);
            let error = outcome.result.expect_err("neither status carries a description");
            assert_eq!(error.is_retryable(), retryable, "{status} -> {error}");
            assert_eq!(outcome.usage.input_tokens, 3);
        }
    }

    #[test]
    fn an_error_event_after_a_200_stops_the_read_and_keeps_the_usage() {
        // Google sends overload and gateway failures mid-stream, after the
        // status line has already promised success. An adapter that only maps
        // statuses calls this a truncated response and tells the user their
        // model is babbling.
        let events = [
            json!({"event_type": "interaction.created",
                   "metadata": {"total_usage": {"total_input_tokens": 812}}})
            .to_string(),
            json!({"event_type": "error",
                   "error": {"code": "service_unavailable", "message": "Overloaded"}})
            .to_string(),
            json!({"event_type": "interaction.completed", "interaction": {"status": "failed"}})
                .to_string(),
        ];
        let outcome = feed(&events, &mut Discard).outcome(NodeKind::Character, None);
        assert!(matches!(outcome.result, Err(Error::Unavailable { .. })));
        assert_eq!(outcome.usage.input_tokens, 812);
    }

    #[test]
    fn usage_riding_on_any_event_is_read_because_a_failure_may_be_all_we_get() {
        // `metadata.total_usage` is documented as optional on *any* streamed
        // event, and it is the only figure that exists before
        // `interaction.completed`. Reading it only from the completion would
        // report zero for every cancelled and every dropped call — which is
        // exactly when the user needs to be told what it cost.
        let events = [
            json!({"event_type": "step.delta", "index": 0, "delta": {"type": "text", "text": "x"},
                   "metadata": {"total_usage": {"total_input_tokens": 900,
                   "total_cached_tokens": 400, "total_output_tokens": 40,
                   "total_thought_tokens": 60}}})
            .to_string(),
        ];
        let usage =
            feed(&events, &mut Discard).outcome(NodeKind::Character, Some(Error::Cancelled)).usage;
        // 900 includes the 400 cached, so the fresh half is 500 — and thoughts
        // are billed at the output rate, so 40 + 60.
        assert_eq!(usage.input_tokens, 500);
        assert_eq!(usage.cached_input_tokens, 400);
        assert_eq!(usage.output_tokens, 100);
        assert_eq!(usage.total_tokens(), 1000, "Google's own total_tokens for these figures");
    }

    #[test]
    fn an_unknown_event_type_is_ignored_rather_than_failing_the_call() {
        // Google's streaming guide asks explicitly that unknown events be
        // handled gracefully, and every stream ends with a `[DONE]` sentinel
        // that is not JSON at all. Treating either as a failure would break
        // Enhance on a change that was announced as safe.
        let mut events = interaction_stream(&document(NodeKind::Character), "completed", true);
        events.insert(1, json!({"event_type": "something_from_2027", "detail": {}}).to_string());
        events.insert(2, "[DONE]".to_string());
        events.insert(3, "not json at all".to_string());
        let outcome = feed(&events, &mut Discard).outcome(NodeKind::Character, None);
        assert!(outcome.is_ok());
    }

    #[test]
    fn a_caller_that_gave_up_outranks_whatever_the_stream_was_saying() {
        // Cancellation must reach the queue as a cancellation. Anything else
        // gets retried, and the user is billed for pressing Stop.
        let events = interaction_stream(&document(NodeKind::Character), "completed", true);
        let outcome =
            feed(&events[..3], &mut Discard).outcome(NodeKind::Character, Some(Error::Cancelled));
        assert!(matches!(outcome.result, Err(Error::Cancelled)));
    }

    #[test]
    fn every_documented_error_code_lands_on_the_variant_the_ui_can_answer() {
        // The regression this guards is a user staring at "the provider could
        // not be reached" over a key they could paste in Settings. The codes are
        // the table at ai.google.dev/gemini-api/docs/api-errors, read
        // 2026-07-31.
        let cases: &[(u16, &str, &str, &str)] = &[
            (401, "authentication", "API key not valid", "provider.bad_key"),
            (403, "permission_denied", "not authorized for this model", "provider.bad_key"),
            (
                403,
                "permission_denied",
                "billing is not enabled on this project",
                "provider.billing_required",
            ),
            (404, "model_not_found", "gemini-9.9-flash", "internal"),
            (429, "rate_limit_exceeded", "requests per minute", "provider.rate_limited"),
            (429, "quota_exceeded", "daily quota", "provider.rate_limited"),
            (
                400,
                "invalid_request",
                "the input token count 1200000 exceeds the maximum number of tokens",
                "provider.context_too_long",
            ),
            (400, "invalid_request", "unknown field `pattern` in schema", "internal"),
            (400, "parameter_unknown", "unknown parameter: thinking", "internal"),
            (499, "cancelled", "client closed request", "cancelled"),
            (500, "api_error", "internal error", "provider.unavailable"),
            (503, "service_unavailable", "the model is overloaded", "provider.unavailable"),
            (400, "safety", "blocked by a safety filter", "provider.bad_response"),
            (400, "prohibited_content", "blocked", "provider.bad_response"),
        ];
        for (status, code, message, expected) in cases {
            let body = json!({"error": {"code": code, "message": message}});
            let error = error_for_status(*status, &body.to_string(), None);
            assert_eq!(error.code(), *expected, "{status} {code}: {message}");
        }
    }

    #[test]
    fn the_older_google_rpc_envelope_maps_the_same_way_as_the_new_one() {
        // A 429 raised by Google's edge before the request reaches the
        // Interactions API arrives as `{"code": 429, "status":
        // "RESOURCE_EXHAUSTED"}` — an integer where the new shape has a name. An
        // adapter that only reads the new shape calls the commonest failure on
        // the free tier "unavailable" and sends the user to check their network.
        let cases: &[(u16, &str, &str, &str)] = &[
            (429, "RESOURCE_EXHAUSTED", "Resource has been exhausted", "provider.rate_limited"),
            (403, "PERMISSION_DENIED", "denied access", "provider.bad_key"),
            (401, "UNAUTHENTICATED", "API key not valid", "provider.bad_key"),
            (
                400,
                "FAILED_PRECONDITION",
                "the free tier is not available in your country",
                "provider.billing_required",
            ),
            (500, "INTERNAL", "internal error", "provider.unavailable"),
            (503, "UNAVAILABLE", "overloaded", "provider.unavailable"),
        ];
        for (status, name, message, expected) in cases {
            let body = json!({"error": {"code": status, "status": name, "message": message}});
            let error = error_for_status(*status, &body.to_string(), None);
            assert_eq!(error.code(), *expected, "{status} {name}: {message}");
        }
    }

    #[test]
    fn a_rate_limit_carries_googles_own_wait_rather_than_a_guess() {
        // Google puts the wait in a `RetryInfo` detail rather than in a header,
        // so an adapter that only reads `retry-after` throws away the one number
        // that would stop the queue hammering a key already over quota.
        let body = json!({"error": {"code": 429, "status": "RESOURCE_EXHAUSTED",
            "message": "Resource has been exhausted", "details": [
                {"@type": "type.googleapis.com/google.rpc.QuotaFailure", "violations": []},
                {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "42s"}]}});
        assert!(matches!(
            error_for_status(429, &body.to_string(), None),
            Error::RateLimited { retry_after: Some(wait), .. } if wait == Duration::from_secs(42)
        ));

        // A `retry-after` header, where one exists, is read by the caller and
        // outranks the body.
        assert!(matches!(
            error_for_status(429, &body.to_string(), Some(Duration::from_secs(7))),
            Error::RateLimited { retry_after: Some(wait), .. } if wait == Duration::from_secs(7)
        ));
    }

    #[test]
    fn a_status_with_an_unreadable_body_still_reaches_the_right_variant() {
        // Google's edge answers some refusals before the request reaches the
        // API, and the body is HTML. Depending on the JSON shape would make that
        // a mystery.
        assert!(matches!(error_for_status(401, "<html>...", None), Error::BadKey { .. }));
        assert!(matches!(
            error_for_status(429, "", None),
            Error::RateLimited { retry_after: None, .. }
        ));
        assert!(matches!(error_for_status(400, "", None), Error::SchemaRejected { .. }));
        assert!(matches!(error_for_status(502, "bad gateway", None), Error::Unavailable { .. }));
    }

    #[test]
    fn a_mid_stream_error_maps_by_code_because_there_is_no_status_left_to_read() {
        assert!(matches!(
            error_for_code("service_unavailable", "Overloaded", None),
            Error::Unavailable { .. }
        ));
        assert!(matches!(error_for_code("authentication", "", None), Error::BadKey { .. }));
        assert!(matches!(
            error_for_code("rate_limit_exceeded", "", None),
            Error::RateLimited { retry_after: None, .. }
        ));
        assert!(matches!(error_for_code("gateway_timeout", "", None), Error::Unavailable { .. }));
        assert!(matches!(
            error_for_code("a_code_from_2027", "who knows", None),
            Error::Unavailable { .. }
        ));
    }

    #[test]
    fn a_detail_that_reaches_a_log_names_the_status_that_produced_it() {
        // `provider.unavailable` and `internal` both say almost nothing on their
        // own. Whoever reads the log needs to know whether they are looking at a
        // 502 to wait out or a 400 to go and fix.
        let error = error_for_status(502, "<html>bad gateway</html>", None);
        let message = error.to_string();
        assert!(message.contains("502"), "{message}");
        assert!(message.contains("bad gateway"), "{message}");
    }
}
