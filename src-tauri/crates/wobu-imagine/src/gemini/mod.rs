//! [`ImageBackend`] over Google's Gemini image models.
//!
//! The remote half of the pair. Written deliberately as a mirror of `comfy/` —
//! pure wire functions in `wire.rs` driven from recorded payloads, a read
//! cancelled by racing rather than polling, and a failure diagnosed into a
//! sentence rather than reported as a status code — so the two can be read side
//! by side and every difference is a difference between the backends.
//!
//! It is also the second Gemini adapter in the workspace, and the first one
//! already answered the questions that are about the *vendor* rather than about
//! images: the `x-goog-api-key` header over the `?key=` query parameter, the
//! `Api-Revision` pin, `/v1beta` over `/v1beta2`, `store: false`, and the error
//! table. Those findings are reused rather than re-derived, and `wire.rs` says
//! at each one where it came from. Provider-neutral HTTP mechanics are shared
//! through `wobu_llm::transport`; the wire decoder and error mapping are not.
//! That boundary is deliberate: `wobu_llm::gemini::wire` maps onto
//! `wobu_llm::Error`, which has `Truncated`, `NotJson` and `ContextTooLong` and
//! none of `Refused`, `NoImage` or `NotAnImage`. Sharing it would mean
//! translating one lossy provider error enum into another. `error.rs` already
//! keeps a hand-copy of the UI's code table for the same reason and says so.
//!
//! Four things this adapter does that a thinner one would not:
//!
//! 1. **It tells "no billing on this account" apart from "this key is wrong".**
//!    They arrive as neighbouring status codes and send the user to two
//!    completely different websites. This is the failure `docs/08-providers.md`
//!    predicts will generate support tickets, because Gemini *text* is free and
//!    every Gemini image model is billing-only: the same key that just enhanced
//!    a species will refuse to draw it. See `no_free_tier`.
//! 2. **It reads the dimensions off the returned image.** The request cannot
//!    even carry pixels — the API takes a size *class* — and there are credible
//!    reports of the size and aspect being ignored outright. What the picture
//!    says it is, is what `Asset.width`/`height` record.
//! 3. **It never hardcodes a rate limit.** Free-tier RPM/TPM/RPD are unpublished
//!    and vary by country and account; the only trustworthy number is the one
//!    Google puts in the body of the 429 it just sent.
//! 4. **It declares the watermark.** Every image out of these models carries
//!    SynthID and no request field turns it off, so [`GeneratedImage::watermark`]
//!    carries it out to the card and the export rather than a log line.
//!
//! ## What is not here
//!
//! **Previews, LoRAs, structure references and a negative prompt.** The API has
//! no field for any of them, so [`capabilities`](ImageBackend::capabilities)
//! declares them false and [`negotiate`](crate::negotiate) reports each withheld
//! fragment on the card it came from. **A seed**, too: Google documents none for
//! the image models, which is why `Capabilities` has no flag for it and why
//! [`GeneratedImage::seed`] comes back `None` — a turnaround's eight views are
//! eight calls that cannot be pinned to each other, and saying so is better than
//! writing the seed we asked for into a record that will not reproduce.

pub(crate) mod wire;

use std::fmt;
use std::sync::{Arc as Shared, LazyLock};

use async_trait::async_trait;
use wobu_influence::{ImageBudget, image_budget};

use crate::Cancel;
use crate::aspect::{AspectRatio, Resolution};
use crate::backend::{
    GeneratedImage, ImageBackend, ImageOutcome, ImageRequest, ImageUsage, ProgressSink, Watermark,
};
use crate::capability::{Capabilities, ReferenceMechanisms};
use crate::dimensions;
use crate::error::{Error, Result};
use wobu_llm::transport;

/// The `backend` in `project.json`, the `backend` field of every `Generation`,
/// and the `wobu/gemini` entry in the OS keychain.
///
/// The same string `wobu_llm::gemini::ID` uses, on purpose and not by accident:
/// Google issues one API key per project and it is the same key for text and for
/// images. Two ids here would mean two keychain entries, and a user who pasted
/// their key into Settings once would be told Generate has no key.
pub const ID: &str = "gemini";

/// The name a person sees, including inside the errors built here.
pub const LABEL: &str = "Gemini";

/// Used when a project names this backend but no model.
///
/// The middle model rather than the cheapest: `gemini-3.1-flash-lite-image` is
/// 1K only, which for a character sheet headed into a pipeline is a thumbnail,
/// and it counts all fourteen of its references in one undivided bucket so a
/// style reference evicts a costume reference. Verified against the table in
/// `docs/08-providers.md`. Nothing checks this against a list — `project.json`
/// carries whatever id it likes — which is the only way a model released next
/// month works without a release of ours.
pub const DEFAULT_MODEL: &str = "gemini-3.1-flash-image";

/// Where the models live, and the *root* rather than one endpoint — this adapter
/// calls two of them.
///
/// **`/v1beta`, not `/v1beta2`**, and the whole argument is in
/// `wobu_llm::gemini`: on 2026-07-31 four of Google's pages wrote
/// `/v1beta/interactions` and only `migrate-to-interactions` wrote `/v1beta2`,
/// and that page is stale in a second way as well. 🚩 Still unconfirmed by a live
/// call, here as there. If Generate comes back 404 with nothing billed, this
/// constant is the first thing to change — and if it is wrong here it is wrong
/// for Enhance too, so change both.
const API_ROOT: &str = "https://generativelanguage.googleapis.com/v1beta";

