//! What we send Google, what we make of what comes back, and how their failures
//! become ours.
//!
//! Everything here is a pure function of bytes, so the half of the adapter where
//! a misread field is a wrong picture rather than a compile error can be driven
//! from recorded payloads instead of from a paid call. The HTTP itself lives next
//! door in `mod.rs` and is the only part a test cannot reach — the same split as
//! `comfy/wire.rs` and `wobu-llm`'s two.
//!
//! Every shape below was read off Google's own documentation on **2026-08-01**,
//! not remembered and not carried over from the text adapter. The pages, so the
//! next person can re-read rather than trust:
//!
//! - Request, reference images and sizes:
//!   <https://ai.google.dev/gemini-api/docs/image-generation> (last updated
//!   2026-07-30)
//! - Field-by-field reference, including `ImageResponseFormat` and `ImageDelta`:
//!   <https://ai.google.dev/api/interactions-api> (last updated 2026-07-31)
//! - Where the config moved and when the old one was removed:
//!   <https://ai.google.dev/gemini-api/docs/interactions-breaking-changes-may-2026>
//! - Error codes: <https://ai.google.dev/gemini-api/docs/api-errors>
//! - Prices and the free tier: <https://ai.google.dev/gemini-api/docs/pricing>
//!
//! ## The field `docs/08-providers.md` flagged, settled
//!
//! The doc's 🚩 was that "the image config field moved between
//! `generationConfig.imageConfig` and `response_format.image` and the docs
//! disagree". Re-checked live on 2026-08-01, they no longer do, and neither
//! candidate was quite right:
//!
//! - The current shape is a top-level **`response_format` whose `type` is
//!   `"image"`**, carrying snake_case `aspect_ratio` and `image_size` — not a
//!   nested `response_format.image`. `ImageResponseFormat` is one of the four
//!   variants of the polymorphic `response_format` in the API reference,
//!   alongside the `type: "text"` one the Enhance adapter sends.
//! - `generation_config.image_config` is the **legacy** form. The
//!   breaking-changes page says in as many words that "`image_config` moves from
//!   `generation_config` to `response_format`" and that "the new schema removes
//!   `image_config` from `generation_config`", and the legacy schema was removed
//!   on 2026-06-08. It survives on that page only as the "Before" half of a
//!   migration example, which is the most likely source of the disagreement the
//!   doc recorded.
//!
//! `imageConfig` in camelCase appears on none of the five pages checked.
//!
//! ## What is still 🚩
//!
//! - **The top-level envelope of a non-streaming response.** Every worked example
//!   published is SDK code reading `interaction.steps`, and the reference notes
//!   that `output_image` is "added by the SDK" rather than being a wire field. So
//!   whether the JSON body is the interaction or is an object with the
//!   interaction under `interaction` is not confirmed; [`image`] accepts either
//!   and neither is a guess about *meaning*.
//! - **The sub-1K size token.** The reference's enum value is `512`, the guide's
//!   prose writes `512px`. Not resolved, and not reached: no ceiling this adapter
//!   declares produces a long side under 513, so `size_class` in `mod.rs` never
//!   emits one.
//! - **`mime_type` on the way out.** `ImageResponseFormat.mime_type` lists only
//!   `image/jpeg`, while every input example uses `image/png` and PNG comes back
//!   in practice. Rather than pick, the request omits the field and takes the
//!   model's default; what actually arrives is read off the bytes.

use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};

use crate::backend::ImageRequest;
use crate::error::Error;

use super::{LABEL, no_free_tier};

/// The step type carrying the answer, and the same constant the text adapter
/// tracks: an interaction is a numbered sequence of steps and only one of them
/// is the output. A `thought` step is real, arrives first, and carries no
/// picture.
const MODEL_OUTPUT_STEP: &str = "model_output";

/// The body of one Generate request.
///
/// Deliberately small, and every parameter the API accepts that is not here is
/// absent on purpose:
///
/// - **No `stream`.** There is one image and nothing before it, so a stream would
///   be an SSE frame per chunk of one base64 blob and a `Capabilities` that
///   promised previews it cannot draw. Google's own REST examples for image
///   generation are unstreamed.
/// - **No `previous_interaction_id`.** That is how the API chains an edit onto a
///   previous generation, and every wobu generation is built from the influence
///   stack rather than from the last picture — one request is one image
///   (`backend.rs`).
/// - **No `generation_config`.** `max_output_tokens` is a text setting; images
///   are billed by size class, which is in `response_format`.
/// - **No negative prompt**, because there is no field for one anywhere in the
///   API. `Capabilities::negative_prompt` is false, so `negotiate` never compiles
///   one, and `mod.rs` refuses a request that carries one anyway rather than
///   dropping it.
///
/// `store: false` is carried over from the text adapter, and the argument is
/// stronger here: the API keeps the request and the response server-side by
/// default so a later call can chain onto them, and for an image request that
/// means somebody's unpublished concept art and every reference photo they
/// attached to it sitting in a Google project until it is deleted. We never
/// chain, so there is nothing to trade for it.
pub(crate) fn request_body(request: &ImageRequest, image_size: &str) -> Value {
    json!({
        "model": request.model,
        "input": input(request),
        "store": false,
        // Top level, `type: "image"`, snake_case — see the note on this module.
        // The legacy `generation_config.image_config` was removed on 2026-06-08
        // and sending it now is a 400 with nothing generated.
        "response_format": {
            "type": "image",
            "aspect_ratio": request.aspect.to_string(),
            "image_size": image_size,
            // Asked for explicitly rather than defaulted. `delivery` is an enum
            // of `inline` and `uri`; this adapter reads base64 out of the
            // response and has no fetcher, so a default that ever became `uri`
            // would turn every generation into "the backend returned no image".
            // 🚩 If a 400 ever names `delivery`, this is the field to drop —
            // inline is what the documented examples return today.
            "delivery": "inline",
        },
    })
}

