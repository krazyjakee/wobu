//! The image-backend trait: one shape that ComfyUI and Gemini both fit.
//!
//! Two adapters land against this rather than one and then the other
//! ([#51](https://github.com/krazyjakee/wobu/issues/51),
//! [#52](https://github.com/krazyjakee/wobu/issues/52)), for the reason
//! `wobu-llm`'s `provider.rs` gives: a trait written against a single vendor is
//! that vendor's request struct wearing a trait, and the second adapter is the
//! one that pays for it. The two here are further apart than Anthropic and
//! Gemini were — one is a local HTTP server driving a node graph with no
//! credentials and no billing, the other is a remote paid API with no graph, no
//! previews and no seed anybody has documented. Everything below is at the
//! intersection, and the places one vendor forced the shape say which.
//!
//! What is deliberately *not* here: a node graph, a workflow template, a
//! checkpoint loader, a sampler name, a CFG scale, an HTTP client, or a runtime.
//! Those are ComfyUI's request, and the moment one of them appears in this file
//! the Gemini adapter is carrying a field it has to ignore.
//!
//! ## How this composes with the job queue rather than shadowing it
//!
//! [#50](https://github.com/krazyjakee/wobu/issues/50) sketches `submit` →
//! `JobHandle` → `progress` → `cancel`, which is a second queue.
//! [#49](https://github.com/krazyjakee/wobu/issues/49) already owns admission,
//! concurrency, retries, backoff, the cancellation grace period and the event
//! names, so this is one method instead:
//!
//! - **submit** is `wobu_jobs::Queue::submit`. A generate task holds a
//!   `Box<dyn ImageBackend>` and calls [`generate`](ImageBackend::generate) from
//!   its `run`; the job id the frontend gets back is the queue's, minted before
//!   the work starts. A `JobHandle` here would be a second id for the same run,
//!   and `job_cancel` would have to know which one it was given.
//! - **progress** is `JobContext::progress` / `JobContext::preview`, emitted as
//!   `job:progress` and `job:preview`. [`ProgressSink`] is the adapter's end of
//!   that pipe and names none of those types, so this crate does not depend on
//!   the queue — which it must not, because `wobu-jobs` will want to read this
//!   crate's [`Error`] the way it already reads `wobu-llm`'s, and that edge only
//!   works in one direction.
//! - **cancel** is the [`Cancel`] token the queue already hands every task, and
//!   `wobu_jobs::Cancel` *is* this type. A backend-owned `cancel(&handle)` would
//!   be a second stop button that the queue's grace period knew nothing about.
//!
//! ## One request is one image
//!
//! A preset that emits eight images is eight jobs, not one call for eight. That
//! is the queue's decision to make and this shape is what lets it: cancellation
//! stops after the current image rather than losing seven, a retry re-spends one
//! image rather than eight, and `Preset::locks_seed` means the seed of each is
//! decided before the call rather than derived inside an adapter. ComfyUI's
//! batching would be marginally cheaper and is not worth paying for in
//! granularity on the one axis — money — where #49 is strictest.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use wobu_core::{AssetRole, Id};
use wobu_influence::{Fragment, RefBucket};
use wobu_llm::Cancel;

use crate::aspect::{AspectRatio, Resolution};
use crate::capability::{Capabilities, ReferenceMechanism};
use crate::error::{Error, Result};
use crate::negotiate::Negotiated;