/// The request and response schema this adapter was written against.
///
/// Sent, and **currently a no-op** — which is a correction to what
/// `wobu_llm::gemini` says and to `docs/08-providers.md`'s "pin the revision"
/// paragraph, both written on 2026-07-31. The breaking-changes page's own
/// timeline, re-read on 2026-08-01, says of 2026-06-08: "Legacy schema removed
/// for Interactions API. `Api-Revision` header ignored." The image-generation
/// guide and the API reference do not mention the header at all.
///
/// Kept anyway, for two reasons and neither of them optimism. It documents in the
/// request itself which shape this file was written against, which is the fact a
/// 400 six months from now needs. And it costs one header on a call that is
/// carrying a base64 image, against the alternative of removing it and having to
/// re-derive why it was ever there.
///
/// <https://ai.google.dev/gemini-api/docs/interactions-breaking-changes-may-2026>
const API_REVISION: &str = "2026-05-20";

/// One process-long transport for checks and paid jobs alike.
///
/// `reqwest::Client` owns the connection and TLS-session pools. Backends remain
/// cheap per-key values, while cloning this `Arc` ensures a status check does
/// not build a fresh TLS stack and a later generation can reuse its connection.
static CLIENT: LazyLock<std::result::Result<Shared<reqwest::Client>, String>> =
    LazyLock::new(|| transport::client().map(Shared::new));

fn shared_client() -> Result<Shared<reqwest::Client>> {
    CLIENT
        .as_ref()
        .map(Shared::clone)
        .map_err(|detail| Error::Unavailable { detail: detail.clone() })
}

/// Where a user turns billing on. Carried in the message rather than left to the
/// UI: [`Error::BillingRequired`] reaches the webview as its `Display`, and a
/// dead end with no link is what the support ticket is about.
const BILLING_URL: &str = "https://aistudio.google.com/apikey";

/// A Gemini key and the client that uses it.
///
/// Constructing one does no IO and cannot fail on a machine with no network: the
/// Inspector draws a backend dropdown before anything has been checked, exactly
/// as it does for ComfyUI.
pub struct GeminiBackend {
    api_key: String,
    base_url: String,
    client: Shared<reqwest::Client>,
}

impl fmt::Debug for GeminiBackend {
    /// Hand-written because the derived one prints the key, and a
    /// `{backend:?}` in a log line somewhere later is not a thing anyone would
    /// think to review.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeminiBackend")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl GeminiBackend {
    /// The key comes from the keychain (`docs/08-providers.md`); this crate
    /// neither resolves nor stores it.
    pub fn new(api_key: impl Into<String>) -> Result<GeminiBackend> {
        let client = shared_client()?;
        Ok(GeminiBackend { api_key: api_key.into(), base_url: API_ROOT.into(), client })
    }