/// The prompt and every reference image, as one flat list of typed blocks.
///
/// **There is no per-bucket field to route into**, which is the finding
/// [#86](https://github.com/krazyjakee/wobu/issues/86) is asking about. Google's
/// own multi-image example is exactly this: one `text` block and then N `image`
/// blocks, undifferentiated, with no `role`, no `parts` and no name on any
/// picture. So `RefBucket` fits this vendor as a *budget* — which is what
/// `Capabilities::image_refs` uses it for, and the per-model caps of 6/5/3 are
/// real and documented — and does not correspond to anything in the request. The
/// consequence: `ImageRequest::in_bucket` is not reached here, and the order
/// references arrive in is the only signal about them that survives.
///
/// Text first, then pictures, which is the order every published example uses.
fn input(request: &ImageRequest) -> Value {
    let mut blocks = vec![json!({ "type": "text", "text": request.prompt })];
    blocks.extend(request.references.iter().map(|reference| {
        json!({
            "type": "image",
            "mime_type": reference.mime,
            // Inline base64. There are no URLs on this API, and the Files API —
            // which the docs offer for larger payloads — would mean an upload,
            // a handle to clean up, and a second thing to get wrong for a
            // reference image that is a few hundred kilobytes.
            "data": base64::engine::general_purpose::STANDARD.encode(&reference.bytes),
        })
    }));
    Value::Array(blocks)
}

/// One image, as it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Returned {
    pub(crate) bytes: Vec<u8>,
    /// What the response *said* it was. Only ever used in an error message: what
    /// the bytes actually are is `dimensions::mime`, for the same reason the size
    /// is read off the header.
    pub(crate) mime: String,
}

/// The picture out of a 200, or the reason there isn't one.
///
/// The order matters and is the point of writing this as one function: **an image
/// in hand outranks whatever the status field says**. A response that carries
/// pixels has been generated and billed, and refusing it because the interaction
/// called itself `incomplete` would charge the user for a picture and then throw
/// it away.
pub(crate) fn image(body: &[u8]) -> Result<Returned, Error> {
    let Ok(document) = serde_json::from_slice::<Value>(body) else {
        return Err(Error::NotAnImage {
            detail: format!("{LABEL} answered with {} bytes that are not JSON", body.len()),
        });
    };
    // The interaction, wherever it is. 🚩 Every published example is SDK code
    // reading `interaction.steps`, so whether the body *is* the interaction or
    // wraps it is unconfirmed; both are accepted rather than one being guessed
    // at.
    let interaction =
        if document.get("steps").is_some() { &document } else { &document["interaction"] };

    if let Some(block) = image_block(interaction) {
        let mime = block["mime_type"].as_str().unwrap_or_default().to_owned();
        let Some(data) = block["data"].as_str() else {
            // `uri` is the other half of the `delivery` enum, and the request
            // asks for `inline`. Reported rather than fetched: a URL we followed
            // would be a second request against a link nothing in this crate can
            // hold, and the field being populated at all means the request was
            // not honoured.
            return Err(Error::NotAnImage {
                detail: match block["uri"].as_str() {
                    Some(_) => format!(
                        "{LABEL} returned a link to the image rather than the image, despite \
                         being asked for inline delivery"
                    ),
                    None => format!("{LABEL} returned an image block with no data in it"),
                },
            });
        };
        return match base64::engine::general_purpose::STANDARD.decode(data) {
            Ok(bytes) => Ok(Returned { bytes, mime }),
            Err(e) => Err(Error::NotAnImage {
                detail: format!(
                    "{LABEL} returned {} that is not valid base64: {e}",
                    mime_or(&mime)
                ),
            }),
        };
    }

    // No picture. Whatever else the response says is now the whole of what we
    // know, and each of these sends the user somewhere different.
    let error = &interaction["error"];
    if let Some(code) = error["code"].as_str() {
        let message = error["message"].as_str().unwrap_or_default();
        // No `retry-after` to read: the header was on a response that already
        // succeeded. The job queue backs off on its own without one.
        return Err(known_error(code, message, None)
            .unwrap_or_else(|| Error::Unavailable { detail: format!("{code}: {message}") }));
    }
    match interaction["status"].as_str() {
        // A 499 or a stream the far end gave up on. It has to reach the queue as
        // a cancellation, or the queue retries something the user stopped.
        Some("cancelled") => Err(Error::Cancelled),
        Some("failed") => {
            Err(Error::Unavailable { detail: format!("{LABEL} reported the generation as failed") })
        }
        // `incomplete` and `budget_exceeded` mean a cap was reached. For text
        // that is half a description; here it is no picture at all, and there is
        // nothing to salvage.
        Some(status @ ("incomplete" | "budget_exceeded")) => Err(Error::Unavailable {
            detail: format!("{LABEL} stopped before it had generated an image ({status})"),
        }),
        // A completed interaction with no image in it. `error.rs` keeps this
        // apart from `Refused` because a silent empty result and a stated refusal
        // send the user to two different places — one is a prompt to edit and the
        // other is a bug report.
        _ => Err(Error::NoImage),
    }
}