/// One reference image, already resolved to bytes and already routed.
///
/// **Bytes and not a path.** A "local" ComfyUI is a server that may be on
/// another machine on the LAN, and it takes uploads over HTTP like any other;
/// Gemini takes base64 inline. Neither can open a file in the project folder, so
/// a path here would be a field one of them has to turn into bytes anyway — and
/// this crate does no IO, so it could not be the one to do it.
///
/// The counting bucket and routing mechanism are both carried rather than
/// re-derived. They answer different questions: #44's bucket says which quota
/// the picture consumed, while #86's mechanism says which adapter path must
/// apply it. Keeping both prevents a ComfyUI adapter from mistaking Gemini's
/// `Characters` counter for a ControlNet input.
#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    /// For the generation record and for the log. The adapter never resolves it.
    pub asset_id: Id,
    /// What the user attached it as. Not what the backend calls it — that is
    /// [`bucket`](Self::bucket) — and carried because it is what the Inspector
    /// labels the picture with.
    pub role: AssetRole,
    /// Which of the provider's counting buckets it consumed.
    pub bucket: RefBucket,
    /// The adapter path that negotiation selected for this image.
    pub mechanism: ReferenceMechanism,
    /// `link.weight × section_priority × user_slider`, already multiplied out.
    ///
    /// It has already done its main job by deciding which references survived
    /// the budget. A backend with a per-reference strength — ComfyUI's
    /// IP-Adapter has one — uses it; Gemini has nothing to put it in and ignores
    /// it, which is not a silent drop because the picture itself is still sent.
    pub weight: f32,
    pub bytes: Vec<u8>,
    pub mime: String,
}

impl Reference {
    /// Build one from the fragment that earned the slot.
    ///
    /// `None` for a fragment with no asset behind it, which is the only way this
    /// can fail. Taking the fragment rather than an id and a role is what stops
    /// an adapter from assembling a reference that no negotiation kept: the
    /// weight, the role and the bucket all come from the thing that survived.
    pub fn from_fragment(
        fragment: Fragment<'_>,
        bucket: RefBucket,
        bytes: Vec<u8>,
        mime: impl Into<String>,
    ) -> Option<Reference> {
        Some(Reference {
            asset_id: fragment.asset_id()?,
            role: fragment.asset_role()?,
            bucket,
            mechanism: ReferenceMechanism::for_target(fragment.target())?,
            weight: fragment.weight(),
            bytes,
            mime: mime.into(),
        })
    }
}

/// One generation: one prompt, one seed, one picture.
///
/// Everything in it has been through [`negotiate`](crate::negotiate), which is
/// why there is no `Option` anywhere and no flag saying whether a field applies.
/// The strongest case is [`negative`](Self::negative): it is compiled from
/// `Negotiated::fragments`, and on a backend with no negative prompt the
/// `never:` fragments are not in that list, so an empty string here is a
/// structural consequence rather than something a caller has to remember.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRequest {
    /// Backend-specific and opaque to this crate: a Gemini model id, a ComfyUI
    /// checkpoint filename. It comes from `project.json` and means nothing
    /// without the adapter that reads it. Model ids move faster than anything
    /// else in `docs/08-providers.md`, which is why there is no enum of them.
    pub model: String,
    /// The compiled positive prompt.
    pub prompt: String,
    /// The compiled negative prompt, empty when there is none — either because
    /// nothing in the stack declared a `never:` or because the backend does not
    /// take one, and those two are told apart by
    /// [`Negotiated::downgrades`](crate::Negotiated::downgrades) rather than by
    /// an `Option` here that every adapter would have to interpret.
    pub negative: String,
    /// The shape the backend will actually produce, which is not necessarily the
    /// one the preset asked for.
    pub aspect: AspectRatio,
    /// The pixel dimensions, sized to the backend's ceiling. ComfyUI uses these
    /// directly; Gemini rounds to its nearest documented size class and then —
    /// per `docs/08-providers.md` — reads back what actually came out rather
    /// than trusting either.
    pub resolution: Resolution,
    /// What `Generation.seed` records, and what makes a turnaround's eight views
    /// eight views of one object rather than eight objects
    /// (`Preset::locks_seed`).
    ///
    /// Chosen by the caller and never by an adapter, because a seed invented
    /// downstream cannot be written into the record that is supposed to
    /// reproduce the image. 🚩 Google does not document a seed parameter for the
    /// image models; if it turns out there is none, the answer is a capability
    /// and a [`Downgrade`](crate::Downgrade) — the user is told their turnaround
    /// will not be consistent — and never an adapter quietly generating one.
    pub seed: u64,
    /// In reading order across the buckets, exactly as
    /// `CompiledImages::kept` yields them.
    pub references: Vec<Reference>,
}