    /// Point at something other than the real API — a local server standing in
    /// for it, or a gateway an organisation puts in front. Kept because the send
    /// path is otherwise the one part of this adapter nothing can reach.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> GeminiBackend {
        self.base_url = base_url.into();
        self
    }

    /// Ask whether this key can generate images, before the user presses
    /// Generate.
    ///
    /// `docs/08-providers.md` asks for this by name: "ideally we probe capability
    /// at key-entry time and show it in Settings before the user ever hits
    /// Generate", because the failure it is guarding against is a key that
    /// enhances perfectly and then refuses to draw.
    ///
    /// **It reads the model, and it does not generate one.** A probe that
    /// actually made an image would answer the billing question outright and
    /// charge a few cents to do it, every time somebody pasted a key — so this
    /// asks the models endpoint instead and reports what that can and cannot
    /// prove. [`KeyCheck::Usable`] is therefore "the key is real and this model
    /// exists for it", not "the account can pay": see the note on that variant.
    pub async fn check_key(&self, model: &str, cancel: &Cancel) -> KeyCheck {
        // The legacy model-metadata endpoint, which is the cheapest thing behind
        // this key that answers differently for a bad one. 🚩 Unconfirmed live,
        // like everything else here — but the failure mode is benign: a 404
        // because Google retired the path lands on `Unknown`, and Settings then
        // says it could not check rather than that the key is bad.
        let path = format!("models/{model}");
        match self.get(&path, cancel).await {
            Ok(_) => KeyCheck::Usable,
            Err(Error::BadKey { .. }) => KeyCheck::BadKey,
            Err(Error::BillingRequired { detail, .. }) => KeyCheck::BillingRequired { detail },
            Err(Error::Cancelled) => KeyCheck::Unknown { detail: "the check was cancelled".into() },
            // A rate limit, a 5xx, no network, a retired endpoint. None of these
            // is a fact about the key, and reporting one as a bad key would send
            // the user to regenerate a key that is fine.
            Err(error) => KeyCheck::Unknown { detail: error.to_string() },
        }
    }

    /// One generation, from a request to bytes we are willing to keep.
    ///
    /// Split out of [`generate`](ImageBackend::generate) so the whole path can be
    /// written with `?`. The wrapper is what decides the usage figure, which is
    /// the one thing that must be decided in exactly one place.
    async fn draw(
        &self,
        request: &ImageRequest,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> Result<GeneratedImage> {
        // A job cancelled while it was queued must not open a connection.
        // Everything before the request goes out is unbilled, and everything
        // after it is not.
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }

        // The trait's first contract point, checked rather than assumed: send
        // what the request says or fail. `negotiate` compiles no `never:`
        // fragments for a backend whose `negative_prompt` is false, so a
        // negative arriving here means the negotiation and this adapter disagree
        // — and the alternative is dropping user-authored canon in silence,
        // which is the failure the whole negotiation exists to prevent.
        if !request.negative.is_empty() {
            return Err(Error::Unsupported {
                detail: format!(
                    "{LABEL} has no negative-prompt field, and this request carries one: {:?}",
                    request.negative,
                ),
            });
        }
        let size = size_class(request, &self.capabilities(&request.model))?;

        // One request, one response, and nothing in between: there is no progress
        // to report and `streaming_preview` is false, so `ProgressSink::preview`
        // is never called. This one step is what keeps the status bar from
        // showing an indeterminate spinner for the whole of a paid call.
        progress.step(0, 1, Some("waiting on Gemini"));

        let response =
            self.post("interactions", &wire::request_body(request, size), cancel).await?;
        accept(wire::image(&response)?)
    }

    async fn get(&self, path: &str, cancel: &Cancel) -> Result<Vec<u8>> {
        self.send(self.client.get(format!("{}/{path}", self.base_url)), cancel).await
    }

    async fn post(&self, path: &str, body: &serde_json::Value, cancel: &Cancel) -> Result<Vec<u8>> {
        let request = self
            .client
            .post(format!("{}/{path}", self.base_url))
            .header("content-type", "application/json")
            .body(serde_json::to_vec(body).unwrap_or_default());
        self.send(request, cancel).await
    }

    /// Send, and race every wait against the cancellation.
    ///
    /// Racing rather than polling, for the reason `comfy/socket.rs` gives at
    /// length and which is more expensive here: a 4K image is tens of seconds
    /// during which the socket says nothing at all, so a loop that checked a flag
    /// between reads would leave a stopped job parked until Google spoke. Losing
    /// the race returns immediately, which drops the response and closes the
    /// connection.
    async fn send(&self, request: reqwest::RequestBuilder, cancel: &Cancel) -> Result<Vec<u8>> {
        let request = request
            // A header rather than the `?key=` query parameter the quickstarts
            // use. Both authenticate; only one of them keeps the key out of proxy
            // logs and out of anything that prints a URL.
            .header("x-goog-api-key", &self.api_key)
            .header("api-revision", API_REVISION);
        let response = match transport::send(request, cancel).await {
            Err(transport::Failure::Cancelled) => return Err(Error::Cancelled),
            // DNS, TLS, a refused connection, the connect timeout. Nothing
            // reached a model, so nothing was charged.
            Err(transport::Failure::Unavailable(error)) => return Err(unreachable(&error)),
            Ok(response) => response,
        };

        let status = response.status().as_u16();
        // Read before the body is consumed, and only the header. Google's own
        // answer to "how long should I wait" is usually in the body instead as a
        // `RetryInfo` detail, which `wire::error_for_status` reads when this is
        // absent — and either beats a backoff we would invent.
        let retry_after = transport::retry_after(&response);
        let body = match transport::bytes_or_empty(response, cancel).await {
            Err(transport::Failure::Cancelled) => return Err(Error::Cancelled),
            // `bytes_or_empty` cannot produce this: the response already exists
            // and body read failures deliberately become an empty body so the
            // status remains mappable.
            Err(transport::Failure::Unavailable(error)) => return Err(unreachable(&error)),
            Ok(body) => body,
        };
        match status {
            200..=299 => Ok(body),
            _ => Err(wire::error_for_status(status, &body, retry_after)),
        }
    }
}

#[async_trait]
impl ImageBackend for GeminiBackend {
    fn id(&self) -> &'static str {
        ID
    }

    fn label(&self) -> &'static str {
        LABEL
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    /// What this model can do, from the tables in `docs/08-providers.md`.
    ///
    /// Nothing is probed, unlike ComfyUI: there is one Gemini and it is the same
    /// Gemini for everybody, so the answers are documented rather than
    /// discovered. What that costs is that a model id this adapter has never
    /// heard of has to be answered anyway — the Inspector renders a dropdown from
    /// this and `project.json` may name a model Google has since removed — and
    /// the answer must be the most conservative *registered* one. Handing an
    /// unknown remote model a 4K ceiling and an unlimited reference budget builds
    /// a request the provider rejects after payment.
    fn capabilities(&self, model: &str) -> Capabilities {
        Capabilities {
            max_resolution: ceiling(model),
            // The intersection both of Google's image pages agree on, which is
            // exactly `AspectRatio::ALL` — that list is written from this table.
            //
            // Declared even though `docs/08-providers.md` reports the parameter
            // being *ignored* on some calls, because the two failures are not the
            // same: an empty list would mean "this model takes no aspect
            // parameter", and then a `21:9` matte would be sent unshaped and come
            // back square with nothing on screen explaining it. Declaring the
            // list is what makes the returned dimensions checkable.
            aspect_ratios: AspectRatio::ALL.to_vec(),
            image_refs: budget(model),
            // Inline image blocks are ordinary image prompts. They have no
            // second mechanism cap — `image_refs` is the documented provider
            // quota — and there is no structure path of any kind.
            reference_mechanisms: ReferenceMechanisms::image_prompt(),
            loras: false,
            // Imagen had `negativePrompt` and Imagen is deprecated; the image
            // models have no field for one. Every `never:` fragment is withheld
            // and reported, which is the whole reason this flag exists.
            negative_prompt: false,
            // True on every model here, and the asymmetry with ComfyUI is the
            // point: the Generate button shows an estimate for this backend and
            // nothing at all for the local one.
            requires_billing: true,
            // One inline image and nothing before it.
            streaming_preview: false,
        }
    }

    async fn generate(
        &self,
        request: &ImageRequest,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> ImageOutcome {
        match self.draw(request, progress, cancel).await {
            Ok(image) => ImageOutcome::new(ImageUsage::billed(1), Ok(image)),
            // Everything that failed before the request left the machine, and
            // every refusal that generated nothing. `error.rs` keeps `Refused`
            // retryable on the grounds that Google's blocking is not
            // deterministic, and #49 holds a retry when the last attempt was
            // billed — so claiming a charge for a block that produced no pixels
            // would take away the retry the classification was chosen for.
            Err(error) => ImageOutcome::new(billed_by(&error), Err(error)),
        }
    }
}