/// The first image block of the output step.
///
/// Steps are searched in order and the `model_output` one is preferred, because a
/// `thought` step is a real step that arrives first — the text adapter tracks the
/// same distinction for the same reason. The fallback to *any* step's image
/// exists because a response that carried a picture outside the step we expected
/// is still a picture the user paid for.
fn image_block(interaction: &Value) -> Option<&Value> {
    let steps = interaction["steps"].as_array()?;
    fn picture(step: &Value) -> Option<&Value> {
        step["content"].as_array()?.iter().find(|block| block["type"] == "image")
    }
    steps
        .iter()
        .filter(|step| step["type"] == MODEL_OUTPUT_STEP)
        .find_map(picture)
        .or_else(|| steps.iter().find_map(picture))
}

fn mime_or(mime: &str) -> String {
    match mime {
        "" => "an image".to_owned(),
        mime => mime.to_owned(),
    }
}

/// A non-2xx response, turned into something the UI has an answer for.
///
/// Two body shapes reach this and both are real, exactly as on the text side. The
/// Interactions API documents a flat `{"error": {"code": "<snake_case>",
/// "message": "..."}}`; Google's older `google.rpc.Status` envelope, which the
/// edge still emits for refusals raised before the request reaches the API, puts
/// an integer in `code` and the name in `status` — `RESOURCE_EXHAUSTED`,
/// `PERMISSION_DENIED`. Reading both means a 429 from a load balancer maps like a
/// 429 from the API.
///
/// The code decides wherever it is recognised, because it is finer than the
/// status. That matters more here than it does for text: **on these models a 429
/// is as likely to mean "this account has no billing" as "you are going too
/// fast"**, and those two are a link to a payment page and a wait respectively.
pub(crate) fn error_for_status(status: u16, body: &[u8], retry_after: Option<Duration>) -> Error {
    let parsed: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let error = &parsed["error"];
    let code = error["code"].as_str().or_else(|| error["status"].as_str()).unwrap_or_default();
    let text = String::from_utf8_lossy(body);
    let message = error["message"].as_str().unwrap_or(&text).trim();
    // The header first, then Google's own `RetryInfo` detail. 🚩 The
    // Interactions error page no longer documents `RetryInfo` at all and asks for
    // plain exponential backoff, but the older envelope still carries one and
    // reading a number Google supplied can only beat a number we invented.
    let retry_after = retry_after.or_else(|| retry_info(error));

    match known_error(code, message, retry_after) {
        Some(error) => error,
        // Nothing in the code table matched, so fall back to the status — which
        // is all there is when Google's edge answers with HTML.
        None => match status {
            401 => Error::BadKey { backend: LABEL },
            403 if mentions_billing(message) => no_free_tier(),
            403 => Error::BadKey { backend: LABEL },
            // The commonest way the billing failure actually arrives. See
            // `no_billing_here`.
            429 if no_billing_here(message) => no_free_tier(),
            429 => Error::RateLimited { backend: LABEL, retry_after },
            404 => Error::Unavailable { detail: unknown_model(message) },
            // Every other 4xx is a request we should not have sent: a field this
            // adapter built wrong, a value the API stopped accepting. Retrying
            // spends money to fail identically, and `Unsupported` is this crate's
            // word for a refusal that is ours to fix.
            400..=499 => Error::Unsupported { detail: detail(status, code, message) },
            // 500, 503, 504, and anything unrecognised. Waiting is the right
            // answer to all of them.
            _ => Error::Unavailable { detail: detail(status, code, message) },
        },
    }
}