impl ImageRequest {
    /// The aspect ratio and resolution come from the negotiation and cannot be
    /// set separately.
    ///
    /// The same discipline as `EnhanceRequest`'s schema following its kind: a
    /// request built for one shape and sized for another would read as the
    /// backend ignoring the dimensions it was given, and the fix would be looked
    /// for in the adapter.
    pub fn new(
        model: impl Into<String>,
        prompt: impl Into<String>,
        seed: u64,
        negotiated: &Negotiated<'_>,
    ) -> ImageRequest {
        ImageRequest {
            model: model.into(),
            prompt: prompt.into(),
            negative: String::new(),
            aspect: negotiated.aspect(),
            resolution: negotiated.resolution(),
            seed,
            references: Vec::new(),
        }
    }

    pub fn with_negative(mut self, negative: impl Into<String>) -> ImageRequest {
        self.negative = negative.into();
        self
    }

    pub fn with_references(mut self, references: Vec<Reference>) -> ImageRequest {
        self.references = references;
        self
    }

    /// The references using one adapter mechanism, in reading order.
    ///
    /// This is routing, unlike `Reference::bucket`, which remains attached for
    /// quota reporting. Gemini emits every image into one flat block list; a
    /// future ComfyUI workflow can select only its ControlNet inputs here.
    pub fn in_mechanism(&self, mechanism: ReferenceMechanism) -> impl Iterator<Item = &Reference> {
        self.references.iter().filter(move |r| r.mechanism == mechanism)
    }
}

/// Where a backend says how it is getting on.
///
/// A sink rather than a returned stream, for the reasons `wobu-llm`'s
/// `DeltaSink` gives — no combinator pipeline to justify the `Pin<Box<dyn
/// Stream>>` a `dyn` trait would need, and cancellation stays a thing the caller
/// holds rather than something achieved by dropping a stream — plus one this
/// crate has of its own: the consumer is `JobContext::progress`, which is fire
/// and forget, and a stream would put a queue of progress ticks between a
/// sampler and a status bar that only ever draws the last one.
///
/// **Primitives rather than a struct, deliberately.** These arguments are the
/// fields of `wobu_jobs::Progress` and `wobu_jobs::Preview`. Naming those types
/// would mean depending on the queue, which this crate must not do; defining
/// look-alikes here would mean two copies of a wire shape that has to match
/// `src/lib/api.ts`. Passing the fields commits to neither, and the bridge in a
/// generate task is one line per method.
pub trait ProgressSink: Send {
    /// How far through, in the backend's own units.
    ///
    /// Throttle at the source. ComfyUI sends one of these per sampler step and
    /// the status bar draws a percentage, so a thirty-step render is thirty
    /// events for a bar with a hundred positions — which is fine, and three
    /// hundred would not be. `note` is what makes "sampling 12/30" possible
    /// where "40%" is not.
    fn step(&mut self, done: u32, total: u32, note: Option<&str>);

    /// An image of the work in flight — a ComfyUI latent preview.
    ///
    /// Defaulted to nothing, because a backend that declares
    /// [`Capabilities::streaming_preview`] false must never call it and a caller
    /// that draws no preview surface should not have to write an empty body.
    /// `image` is an opaque string for the same reason `wobu_jobs::Preview::image`
    /// is: whether previews reach the webview as a `data:` URL or an `asset://`
    /// path is #40's decision and is not settled, and nothing on the way past
    /// should buffer image bytes.
    fn preview(&mut self, image: &str, step: Option<u32>) {
        let _ = (image, step);
    }
}

/// A sink for callers that only want the picture — a batch run, a test. Named
/// rather than an empty closure so that reaching for it reads as a decision.
pub struct Discard;