/// Decoded bytes, checked and measured, or a reason not to keep them.
///
/// Split out of [`GeminiBackend::draw`] so that the one thing
/// `docs/08-providers.md` is emphatic about — that the size is read off the
/// picture rather than off the request — can be checked without a network. It is
/// the last thing between a provider and `assets/`, and it takes no request, so
/// there is nothing here to echo even by accident.
fn accept(returned: wire::Returned) -> Result<GeneratedImage> {
    let bytes = returned.bytes;
    let Some((width, height)) = dimensions::read(&bytes) else {
        return Err(Error::NotAnImage {
            detail: format!(
                "{LABEL} returned {} bytes labelled {}, and they are not a PNG, JPEG or WebP",
                bytes.len(),
                returned.mime,
            ),
        });
    };
    Ok(GeneratedImage {
        // The container's own answer, not the one the response claimed and not
        // the one we asked for: `docs/08-providers.md` reports `imageSize` and
        // `aspect_ratio` being silently ignored, and the request could not have
        // carried pixels anyway — it names a size class.
        mime: dimensions::mime(&bytes).to_owned(),
        bytes,
        width,
        height,
        // Google documents no seed parameter for the image models, so there is
        // nothing to report and nothing to invent. `backend.rs` is explicit that
        // echoing the requested one would write a number into `Generation.seed`
        // that reproduces nothing.
        seed: None,
        // Unconditional: every image out of every one of these models carries one
        // and no request field turns it off (`docs/08-providers.md`). Read out of
        // a response field instead, an API that stopped sending that field would
        // quietly take the badge off images that still have the mark.
        watermark: Some(Watermark::SynthId),
    })
}

/// What a failure is known to have cost.
///
/// One image or none, and the line is whether the provider is understood to have
/// generated pixels. Two of these are judgement calls and both err towards
/// *having* been charged, because `ImageUsage` is what #55's ceiling reads and a
/// ceiling that undercounts stops being a ceiling:
///
/// - **A cancellation after the request went out.** `backend.rs` states the rule
///   in as many words — "a remote render that is abandoned rather than stopped is
///   billed in full" — and there is no call to make that stops Gemini generating.
///   Dropping the response only stops us waiting for it.
/// - **A response we could not read.** Bytes that are not an image, or a
///   response with no image in it at all, both arrived after a 200. Something was
///   generated to produce them.
fn billed_by(error: &Error) -> ImageUsage {
    match error {
        Error::NotAnImage { .. } | Error::NoImage => ImageUsage::billed(1),
        // The one cancellation that is genuinely free is the one that beat the
        // request out of the door, and `draw` returns that before it builds a
        // body — but it is indistinguishable from the expensive one by the time
        // it is an `Error`, so this is the safe reading of both.
        Error::Cancelled => ImageUsage::billed(1),
        // A refusal generated nothing, and everything else never reached a model.
        _ => ImageUsage::free(),
    }
}

/// The largest image this model will produce.
///
/// Three ceilings and not one, which is the case `capability.rs` says forces
/// capabilities to be per model: `gemini-3.1-flash-lite-image` is 1K only, and a
/// single answer for "Gemini" would have to be that one — capping the model the
/// user is paying 4K prices for at a quarter of the width.
///
/// These are squares because the API takes a size *class* rather than pixels, and
/// `AspectRatio::fit` is what turns the class into the dimensions of a shape. The
/// result is approximate on purpose: 4K at `16:9` computes here as 4096×2304 and
/// Google will return whatever it returns, which is why the answer is read back
/// off the image rather than trusted.
fn ceiling(model: &str) -> Resolution {
    match model {
        "gemini-3.1-flash-lite-image" => Resolution::new(1024, 1024),
        "gemini-3.1-flash-image" | "gemini-3-pro-image" => Resolution::new(4096, 4096),
        // A model id nobody here has seen. 1K is the smallest documented class,
        // so it is the one request every one of these models is known to accept.
        _ => Resolution::new(1024, 1024),
    }
}

/// The reference budget, read out of #44's registry and never restated.
///
/// A model the registry has never heard of gets `gemini-3-pro-image`'s, which is
/// the most conservative *registered* budget rather than the largest: six objects
/// where the lite model takes fourteen. `capability.rs` is explicit that it must
/// never be [`ImageBudget::unlimited`] — building a fourteen-image request for a
/// model that takes six lets the provider be the one to point it out, after
/// payment.
fn budget(model: &str) -> ImageBudget {
    image_budget(model)
        .unwrap_or_else(|| image_budget("gemini-3-pro-image").expect("the registry's own model id"))
}