/// The documented codes, and `None` for anything else.
///
/// The snake_case names are the Interactions API's own table
/// (<https://ai.google.dev/gemini-api/docs/api-errors>, read 2026-08-01); the
/// SCREAMING_CASE ones are `google.rpc.Code` names from the older envelope.
fn known_error(code: &str, message: &str, retry_after: Option<Duration>) -> Option<Error> {
    let error = match code {
        "authentication" | "UNAUTHENTICATED" => Error::BadKey { backend: LABEL },
        // One code for two different conversations: a key that is not allowed
        // near this model, and a project that cannot pay. Only the message tells
        // them apart, and sending a user to their billing page over a project
        // permission is a dead end.
        "permission_denied" | "PERMISSION_DENIED" if mentions_billing(message) => no_free_tier(),
        "permission_denied" | "PERMISSION_DENIED" => Error::BadKey { backend: LABEL },
        // **The one that matters on this backend.** `rate_limit_exceeded` is
        // per-minute and `quota_exceeded` is the day's allowance — but an image
        // model on an account with no billing has a free-tier allowance of zero,
        // and a quota of zero arrives as one of these. Waiting out a limit of
        // zero is waiting forever.
        "rate_limit_exceeded" | "quota_exceeded" | "RESOURCE_EXHAUSTED"
            if no_billing_here(message) =>
        {
            no_free_tier()
        }
        "rate_limit_exceeded" | "quota_exceeded" | "RESOURCE_EXHAUSTED" => {
            Error::RateLimited { backend: LABEL, retry_after }
        }
        // The older envelope's answer for "the free tier is not available in your
        // country and billing is not enabled". 🚩 Absent from the current
        // Interactions error page, kept because it costs nothing and the edge
        // still speaks the old envelope.
        "FAILED_PRECONDITION" => no_free_tier(),
        "not_found" | "model_not_found" | "NOT_FOUND" => {
            Error::Unavailable { detail: unknown_model(message) }
        }
        "invalid_request" | "parameter_unknown" | "INVALID_ARGUMENT" => {
            Error::Unsupported { detail: format!("{code}: {message}") }
        }
        "cancelled" | "CANCELLED" => Error::Cancelled,
        "api_error" | "service_unavailable" | "INTERNAL" | "UNAVAILABLE" | "DEADLINE_EXCEEDED" => {
            Error::Unavailable { detail: format!("{code}: {message}") }
        }
        // Documented on the errors page as its own generation failure: "the model
        // was unable to generate an image". Not a refusal — nothing objected —
        // which `error.rs` keeps apart because a stated refusal is a prompt to
        // edit and a silent empty result is a bug report.
        "no_image" => Error::NoImage,
        code if is_blocked(code) => Error::Refused {
            detail: match message {
                "" => code.to_owned(),
                message => format!("{code}: {message}"),
            },
        },
        _ => return None,
    };
    Some(error)
}

/// Never includes anything from the request, because this string reaches a log.
fn detail(status: u16, code: &str, message: &str) -> String {
    match (code.is_empty(), message.is_empty()) {
        (true, true) => format!("HTTP {status}"),
        (true, false) => format!("HTTP {status}: {message}"),
        (false, _) => format!("HTTP {status} {code}: {message}"),
    }
}

/// The codes for a picture the model would not produce, as opposed to a call that
/// failed.
///
/// The four `image_*` ones are documented on the errors page and are this
/// backend's own; the rest are shared with text and can still arrive, because the
/// prompt is classified before anything is drawn.
fn is_blocked(code: &str) -> bool {
    matches!(
        code,
        "image_safety"
            | "image_prohibited_content"
            | "image_recitation"
            | "image_other"
            | "safety"
            | "recitation"
            | "prohibited_content"
            | "spii"
            | "blocklist"
            | "content_blocked"
            | "language"
    )
}

/// Whether a quota refusal is really the billing failure.
///
/// This is the detection `docs/08-providers.md` asks for, and the reason it is a
/// string match is that Google gives it no code of its own: an image model on an
/// account with no billing has a free-tier allowance of **zero**, so the refusal
/// arrives as an ordinary quota error whose message names the free-tier metric
/// and a limit of 0.
///
/// Written to under-match, deliberately. A genuine rate limit reported as a
/// billing problem sends someone to a payment page they do not need, which is
/// worse than the reverse: a billing problem reported as a rate limit is a wait
/// that never ends, but the message still carries Google's own words.
fn no_billing_here(message: &str) -> bool {
    let message = message.to_lowercase();
    (message.contains("free") && message.contains("limit: 0"))
        || (message.contains("free_tier") && message.contains("limit: 0"))
        || mentions_billing(&message)
}

fn mentions_billing(message: &str) -> bool {
    let message = message.to_lowercase();
    message.contains("billing")
        || message.contains("credit")
        || message.contains("payment")
        || message.contains("free tier")
        || message.contains("paid tier")
}

/// A model id that does not exist, answered with the ones that do.
///
/// The same move `comfy::no_such_model` makes and for the same reason: a dead end
/// and a choice out of the same 404. These three are the registry's, which is
/// where a model released next month lands.
fn unknown_model(message: &str) -> String {
    let known: Vec<&str> =
        wobu_influence::model_refs_registry().iter().map(|model| model.model).collect();
    format!(
        "{LABEL} has no such model — {message}. The image models wobu knows about are: {}",
        known.join(", "),
    )
}