impl ProgressSink for Discard {
    fn step(&mut self, _done: u32, _total: u32, _note: Option<&str>) {}
}

/// What a call is known to have cost, whatever it did afterwards.
///
/// The image analogue of `wobu_llm::Usage`, and it counts images rather than
/// tokens because that is what the vendors bill: Gemini charges per image at a
/// price that depends on the model and the size, both of which are in the
/// request, so the only fact the outcome has to add is how many were made.
///
/// A struct with one field rather than a bare `u32`, for the reason `Usage` is
/// one: this is what the spend ceiling reads, it will grow the day a provider
/// reports its own charge back, and a `u32` in a signature is a number anybody
/// can pass anything to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImageUsage {
    /// Images the backend generated and will charge for.
    ///
    /// Zero means "nothing we know of was billed", which is not the same as
    /// "nothing was billed": a call cancelled after the provider started
    /// generating may still be paid for. Adapters report the last figure they
    /// have rather than waiting for a clean finish, because #55's ceiling is
    /// only a ceiling if it counts those.
    pub billed_images: u32,
}

impl ImageUsage {
    /// What a local backend reports, and what an adapter reports when it knows
    /// the call never reached the provider. Named so that reaching for it is a
    /// claim about billing rather than a convenience.
    pub fn free() -> ImageUsage {
        ImageUsage::default()
    }

    pub fn billed(images: u32) -> ImageUsage {
        ImageUsage { billed_images: images }
    }

    /// Whether money moved. `wobu_jobs::Billed` is built from this, and the
    /// queue holds a retry when it is true.
    pub fn is_billed(self) -> bool {
        self.billed_images > 0
    }
}

/// The result of one generation: what it cost, and what came back.
///
/// Not a `Result`, on purpose, and the argument is `wobu-llm`'s verbatim because
/// it is worth more here: `?` on a `Result<_, E>` would carry the error out and
/// leave the usage behind, and "the call failed so nothing was charged" is false
/// often enough — a refusal after generation, a cancellation mid-render, a
/// response we could not decode — that the spend ceiling would drift low
/// precisely when the user is hitting limits. Destructuring is the only way past
/// this type, and destructuring puts [`ImageUsage`] in front of whoever wrote
/// the call.
#[derive(Debug)]
pub struct ImageOutcome {
    pub usage: ImageUsage,
    pub result: Result<GeneratedImage>,
}

impl ImageOutcome {
    pub fn new(usage: ImageUsage, result: Result<GeneratedImage>) -> ImageOutcome {
        ImageOutcome { usage, result }
    }

    /// A failure before the backend could have charged anything: no key, a
    /// refused connection, a cancellation that beat the request out of the door.
    /// Anything that got as far as a generated image should be reporting real
    /// figures through [`ImageOutcome::new`].
    pub fn unbilled(error: Error) -> ImageOutcome {
        ImageOutcome { usage: ImageUsage::free(), result: Err(error) }
    }

    pub fn is_ok(&self) -> bool {
        self.result.is_ok()
    }
}

/// One picture, as bytes, with the dimensions it actually has.
///
/// No path and no asset id: `wobu-imagine` does no IO, and where this lands is
/// the store's decision — content-addressed under `assets/`, which means the
/// bytes have to exist before the path does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    pub bytes: Vec<u8>,
    pub mime: String,
    /// Read back from the image, not copied from the request.
    /// `docs/08-providers.md` is explicit about why: there are credible reports
    /// of Gemini silently ignoring `imageSize` and `aspect_ratio`, so the
    /// dimensions we asked for are a hope and these are the fact. They are what
    /// `Asset.width`/`height` record.
    pub width: u32,
    pub height: u32,
    /// The seed the backend says it used, when it says.
    ///
    /// `None` is "it did not tell us", not "there wasn't one". ComfyUI resolves
    /// a randomised seed server-side and reports the value, which is the only
    /// way a generation the user liked can be reproduced; a backend that reports
    /// nothing leaves `Generation.seed` as the one we asked for, which is the
    /// best claim anyone can make.
    pub seed: Option<u64>,
    /// Whether the provider says these pixels carry an invisible watermark.
    ///
    /// `None` is "no watermark was declared", which is where a local render
    /// lands. See [`Watermark`] for why this is on the image rather than in a
    /// sentence somewhere.
    pub watermark: Option<Watermark>,
}