/// The documented size class a request's pixel dimensions round to.
///
/// The API takes a size *class* and not pixels, so this is the rounding
/// `backend.rs` describes: "Gemini rounds to its nearest documented size class
/// and then reads back what actually came out". **Uppercase K**, which
/// `docs/08-providers.md` calls out and Google states outright — "lowercase
/// parameters (e.g., 1k) will be rejected" — so the token is a literal here and
/// nothing downstream normalises it.
///
/// Rounds *up* to the class containing the long side, so a `21:9` matte fitted to
/// 4096×1755 asks for 4K rather than for the 2K its short side would suggest.
///
/// 🚩 There is a fourth class below these — `gemini-3.1-flash-image` adds a half-K
/// one — and it is not emitted, because the two pages that mention it disagree on
/// the literal: the API reference's enum value is `512` and the guide's prose
/// writes `512px`. It is also unreachable, since the smallest ceiling this
/// adapter declares is 1024 and `AspectRatio::fit` never shortens the long side.
/// Adding it means picking one of the two spellings, which is a guess, on a call
/// that costs money.
///
/// [`Error::Unsupported`] when the answer is larger than the model's ceiling,
/// which is unreachable if `negotiate` has run — and is therefore reported as our
/// bug rather than quietly downgraded, because a quiet downgrade here is the user
/// paying 4K prices for a 1K picture.
fn size_class(request: &ImageRequest, caps: &Capabilities) -> Result<&'static str> {
    if !request.resolution.fits_in(caps.max_resolution) {
        return Err(Error::Unsupported {
            detail: format!(
                "{} was negotiated at {} and tops out at {}",
                request.model, request.resolution, caps.max_resolution,
            ),
        });
    }
    Ok(match request.resolution.width.max(request.resolution.height) {
        0..=1024 => "1K",
        1025..=2048 => "2K",
        _ => "4K",
    })
}

/// **The failure that generates support tickets.**
///
/// `docs/08-providers.md` gives it a heading of its own: Gemini image generation
/// has no free tier, `gemini-3.6-flash` text does, and so the sequence that
/// happens to everybody is paste a key, enhance a species successfully, press
/// Generate, and get an error. Reported as a raw 403 or 429 the reasonable
/// conclusion is that wobu is broken.
///
/// The sentence has to do two things a status code cannot. It has to name the
/// fix, because [`Error::BillingRequired`] reaches the webview as its `Display`
/// and a message with no link is a dead end. And it has to be impossible to
/// mistake for [`Error::BadKey`] — "Gemini rejected the API key" — because the
/// two arrive as neighbouring status codes and send the user to two completely
/// different places: one regenerates a key that was fine, the other adds a card.
fn no_free_tier() -> Error {
    Error::BillingRequired {
        backend: LABEL,
        detail: format!(
            "the key itself is fine — Gemini's text models have a free tier and its image models \
             do not, so the same key that enhances will refuse to draw. Enable billing at \
             {BILLING_URL} and try again"
        ),
    }
}

/// Nothing answered, or answered too slowly.
fn unreachable(error: &reqwest::Error) -> Error {
    Error::Unavailable {
        detail: if error.is_timeout() {
            format!(
                "{LABEL} did not answer within {} seconds — check the network and try again",
                transport::CONNECT_TIMEOUT.as_secs(),
            )
        } else if error.is_connect() {
            format!("could not reach {LABEL}: {error}. Check the network and any proxy")
        } else {
            format!("could not reach {LABEL}: {error}")
        },
    }
}

/// What Settings can say about a pasted key before anything has been generated.
///
/// Not a `Result`: "this key cannot generate images" is an answer, not a failure
/// to find one out, and a Settings row with an error type in it would have to
/// invent a sentence for the `Err` arm anyway. The same shape and the same
/// argument as `comfy::Health`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCheck {
    /// The key is real and this model exists for it.
    ///
    /// **Not a promise that Generate will work.** The only call that proves an
    /// account can pay is one that charges it, and running that every time
    /// somebody pastes a key would bill them a few cents for typing. So this is
    /// the strongest claim a free probe can make, and [`KeyCheck::message`] says
    /// so rather than letting Settings imply more.
    Usable,
    /// The key is wrong, revoked, or from a different project. Nothing about
    /// billing.
    BadKey,
    /// The account cannot pay, which the probe found out early — the good case.
    /// Carries `no_free_tier`'s sentence, so Settings and the failed Generate
    /// say the same thing.
    BillingRequired { detail: String },
    /// Offline, rate limited, a 5xx, or an endpoint Google has retired. None of
    /// these is a fact about the key, and reporting one as a bad key sends the
    /// user to replace something that works.
    Unknown { detail: String },
}

impl KeyCheck {
    /// Whether Settings should show this key as set up.
    pub fn is_usable(&self) -> bool {
        matches!(self, KeyCheck::Usable)
    }