/// The wait Google itself asks for, out of a `google.rpc.RetryInfo` detail.
///
/// Written to under-match: a delay we fail to find costs the queue its own
/// backoff, where one we invent costs a hammered key. 🚩 The current Interactions
/// error page documents no such detail and asks for plain exponential backoff;
/// this reads it where it is present rather than requiring it.
fn retry_info(error: &Value) -> Option<Duration> {
    let details = error["details"].as_array()?;
    details.iter().find_map(|detail| {
        if !detail["@type"].as_str()?.ends_with("google.rpc.RetryInfo") {
            return None;
        }
        // `google.protobuf.Duration` in JSON: seconds with an optional fractional
        // part and a trailing `s`, as in "42s" or "1.500s".
        let delay = detail["retryDelay"].as_str().or_else(|| detail["retry_delay"].as_str())?;
        let seconds: f64 = delay.trim_end_matches('s').parse().ok()?;
        (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wobu_core::{AssetRole, Id};
    use wobu_influence::RefBucket;

    use crate::aspect::{AspectRatio, Resolution};
    use crate::backend::{ImageBackend, Reference};
    use crate::capability::Capabilities;
    use crate::negotiate::negotiate;

    /// The real declaration, not a hand-written one: a request built against
    /// capabilities this backend does not have is a request no negotiation could
    /// produce.
    fn caps(model: &str) -> Capabilities {
        super::super::GeminiBackend::new("k").unwrap().capabilities(model)
    }

    fn request() -> ImageRequest {
        let negotiated =
            negotiate(&[], AspectRatio::parse("16:9").unwrap(), &caps("gemini-3-pro-image"));
        ImageRequest::new(
            "gemini-3-pro-image",
            "a hooded figure in ash-glazed plate",
            42,
            &negotiated,
        )
    }

    fn reference(role: AssetRole, bucket: RefBucket, bytes: &[u8]) -> Reference {
        Reference {
            asset_id: Id::nil(),
            role,
            bucket,
            weight: 1.0,
            bytes: bytes.to_vec(),
            mime: "image/png".into(),
        }
    }

    /// A PNG header of the given size, which is all `dimensions::read` looks at.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    /// The documented response, in the shape the SDK examples read: an
    /// interaction with numbered steps, a thought step first, and the picture as
    /// a content block of the `model_output` step.
    fn response(status: &str, blocks: Vec<Value>) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "id": "v1_abc",
            "object": "interaction",
            "model": "gemini-3-pro-image",
            "status": status,
            "steps": [
                {"type": "thought", "content": [{"type": "text", "text": "The user wants…"}]},
                {"type": MODEL_OUTPUT_STEP, "content": blocks},
            ],
            "usage": {"total_input_tokens": 812, "total_output_tokens": 1120},
        }))
        .unwrap()
    }

    fn inline(bytes: &[u8]) -> Value {
        json!({
            "type": "image",
            "mime_type": "image/png",
            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
        })
    }

    #[test]
    fn the_image_config_goes_in_a_top_level_response_format_and_not_in_generation_config() {
        // The 🚩 `docs/08-providers.md` raised, pinned. Re-checked live on
        // 2026-08-01: `image_config` moved out of `generation_config` in the May
        // 2026 revision and the legacy schema was removed on 2026-06-08, so the
        // old shape is a 400 with nothing generated — and the new one is
        // `response_format` with `type: "image"`, not a nested
        // `response_format.image`.
        let body = request_body(&request(), "4K");
        assert_eq!(body["response_format"]["type"], "image");
        assert_eq!(body["response_format"]["aspect_ratio"], "16:9");
        assert_eq!(body["response_format"]["image_size"], "4K");
        assert_eq!(body["response_format"]["delivery"], "inline");
        assert!(body.get("generation_config").is_none(), "the legacy home of image_config");
        assert!(body["response_format"].get("image").is_none(), "and not nested under `image`");
        assert!(body["response_format"].get("imageConfig").is_none());
        assert_eq!(body["model"], "gemini-3-pro-image");
    }

    #[test]
    fn the_size_class_is_sent_verbatim_because_a_lowercase_k_is_a_400() {
        // Google's own sentence: "You must use an uppercase 'K'… Lowercase
        // parameters (e.g., 1k) will be rejected." Nothing here may normalise it,
        // and the only place the token is decided is `mod.rs::size_class`.
        for class in ["1K", "2K", "4K"] {
            assert_eq!(request_body(&request(), class)["response_format"]["image_size"], class);
        }
    }

    #[test]
    fn the_request_opts_out_of_server_side_storage() {
        // The API keeps the request and the response by default so a later call
        // can chain onto them. We never chain, and the payload here is somebody's
        // unpublished concept art *and* every reference photo attached to it.
        assert_eq!(request_body(&request(), "1K")["store"], false);
    }

    #[test]
    fn reference_images_ride_inline_in_one_flat_list_because_there_is_no_bucket_field() {
        // #86's question, answered from the vendor's side. Google's multi-image
        // example is one text block and then N undifferentiated image blocks —
        // no `role`, no `parts`, no name on any picture. So the buckets are a
        // budget and not a routing table, and the order is the only signal about
        // a reference that survives the request.
        let request = request().with_references(vec![
            reference(AssetRole::Costume, RefBucket::StyleRefs, b"\x89PNG-costume"),
            reference(AssetRole::Pose, RefBucket::Characters, b"\x89PNG-pose"),
        ]);
        let body = request_body(&request, "2K");
        let input = body["input"].as_array().unwrap();

        assert_eq!(input.len(), 3, "one prompt and two pictures");
        assert_eq!(input[0]["type"], "text");
        assert_eq!(input[0]["text"], "a hooded figure in ash-glazed plate");
        for block in &input[1..] {
            assert_eq!(block["type"], "image");
            assert_eq!(block["mime_type"], "image/png");
            assert!(block.get("role").is_none(), "the API has no such field to put a bucket in");
        }
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(input[1]["data"].as_str().unwrap())
                .unwrap(),
            b"\x89PNG-costume",
            "and in the order the negotiation kept them",
        );
    }

    #[test]
    fn a_prompt_with_no_references_is_still_a_list_and_not_a_bare_string() {
        // `input` accepts both, and one shape for both cases is one shape to get
        // wrong. The alternative is a request that changes structure the first
        // time somebody attaches a picture.
        let body = request_body(&request(), "1K");
        assert!(body["input"].is_array());
        assert_eq!(body["input"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn the_picture_is_decoded_out_of_the_model_output_step_and_not_out_of_the_thought() {
        // Steps are numbered and a `thought` step arrives first, carrying its own
        // content. The text adapter tracks the same distinction; here, reading
        // the wrong step is a generation that reports no image while the picture
        // sits two lines further down the response.
        let expected = png(3840, 2160);
        let returned = image(&response("completed", vec![inline(&expected)])).unwrap();
        assert_eq!(returned.bytes, expected);
        assert_eq!(returned.mime, "image/png");

        // And a response with text alongside the picture still finds the picture.
        let mixed = response("completed", vec![
            json!({"type": "text", "text": "Here is the image you asked for."}),
            inline(&expected),
        ]);
        assert_eq!(image(&mixed).unwrap().bytes, expected);
    }

    #[test]
    fn a_body_that_wraps_the_interaction_is_read_the_same_as_one_that_is_it() {
        // 🚩 Every published example is SDK code reading `interaction.steps`, so
        // the top-level envelope of the raw body is not confirmed. Accepting both
        // costs one branch; guessing wrong costs every generation.
        let inner: Value =
            serde_json::from_slice(&response("completed", vec![inline(&png(1, 1))])).unwrap();
        let wrapped = serde_json::to_vec(&json!({ "interaction": inner })).unwrap();
        assert_eq!(image(&wrapped).unwrap().bytes, png(1, 1));
    }

    #[test]
    fn an_image_in_hand_outranks_whatever_the_status_field_says() {
        // A response carrying pixels has been generated and billed. Refusing it
        // because the interaction called itself `incomplete` charges the user for
        // a picture and then throws it away.
        for status in ["completed", "incomplete", "failed", "budget_exceeded"] {
            let body = response(status, vec![inline(&png(64, 64))]);
            assert!(image(&body).is_ok(), "{status}");
        }
    }

    #[test]
    fn a_completed_response_with_no_picture_is_told_apart_from_a_refusal() {
        // `error.rs` keeps these apart because they send the user to two
        // different places: a stated refusal is a prompt to edit, and a silent
        // empty result is a bug report.
        let empty = response("completed", vec![json!({"type": "text", "text": "I can't."})]);
        assert!(matches!(image(&empty), Err(Error::NoImage)));

        let cancelled = response("cancelled", vec![]);
        assert!(matches!(image(&cancelled), Err(Error::Cancelled)), "and is never retried");

        let failed = response("failed", vec![]);
        assert_eq!(image(&failed).unwrap_err().code(), "provider.unavailable");
    }

    #[test]
    fn a_refusal_reported_inside_a_200_is_still_a_refusal() {
        // Google decides some blocks after the status line has already promised
        // success. An adapter that only maps status codes calls this an empty
        // response and tells the user to file a bug about a prompt they could
        // edit.
        let body = serde_json::to_vec(&json!({
            "status": "failed", "steps": [],
            "error": {"code": "image_safety", "message": "blocked by a safety filter"},
        }))
        .unwrap();
        let error = image(&body).unwrap_err();
        assert_eq!(error.code(), "provider.bad_response");
        assert!(error.to_string().contains("image_safety"), "{error}");
        assert!(error.is_retryable(), "Google's blocking is not deterministic");
    }

    #[test]
    fn bytes_that_are_not_base64_or_not_json_are_reported_rather_than_written_to_disk() {
        // What a proxy's error page arrives as, and what a truncated response
        // arrives as. Either one written into `assets/` is a content-addressed
        // file that no thumbnail can open.
        assert!(matches!(image(b"<html>502</html>"), Err(Error::NotAnImage { .. })));

        let broken = response("completed", vec![json!({
            "type": "image", "mime_type": "image/png", "data": "not base64!!",
        })]);
        let error = image(&broken).unwrap_err();
        assert!(error.to_string().contains("image/png"), "{error}");
        assert_eq!(error.code(), "provider.bad_response");
    }

    #[test]
    fn a_link_where_the_image_should_be_says_so_rather_than_reporting_nothing() {
        // `delivery` is an enum of `inline` and `uri` and the request asks for
        // the first. A `uri` coming back means it was not honoured — and this
        // crate has no fetcher, so reporting "no image" would hide the one fact
        // that explains it.
        let body = response("completed", vec![json!({
            "type": "image", "mime_type": "image/png",
            "uri": "https://generativelanguage.googleapis.com/…",
        })]);
        let error = image(&body).unwrap_err();
        assert!(error.to_string().contains("link to the image"), "{error}");
    }

    #[test]
    fn a_free_tier_quota_of_zero_is_the_billing_failure_and_not_a_rate_limit() {
        // The support ticket `docs/08-providers.md` predicts, in the form it
        // actually arrives in. Image generation has no free tier, so the
        // allowance on an account with no billing is zero — and a quota of zero
        // comes back as an ordinary 429. Reported as a rate limit it is a wait
        // that never ends, and the queue retries it until the backoff gives up.
        let body = json!({"error": {"code": "quota_exceeded", "message":
            "You exceeded your current quota. metric: \
             generativelanguage.googleapis.com/generate_content_free_tier_requests, limit: 0"}});
        let error = error_for_status(429, body.to_string().as_bytes(), None);
        assert_eq!(error.code(), "provider.billing_required");
        assert!(!error.is_retryable(), "waiting out a limit of zero is waiting forever");
        assert!(error.to_string().contains("aistudio.google.com"), "{error}");

        // A real rate limit on a paying account is still a wait, and still
        // carries whatever number Google supplied rather than one we invented.
        let ordinary = json!({"error": {"code": "rate_limit_exceeded",
            "message": "Requests per minute exceeded for this model"}});
        assert!(matches!(
            error_for_status(429, ordinary.to_string().as_bytes(), Some(Duration::from_secs(7))),
            Error::RateLimited { retry_after: Some(wait), .. } if wait == Duration::from_secs(7)
        ));
    }

    #[test]
    fn no_rate_limit_number_is_ever_invented_here() {
        // `docs/08-providers.md`: "Concrete free-tier RPM/TPM/RPD numbers are
        // deliberately unpublished and vary; do not hardcode them." Google's
        // rate-limit page says the same — limits depend on tier and account and
        // are only visible in AI Studio. So the only wait this crate knows is one
        // Google just sent, and `None` means the queue picks.
        let bare = json!({"error": {"code": "rate_limit_exceeded", "message": "slow down"}});
        assert!(matches!(
            error_for_status(429, bare.to_string().as_bytes(), None),
            Error::RateLimited { retry_after: None, .. }
        ));

        // Out of the body when the header is absent, which is where the older
        // envelope puts it.
        let with_detail = json!({"error": {"code": 429, "status": "RESOURCE_EXHAUSTED",
            "message": "Resource has been exhausted", "details": [
                {"@type": "type.googleapis.com/google.rpc.QuotaFailure", "violations": []},
                {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "42s"}]}});
        assert!(matches!(
            error_for_status(429, with_detail.to_string().as_bytes(), None),
            Error::RateLimited { retry_after: Some(wait), .. } if wait == Duration::from_secs(42)
        ));
    }

    #[test]
    fn every_documented_error_code_lands_on_the_variant_the_ui_can_answer() {
        // The regression this guards is a user staring at "the backend could not
        // be reached" over a key they could paste in Settings, or over a card
        // they could add. The codes are the table at
        // ai.google.dev/gemini-api/docs/api-errors, read 2026-08-01.
        let cases: &[(u16, &str, &str, &str)] = &[
            (401, "authentication", "API key not valid", "provider.bad_key"),
            (403, "permission_denied", "not authorized for this model", "provider.bad_key"),
            (
                403,
                "permission_denied",
                "billing is not enabled on this project",
                "provider.billing_required",
            ),
            (404, "model_not_found", "gemini-9.9-flash-image", "provider.unavailable"),
            (429, "rate_limit_exceeded", "requests per minute", "provider.rate_limited"),
            (429, "quota_exceeded", "daily quota", "provider.rate_limited"),
            (400, "invalid_request", "unknown field `delivery`", "internal"),
            (400, "parameter_unknown", "unknown parameter: seed", "internal"),
            (499, "cancelled", "client closed request", "cancelled"),
            (500, "api_error", "internal error", "provider.unavailable"),
            (503, "service_unavailable", "the model is overloaded", "provider.unavailable"),
            (400, "image_safety", "blocked", "provider.bad_response"),
            (400, "image_prohibited_content", "blocked", "provider.bad_response"),
            (400, "image_recitation", "blocked", "provider.bad_response"),
            (400, "image_other", "blocked", "provider.bad_response"),
            (400, "no_image", "the model was unable to generate an image", "provider.bad_response"),
        ];
        for (status, code, message, expected) in cases {
            let body = json!({"error": {"code": code, "message": message}});
            let error = error_for_status(*status, body.to_string().as_bytes(), None);
            assert_eq!(error.code(), *expected, "{status} {code}: {message}");
        }
    }

    #[test]
    fn the_older_google_rpc_envelope_maps_the_same_way_as_the_new_one() {
        // A 429 raised by Google's edge before the request reaches the API
        // arrives as `{"code": 429, "status": "RESOURCE_EXHAUSTED"}` — an integer
        // where the new shape has a name. An adapter that only reads the new
        // shape calls the commonest failure on this backend "unavailable" and
        // sends the user to check their network.
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
            (503, "UNAVAILABLE", "overloaded", "provider.unavailable"),
        ];
        for (status, name, message, expected) in cases {
            let body = json!({"error": {"code": status, "status": name, "message": message}});
            let error = error_for_status(*status, body.to_string().as_bytes(), None);
            assert_eq!(error.code(), *expected, "{status} {name}: {message}");
        }
    }

    #[test]
    fn a_status_with_an_unreadable_body_still_reaches_the_right_variant() {
        // Google's edge answers some refusals before the request reaches the API,
        // and the body is HTML. Depending on the JSON shape would make that a
        // mystery, on a call that has already cost money.
        assert!(matches!(error_for_status(401, b"<html>...", None), Error::BadKey { .. }));
        assert!(matches!(error_for_status(429, b"", None), Error::RateLimited {
            retry_after: None,
            ..
        }));
        assert!(matches!(error_for_status(400, b"", None), Error::Unsupported { .. }));
        assert!(matches!(error_for_status(502, b"bad gateway", None), Error::Unavailable { .. }));

        // And whoever reads the log needs to know whether they are looking at a
        // 502 to wait out or a 400 to go and fix.
        let message = error_for_status(502, b"<html>bad gateway</html>", None).to_string();
        assert!(message.contains("502"), "{message}");
        assert!(message.contains("bad gateway"), "{message}");
    }

    #[test]
    fn a_model_id_that_does_not_exist_is_answered_with_the_ones_that_do() {
        // A dead end and a choice, out of the same 404 — the move
        // `comfy::no_such_model` makes for a missing checkpoint. The list is the
        // registry's, so a model released next month is a row in that table
        // rather than an edit here.
        let body = json!({"error": {"code": "model_not_found", "message": "gemini-2-image"}});
        let message = error_for_status(404, body.to_string().as_bytes(), None).to_string();
        assert!(message.contains("gemini-2-image"), "{message}");
        assert!(message.contains("gemini-3-pro-image"), "{message}");
        assert!(message.contains("gemini-3.1-flash-lite-image"), "{message}");
    }

    #[test]
    fn no_error_message_carries_the_key() {
        // `redact::scrub` at the command boundary is the real guard, but an error
        // that carries a key at all has already put it in a log line — and the
        // body of a 401 from Google quotes the request back.
        let body = json!({"error": {"code": "authentication",
            "message": "API key not valid: AIzaSyD-secret-key-material"}});
        for status in [400, 401, 403, 429, 500] {
            let message = error_for_status(status, body.to_string().as_bytes(), None).to_string();
            assert!(!message.contains("AIzaSyD"), "{status}: {message}");
        }
    }

    #[test]
    fn the_shape_sent_is_sized_by_the_negotiation_and_never_by_this_file() {
        // `request_body` takes the size class rather than computing one, so there
        // is exactly one place that decides what is asked for — and it is the one
        // that checked the model's ceiling first. Two deciders is how a 4K
        // request reaches a model that tops out at 1K.
        let square = {
            let negotiated = negotiate(
                &[],
                AspectRatio::parse("1:1").unwrap(),
                &caps("gemini-3.1-flash-lite-image"),
            );
            assert_eq!(negotiated.resolution(), Resolution::new(1024, 1024));
            ImageRequest::new("gemini-3.1-flash-lite-image", "p", 0, &negotiated)
        };
        assert_eq!(request_body(&square, "1K")["response_format"]["aspect_ratio"], "1:1");
        assert!(
            request_body(&square, "1K")["response_format"].get("width").is_none(),
            "the API takes a size class, not pixels",
        );
    }
}