impl GeneratedImage {
    pub fn resolution(&self) -> Resolution {
        Resolution::new(self.width, self.height)
    }
}

/// An invisible watermark the provider says it embedded in the pixels.
///
/// Data on the outcome rather than a sentence an adapter logs, because the two
/// places this has to reach are UI: the generation card
/// ([#54](https://github.com/krazyjakee/wobu/issues/54)) and whatever exports or
/// hands the file on ([#59](https://github.com/krazyjakee/wobu/issues/59)).
/// `docs/08-providers.md` asks for it to be stated because this is concept art
/// headed into a pipeline — a plate that carries a detectable mark downstream is
/// a fact about the asset, and one nobody can discover by looking at it.
///
/// **A claim by the provider, not a measurement.** Nothing here inspects the
/// pixels; this is what the vendor documents about its own output, which is the
/// only source there is — the mark is designed not to be visible to us either.
///
/// An enum with one variant rather than a `bool`, because a `bool` on
/// [`GeneratedImage`] could not say *which* scheme, and "watermarked" with no
/// name is not something a person can go and read about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Watermark {
    /// Google's scheme. Every image out of every Gemini image model carries one
    /// (`docs/08-providers.md`); there is no request field that turns it off, so
    /// the adapter declares it unconditionally rather than reading it back.
    SynthId,
}

impl Watermark {
    /// What the UI prints. Capitalised the way the vendor writes it, because it
    /// is a proper noun the user may go and search for.
    pub fn label(self) -> &'static str {
        match self {
            Watermark::SynthId => "SynthID",
        }
    }
}

/// An image backend, selected per project and held behind a `dyn`.
///
/// `#[async_trait]` rather than a native `async fn` for the same reason
/// `TextProvider` uses it: the backend is chosen at runtime from `project.json`,
/// so this is stored as `Box<dyn ImageBackend>`, and a native async trait method
/// gives no `Send` future to put in a box. Nothing here names a runtime — Tauri's
/// is the one it will run on — and nothing here names an HTTP client.
#[async_trait]
pub trait ImageBackend: Send + Sync {
    /// Stable id: the `backend` in `project.json`, the `backend` field of every
    /// `Generation`, and the `wobu/<backend>` entry in the OS keychain.
    /// Renaming one breaks all three, and the third breaks silently.
    fn id(&self) -> &'static str;

    /// The name a person sees, including inside error messages built here.
    fn label(&self) -> &'static str;

    /// Used when a project names this backend but no model.
    fn default_model(&self) -> &'static str;

    /// What this backend can do with this model.
    ///
    /// Per model rather than per backend, which the reference-budget table in
    /// `docs/08-providers.md` forces: three Gemini image models, three different
    /// budgets and three different ceilings. One answer for the whole backend
    /// would have to be the worst of them.
    ///
    /// Total: a model id this backend has never heard of still gets an answer,
    /// because the Inspector renders a dropdown from it and a project may name a
    /// model that was removed. The answer must be the most conservative
    /// *registered* one and never
    /// [`ImageBudget::unlimited`](wobu_influence::ImageBudget::unlimited) —
    /// handing an unknown remote model an unlimited reference budget builds a
    /// request the provider rejects after payment.
    fn capabilities(&self, model: &str) -> Capabilities;