    /// The line Settings prints.
    pub fn message(&self) -> String {
        match self {
            KeyCheck::Usable => format!(
                "{LABEL} key accepted. Image generation is billed per image and has no free \
                 tier — whether this account can pay is only known on the first Generate"
            ),
            KeyCheck::BadKey => format!("{LABEL} rejected this key"),
            KeyCheck::BillingRequired { detail } => {
                format!(
                    "{LABEL} image generation requires billing enabled on your account — {detail}"
                )
            }
            KeyCheck::Unknown { detail } => {
                format!("could not check this {LABEL} key: {detail}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    use wobu_influence::{RefBucket, Refs};

    use crate::backend::Discard;
    use crate::negotiate::negotiate;

    /// A one-thread executor, so the adapter's async surface is exercised without
    /// a runtime. `wobu-imagine` names none — it runs on Tauri's — and pulling
    /// tokio in to prove that would undo the claim.
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

    fn backend() -> GeminiBackend {
        GeminiBackend::new("AIza-not-a-real-key")
            .unwrap()
            // Unroutable by definition, so a request that did go out fails as
            // `Unavailable` rather than as anything that could be mistaken for
            // success.
            .with_base_url("http://127.0.0.1:1/v1beta")
    }

    #[test]
    fn health_checks_and_generation_backends_share_the_http_pool() {
        // The command and job paths each construct a backend with their own
        // key value. Pointer identity proves that those short-lived wrappers
        // still use the same process-long reqwest connection/TLS pool.
        let health = GeminiBackend::new("health-check-key").unwrap();
        let job = GeminiBackend::new("generation-key").unwrap();
        assert!(Arc::ptr_eq(&health.client, &job.client));
        assert_ne!(health.api_key, job.api_key, "only the transport is shared");
    }

    fn request(model: &str, aspect: &str) -> ImageRequest {
        let caps = backend().capabilities(model);
        let negotiated = negotiate(&[], AspectRatio::parse(aspect).unwrap(), &caps);
        ImageRequest::new(model, "a hooded figure in ash-glazed plate", 42, &negotiated)
    }

    #[test]
    fn each_image_model_declares_its_own_ceiling_and_its_own_reference_budget() {
        // The reason `capabilities` takes a model id at all. One answer for
        // "Gemini" would have to be the lite model's — 1K, and fourteen
        // undifferentiated references — which caps the model the user is paying
        // 4K prices for and hides two thirds of the budget on the pro one.
        let backend = backend();
        assert_eq!(
            backend.capabilities("gemini-3.1-flash-lite-image").max_resolution,
            Resolution::new(1024, 1024),
        );
        assert_eq!(
            backend.capabilities("gemini-3-pro-image").max_resolution,
            Resolution::new(4096, 4096),
        );

        let pro = backend.capabilities("gemini-3-pro-image").image_refs;
        assert_eq!(pro, image_budget("gemini-3-pro-image").unwrap());
        assert_eq!(pro.meter(RefBucket::StyleRefs), (RefBucket::StyleRefs, Refs::new(3)));

        // And the lite model does not *separate* the other two buckets, which is
        // a dash and not a zero: its references are metered as objects.
        let lite = backend.capabilities("gemini-3.1-flash-lite-image").image_refs;
        assert!(!lite.declares(RefBucket::StyleRefs));
        assert_eq!(lite.meter(RefBucket::StyleRefs), (RefBucket::Objects, Refs::new(14)));
    }

    #[test]
    fn a_model_id_nobody_here_has_seen_gets_the_most_conservative_registered_answer() {
        // `project.json` can name a model Google released last week or retired
        // last month, and the Inspector still has to draw a dropdown. An
        // unlimited budget here builds a fourteen-image request for a model that
        // takes six, and the provider points it out after payment.
        let caps = backend().capabilities("gemini-4-ultra-image");
        assert_eq!(caps.max_resolution, Resolution::new(1024, 1024), "the smallest class");
        assert_eq!(caps.image_refs, image_budget("gemini-3-pro-image").unwrap());
        assert_ne!(caps.image_refs, ImageBudget::unlimited());
        assert!(caps.requires_billing, "and an unknown Gemini model is still not free");
    }

    #[test]
    fn nothing_this_backend_lacks_is_declared_as_something_it_has() {
        // Each of these is a UI surface that must not appear and a fragment that
        // must be reported as withheld rather than dropped. Declaring one of them
        // true routes user-authored canon into a field that does not exist.
        for model in ["gemini-3.1-flash-image", "gemini-3-pro-image", "unknown"] {
            let caps = backend().capabilities(model);
            assert_eq!(
                caps.reference_mechanisms.structure.get(),
                0,
                "{model}: no structure adapter of any kind",
            );
            assert!(!caps.loras, "{model}");
            assert!(!caps.negative_prompt, "{model}: Imagen had one and Imagen is gone");
            assert!(!caps.streaming_preview, "{model}: one inline image and nothing before it");
        }
    }

    #[test]
    fn every_shape_the_dropdown_offers_is_one_this_backend_declares() {
        // `AspectRatio::ALL` is written from the intersection both of Google's
        // image pages agree on, so declaring anything less would drop a shape
        // from the dropdown that the API takes. Declaring the list at all is what
        // makes a returned square checkable against a `21:9` request.
        let caps = backend().capabilities(DEFAULT_MODEL);
        for aspect in AspectRatio::ALL {
            assert!(caps.supports_aspect(aspect), "{aspect}");
            assert_eq!(caps.nearest_aspect(aspect), aspect);
        }
        assert!(
            !caps.aspect_ratios.is_empty(),
            "an empty list would mean the parameter is ignored"
        );
    }

    #[test]
    fn a_resolution_rounds_to_a_documented_size_class_with_an_uppercase_k() {
        // `docs/08-providers.md` calls the capital out because a lowercase `1k`
        // is a 400 on a request that is otherwise perfect. The rounding is up to
        // the class containing the *long* side, so a `21:9` matte asks for the
        // class its width needs rather than the one its height suggests.
        let flash = backend().capabilities("gemini-3.1-flash-image");
        assert_eq!(size_class(&request("gemini-3.1-flash-image", "1:1"), &flash).unwrap(), "4K");
        assert_eq!(size_class(&request("gemini-3.1-flash-image", "21:9"), &flash).unwrap(), "4K");

        // The lite model is 1K only, and the class follows the ceiling rather
        // than the model name — which is what keeps the two from disagreeing.
        let lite = backend().capabilities("gemini-3.1-flash-lite-image");
        assert_eq!(
            size_class(&request("gemini-3.1-flash-lite-image", "1:1"), &lite).unwrap(),
            "1K"
        );
        assert_eq!(
            size_class(&request("gemini-3.1-flash-lite-image", "9:16"), &lite).unwrap(),
            "1K",
            "the long side decides, and a portrait's long side is its height",
        );

        // 2K is reached by a ceiling between the two, which is what a model
        // released against a middle size class would declare.
        let middling = Capabilities { max_resolution: Resolution::new(2048, 2048), ..flash };
        let negotiated = negotiate(&[], AspectRatio::parse("1:1").unwrap(), &middling);
        let request = ImageRequest::new("gemini-3.1-flash-image", "p", 0, &negotiated);
        assert_eq!(size_class(&request, &middling).unwrap(), "2K");
    }

    #[test]
    fn a_size_the_model_cannot_reach_is_reported_as_our_bug_rather_than_shrunk() {
        // Unreachable if `negotiate` ran: the ceiling it sized against is the one
        // this checks. Reaching it means the capabilities and the adapter
        // disagree — and the alternative, quietly asking for 1K, is the user
        // paying 4K prices for a thumbnail.
        let oversized = ImageRequest {
            resolution: Resolution::new(4096, 4096),
            ..request("gemini-3.1-flash-lite-image", "1:1")
        };
        let caps = backend().capabilities("gemini-3.1-flash-lite-image");
        let error = size_class(&oversized, &caps).unwrap_err();
        assert_eq!(error.code(), "internal");
        assert!(!error.is_retryable());
        assert!(error.to_string().contains("gemini-3.1-flash-lite-image"), "{error}");
    }

    #[test]
    fn the_billing_message_names_the_fix_and_cannot_be_read_as_a_bad_key() {
        // The support ticket `docs/08-providers.md` predicts, and the whole
        // reason this adapter exists rather than a status code. The two failures
        // send the user to two different websites, so the sentences have to be
        // impossible to confuse — and the billing one has to carry the link,
        // because the webview shows this string and nothing else.
        let billing = no_free_tier().to_string();
        assert!(billing.contains("Gemini image generation requires billing"), "{billing}");
        assert!(billing.contains("your account"), "{billing}");
        assert!(billing.contains(BILLING_URL), "{billing}");
        assert!(billing.contains("free tier"), "{billing}");
        assert!(billing.contains("the key itself is fine"), "{billing}");

        let bad_key = Error::BadKey { backend: LABEL }.to_string();
        assert!(bad_key.contains("rejected the API key"), "{bad_key}");
        assert!(!bad_key.contains("billing"), "{bad_key}");
        assert!(!billing.contains("rejected"), "{billing}");

        // And neither is retryable, because trying again changes nothing until
        // the user has been somewhere and done something.
        assert!(!no_free_tier().is_retryable());
        assert_eq!(no_free_tier().code(), "provider.billing_required");
    }

    #[test]
    fn settings_is_told_what_a_free_probe_can_and_cannot_prove() {
        // A probe that reported "ready to generate" off a models call would be
        // wrong for exactly the user this whole path exists for: the one with a
        // working key and no card. The only call that proves the account can pay
        // is one that charges it.
        let usable = KeyCheck::Usable;
        assert!(usable.is_usable());
        assert!(usable.message().contains("no free tier"), "{}", usable.message());
        assert!(usable.message().contains("first Generate"), "{}", usable.message());

        let billing = KeyCheck::BillingRequired { detail: "enable billing".into() };
        assert!(!billing.is_usable());
        assert!(billing.message().contains("requires billing"), "{}", billing.message());

        // A network failure is not a verdict on the key, and Settings must not
        // print one.
        let unknown = KeyCheck::Unknown { detail: "connection refused".into() };
        assert!(!unknown.is_usable());
        assert!(unknown.message().starts_with("could not check"), "{}", unknown.message());
        assert_ne!(unknown.message(), KeyCheck::BadKey.message());
    }

    #[test]
    fn the_size_recorded_is_the_one_the_picture_has_and_not_the_one_that_was_asked_for() {
        // The instruction `docs/08-providers.md` gives in as many words: there
        // are credible reports of `imageSize` and `aspect_ratio` being silently
        // ignored, so read back the returned dimensions and trust those. Here the
        // request was negotiated to 4096×2304 and the response is a 1024×1024
        // square — which is exactly what a silently ignored aspect ratio looks
        // like. `Asset.width`/`height` must record the square, or every thumbnail
        // built from it is stretched and nothing fails.
        let asked = request(DEFAULT_MODEL, "16:9");
        assert_eq!(asked.resolution, Resolution::new(4096, 2304));

        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&1024u32.to_be_bytes());
        png.extend_from_slice(&1024u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        let body = serde_json::to_vec(&serde_json::json!({
            "status": "completed",
            "steps": [{"type": "model_output", "content": [{"type": "image",
                // Deliberately mislabelled as JPEG as well: what the response
                // claims and what the bytes are do not have to agree, and only
                // one of them is a fact.
                "mime_type": "image/jpeg",
                "data": base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD, &png),
            }]}],
        }))
        .unwrap();

        let image = accept(wire::image(&body).unwrap()).unwrap();
        assert_eq!(image.resolution(), Resolution::new(1024, 1024));
        assert_ne!(image.resolution(), asked.resolution, "the request is a hope, not a fact");
        assert_eq!(image.mime, "image/png", "and the mime comes off the bytes too");
        assert_eq!(image.seed, None, "Google documents no seed, so there is none to claim");
        assert_eq!(
            image.watermark,
            Some(Watermark::SynthId),
            "every image out of these models carries one and no field turns it off",
        );
    }

    #[test]
    fn a_negative_prompt_this_backend_cannot_send_is_refused_rather_than_dropped() {
        // The trait's first contract point. `negotiate` compiles no `never:`
        // fragments for a backend whose `negative_prompt` is false, so this is
        // unreachable — and if it is ever reached, silently leaving the canon out
        // is the one failure the whole negotiation exists to prevent.
        let request = request(DEFAULT_MODEL, "1:1").with_negative("modern firearms");
        let outcome = block_on(backend().generate(&request, &mut Discard, &Cancel::new()));
        let error = outcome.result.unwrap_err();
        assert_eq!(error.code(), "internal", "ours to fix, not the user's");
        assert!(error.to_string().contains("negative-prompt"), "{error}");
        assert!(!outcome.usage.is_billed(), "and nothing left the machine");
    }

    #[test]
    fn a_cancelled_job_never_opens_a_connection() {
        // The queue can cancel a job between queueing it and starting it, and a
        // request sent and then abandoned is one the user is billed for in full.
        let cancel = Cancel::new();
        cancel.cancel();
        let outcome =
            block_on(backend().generate(&request(DEFAULT_MODEL, "1:1"), &mut Discard, &cancel));
        assert!(matches!(outcome.result, Err(Error::Cancelled)));
    }

    #[test]
    fn an_abandoned_call_is_counted_as_billed_because_google_does_not_stop() {
        // `backend.rs`: "a remote render that is abandoned rather than stopped is
        // billed in full". There is no call that stops Gemini generating —
        // dropping the response only stops us waiting — so #55's ceiling has to
        // count it, and a response that arrived and could not be read even more
        // so.
        assert!(billed_by(&Error::Cancelled).is_billed());
        assert!(billed_by(&Error::NotAnImage { detail: "html".into() }).is_billed());
        assert!(billed_by(&Error::NoImage).is_billed());

        // And a refusal generated no pixels. Claiming a charge would take away
        // the retry `error.rs` deliberately gives it, on the grounds that
        // Google's blocking is not deterministic.
        assert!(!billed_by(&Error::Refused { detail: "safety".into() }).is_billed());
        assert!(!billed_by(&no_free_tier()).is_billed());
        assert!(!billed_by(&Error::BadKey { backend: LABEL }).is_billed());
    }

    #[test]
    fn a_backend_works_through_a_box_dyn_and_needs_no_network_to_build() {
        // `project.json` names the backend, so the generate path holds a
        // `Box<dyn ImageBackend>` — and the Inspector draws the dropdown on a
        // machine that is offline.
        let boxed: Box<dyn ImageBackend> = Box::new(GeminiBackend::new("AIza-x").unwrap());
        assert_eq!(boxed.id(), ID);
        assert_eq!(boxed.label(), LABEL);
        assert_eq!(boxed.default_model(), DEFAULT_MODEL);
        assert!(boxed.capabilities(DEFAULT_MODEL).requires_billing);

        // The two backends in this crate do not share an id: it is the keychain
        // entry and the `backend` field of every `Generation`.
        assert_ne!(ID, crate::comfy::ID);
        assert_ne!(DEFAULT_MODEL, crate::comfy::DEFAULT_MODEL);
    }

    #[test]
    fn the_image_backend_and_the_text_provider_share_one_key_entry() {
        // Google issues one API key per project and it is the same key for text
        // and for images. Two ids would be two keychain entries, and a user who
        // pasted their key into Settings once would be told Generate has no key.
        assert_eq!(ID, wobu_llm::gemini::ID);
        assert_eq!(LABEL, wobu_llm::gemini::LABEL);
        // The default *models* are not shared, and must not be: one of these is
        // free and one of them is not.
        assert_ne!(DEFAULT_MODEL, wobu_llm::gemini::DEFAULT_MODEL);
    }

    #[test]
    fn debug_output_does_not_carry_the_key() {
        // `redact::scrub` at the command boundary is the real guard, but a
        // `{backend:?}` in a log line has already leaked by the time it gets
        // there.
        let printed = format!("{:?}", GeminiBackend::new("AIzaSyD-secret-key-material").unwrap());
        assert!(!printed.contains("AIza"), "{printed}");
        assert!(printed.contains(API_ROOT), "{printed}");
    }

    #[test]
    fn the_endpoint_is_the_interactions_api_at_the_revision_this_was_written_against() {
        // Two failures this pins, both of which read as a broken key from the UI:
        // the legacy `:generateContent` path, whose request shape this adapter
        // does not speak, and `/v1beta2`, which one of Google's own pages prints
        // and four others contradict. Pinned identically to `wobu-llm`'s, because
        // if one of them is wrong both are.
        assert!(API_ROOT.ends_with("/v1beta"), "{API_ROOT}");
        assert!(!API_ROOT.contains("generateContent"));
        assert_eq!(API_REVISION, "2026-05-20");
    }
}