    /// Make one image.
    ///
    /// Contract for implementors, all of it load-bearing:
    ///
    /// 1. **Send exactly what the request says, or fail.** Every field has been
    ///    through [`negotiate`](crate::negotiate), so an adapter that finds
    ///    itself substituting a value has been handed something this backend
    ///    said it could take. That is [`Error::Unsupported`] — our bug, reported
    ///    as one — and never a quiet substitution, which is the failure the
    ///    whole negotiation exists to prevent.
    /// 2. **Honour `cancel` by stopping**, not by finishing and discarding the
    ///    answer. Racing the next read against `Cancel::cancelled` is the shape
    ///    this is designed for; polling `Cancel::check` between chunks alone
    ///    leaves the user paying for however long a quiet provider takes. A
    ///    remote render that is abandoned rather than stopped is billed in full,
    ///    and a local one holds a GPU the next job in the queue is waiting for.
    /// 3. **Report cost honestly.** The returned [`ImageUsage`] is the best
    ///    figure known at the moment the call ended, success or not. It is the
    ///    only thing standing between the user and a silent second charge, and
    ///    neither this crate nor the queue can check it.
    /// 4. **Do not call [`ProgressSink::preview`] unless
    ///    [`Capabilities::streaming_preview`] is true.** The UI reserves the
    ///    surface off that flag, and one that is false and then draws is a
    ///    layout that shifts under the user mid-render.
    /// 5. **Read the dimensions back off the image**, rather than echoing the
    ///    ones that were requested (`docs/08-providers.md`).
    /// 6. **Declare a [`Watermark`] where the provider says it embeds one.** It
    ///    cannot be seen, so an image that arrives without the declaration is an
    ///    image nobody downstream can know is marked.
    async fn generate(
        &self,
        request: &ImageRequest,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> ImageOutcome;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aspect::AspectRatio;
    use crate::capability::ReferenceMechanisms;
    use crate::negotiate::negotiate;
    use wobu_influence::{ImageBudget, Refs};

    fn caps() -> Capabilities {
        Capabilities {
            max_resolution: Resolution::new(2048, 2048),
            aspect_ratios: vec![AspectRatio::parse("16:9").unwrap()],
            image_refs: ImageBudget {
                objects: Refs::new(6),
                characters: Some(Refs::new(5)),
                style_refs: Some(Refs::new(3)),
            },
            reference_mechanisms: ReferenceMechanisms::image_prompt(),
            loras: false,
            negative_prompt: false,
            requires_billing: true,
            streaming_preview: false,
        }
    }

    #[test]
    fn a_request_carries_the_shape_the_negotiation_settled_on_and_not_the_one_asked_for() {
        // The regression: a request that names `21:9` and is sized `16:9`, which
        // reads as the backend ignoring the dimensions it was given, and which
        // would be looked for in the adapter. The two cannot be set separately
        // because neither is a parameter of `new`.
        let negotiated = negotiate(&[], AspectRatio::parse("21:9").unwrap(), &caps());
        let request = ImageRequest::new("gemini-3-pro-image", "a hooded figure", 42, &negotiated);
        assert_eq!(request.aspect, AspectRatio::parse("16:9").unwrap());
        assert_eq!(request.resolution, Resolution::new(2048, 1152));
        assert_eq!(request.aspect.fit(caps().max_resolution), request.resolution);
        assert_eq!(request.seed, 42);
    }

    #[test]
    fn a_request_starts_with_no_negative_prompt_rather_than_an_optional_one() {
        // On a backend with no negative prompt the `never:` fragments never
        // reach `compile`, so the string a caller would have to set is empty
        // anyway. What this pins is that the default is the safe one: a request
        // built and never given a negative carries none, rather than carrying
        // whatever a `Default` felt like.
        let negotiated = negotiate(&[], AspectRatio::parse("16:9").unwrap(), &caps());
        let request = ImageRequest::new("m", "p", 0, &negotiated);
        assert!(request.negative.is_empty());
        assert_eq!(request.with_negative("modern firearms").negative, "modern firearms");
    }

    #[test]
    fn references_come_back_grouped_by_the_mechanism_the_adapter_uses() {
        let request = ImageRequest {
            references: vec![
                reference(AssetRole::Costume, RefBucket::StyleRefs),
                reference(AssetRole::Palette, RefBucket::Objects),
                reference(AssetRole::Material, RefBucket::StyleRefs),
            ],
            ..ImageRequest::new(
                "m",
                "p",
                0,
                &negotiate(&[], AspectRatio::parse("16:9").unwrap(), &caps()),
            )
        };
        let image_prompts: Vec<_> = request
            .in_mechanism(ReferenceMechanism::ImagePrompt)
            .map(|r| r.role.as_str())
            .collect();
        assert_eq!(image_prompts, ["costume", "palette", "material"]);
        assert_eq!(request.in_mechanism(ReferenceMechanism::Structure).count(), 0);
    }

    fn reference(role: AssetRole, bucket: RefBucket) -> Reference {
        Reference {
            asset_id: Id::nil(),
            role,
            bucket,
            mechanism: ReferenceMechanism::for_target(role.target()).unwrap(),
            weight: 1.0,
            bytes: vec![0x89, b'P', b'N', b'G'],
            mime: "image/png".into(),
        }
    }

    #[test]
    fn an_unbilled_failure_says_zero_rather_than_saying_nothing() {
        // `usage` is not optional, so "we do not know" and "nothing was charged"
        // have to be the same value; the spend ceiling reads it either way.
        let outcome = ImageOutcome::unbilled(Error::NoKey { backend: "Gemini" });
        assert_eq!(outcome.usage, ImageUsage::free());
        assert!(!outcome.usage.is_billed());
        assert!(!outcome.is_ok());
    }

    #[test]
    fn a_failure_after_the_image_was_made_still_reports_what_it_cost() {
        // The case the whole non-`Result` shape exists for: a refusal or a
        // decode failure after the provider generated and charged. If the usage
        // rode out with the error it would be dropped by the first `?`, and the
        // ceiling would drift low exactly when the user is hitting limits.
        let outcome = ImageOutcome::new(
            ImageUsage::billed(1),
            Err(Error::NotAnImage { detail: "no PNG signature".into() }),
        );
        assert!(outcome.usage.is_billed());
        assert!(!outcome.is_ok());
    }

    #[test]
    fn a_generated_image_reports_the_size_it_has_and_not_the_size_it_was_asked_for() {
        // `docs/08-providers.md`: there are credible reports of `imageSize` and
        // `aspect_ratio` being silently ignored, so `Asset.width`/`height` must
        // come from the bytes. A thumbnail generated against the requested
        // dimensions would be stretched, and nothing would fail.
        let image = GeneratedImage {
            bytes: vec![],
            mime: "image/png".into(),
            width: 1024,
            height: 1024,
            seed: Some(7),
            watermark: None,
        };
        assert_eq!(image.resolution(), Resolution::new(1024, 1024));
        assert_ne!(image.resolution(), Resolution::new(2048, 1152));
    }

    #[test]
    fn a_watermark_is_carried_as_data_because_nobody_can_see_it() {
        // `docs/08-providers.md` asks for SynthID to be stated in the UI, and
        // the UI is #54's card and #59's export. A watermark mentioned only in a
        // log line is a fact about a file that leaves this tool without it.
        assert_eq!(Watermark::SynthId.label(), "SynthID");
        assert_eq!(serde_json::to_value(Watermark::SynthId).unwrap(), "synth_id");
    }

    #[test]
    fn usage_serialises_camel_case_for_the_spend_meter() {
        // #55 reads this over the bridge, and every other wire form in the
        // workspace is camelCase.
        let json = serde_json::to_value(ImageUsage::billed(3)).unwrap();
        assert_eq!(json["billedImages"], 3);
        assert_eq!(
            serde_json::from_value::<ImageUsage>(serde_json::json!({})).unwrap(),
            ImageUsage::free(),
        );
    }
}
