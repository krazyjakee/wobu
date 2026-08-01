//! Two implementations of [`ImageBackend`] that never touch a network, and the
//! behaviour every real adapter has to reproduce.
//!
//! They live outside the crate on purpose: an in-crate fake can reach for
//! private items, so it would prove the trait is implementable *here* rather
//! than implementable at all. Everything below uses only what `wobu-imagine`
//! exports, which is the same position the ComfyUI
//! ([#51](https://github.com/krazyjakee/wobu/issues/51)) and Gemini image
//! ([#52](https://github.com/krazyjakee/wobu/issues/52)) adapters will be in.
//!
//! There are two rather than one because the whole claim of the trait is that it
//! fits both, and the two are further apart than Anthropic and Gemini were: a
//! local server with a node graph, no credentials, no billing and live previews,
//! against a remote paid API with none of those and a hard reference budget. A
//! fake of only one of them would prove nothing the trait is being asked to do.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use wobu_core::{AssetRef, AssetRole, Description, Node, NodeKind, SectionValue, new_id, preset};
use wobu_imagine::{
    AspectRatio, Cancel, Capabilities, Discard, Downgrade, Error, GeneratedImage, ImageBackend,
    ImageOutcome, ImageRequest, ImageUsage, Negotiated, ProgressSink, Reference, Resolution,
    negotiate,
};
use wobu_influence::{
    Fragment, ImageBudget, RefBucket, Refs, Sliders, World, fragments, image_budget, resolve,
};

/// A one-thread executor, so the trait's async surface can be exercised without
/// a runtime dependency. `wobu-imagine` names no runtime — it runs on Tauri's —
/// and pulling tokio in to prove that would undo the claim.
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

/// A local ComfyUI: a node graph on the user's own GPU. Everything the remote
/// one has not, and none of the cost.
struct LocalComfy {
    steps: u32,
    /// Observed from the test rather than inferred from the progress events, so
    /// "cancellation stopped the work" can be told apart from "cancellation
    /// stopped the reporting".
    steps_run: Arc<AtomicUsize>,
}

impl LocalComfy {
    fn new() -> LocalComfy {
        LocalComfy { steps: 4, steps_run: Arc::new(AtomicUsize::new(0)) }
    }
}

#[async_trait::async_trait]
impl ImageBackend for LocalComfy {
    fn id(&self) -> &'static str {
        "comfyui"
    }

    fn label(&self) -> &'static str {
        "ComfyUI"
    }

    fn default_model(&self) -> &'static str {
        "flux-dev"
    }

    fn capabilities(&self, _model: &str) -> Capabilities {
        Capabilities {
            max_resolution: Resolution::new(2048, 2048),
            // Width and height, not a ratio: this backend does not take the
            // parameter, which is not the same as refusing every value of it.
            aspect_ratios: vec![],
            // By name, and not by asking the registry about a checkpoint
            // filename it has never heard of.
            image_refs: ImageBudget::unlimited(),
            reference_mechanisms: wobu_imagine::ReferenceMechanisms::unlimited(),
            loras: true,
            negative_prompt: true,
            requires_billing: false,
            streaming_preview: true,
        }
    }

    async fn generate(
        &self,
        request: &ImageRequest,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> ImageOutcome {
        for step in 1..=self.steps {
            // Checked before the work of the step, not after: a job cancelled
            // while queued must not hold a GPU for one sampler pass first.
            if cancel.is_cancelled() {
                return ImageOutcome::new(ImageUsage::free(), Err(Error::Cancelled));
            }
            self.steps_run.fetch_add(1, Ordering::SeqCst);
            progress.step(step, self.steps, Some("sampling"));
            progress.preview(&format!("data:latent,{step}"), Some(step));
        }
        ImageOutcome::new(
            // A local render costs the user's electricity and nothing a spend
            // ceiling can meter, which is what `free` means.
            ImageUsage::free(),
            Ok(GeneratedImage {
                bytes: b"PNG".to_vec(),
                mime: "image/png".into(),
                width: request.resolution.width,
                height: request.resolution.height,
                // ComfyUI resolves a randomised seed server-side and reports it
                // back, which is the only way a render the user liked can be
                // repeated.
                seed: Some(request.seed),
                // Nothing local marks its own output.
                watermark: None,
            }),
        )
    }
}

/// How the remote backend's call ends, which is the axis every interesting case
/// sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// One image, paid for.
    Image,
    /// A content filter declined — after generating, and after billing.
    Refused,
    /// The account has no billing enabled, which is the first thing a working
    /// Gemini key does on Generate (`docs/08-providers.md`).
    NoBilling,
}

/// Gemini's image models: remote, paid, and with none of ComfyUI's adapters.
struct RemoteGemini {
    ending: Ending,
}

#[async_trait::async_trait]
impl ImageBackend for RemoteGemini {
    fn id(&self) -> &'static str {
        "gemini"
    }

    fn label(&self) -> &'static str {
        "Google Gemini"
    }

    fn default_model(&self) -> &'static str {
        "gemini-3.1-flash-image"
    }

    fn capabilities(&self, model: &str) -> Capabilities {
        Capabilities {
            max_resolution: match model {
                // The lite model is 1K only, which is the reason capabilities
                // are asked per model rather than per backend.
                "gemini-3.1-flash-lite-image" => Resolution::new(1024, 1024),
                _ => Resolution::new(4096, 4096),
            },
            aspect_ratios: AspectRatio::ALL.to_vec(),
            // A model the registry has never heard of falls back to the
            // smallest registered budget, never to `unlimited`: a fourteen-image
            // request to a model that takes six is refused after payment.
            image_refs: image_budget(model)
                .unwrap_or_else(|| image_budget("gemini-3-pro-image").unwrap()),
            reference_mechanisms: wobu_imagine::ReferenceMechanisms::image_prompt(),
            loras: false,
            negative_prompt: false,
            requires_billing: true,
            streaming_preview: false,
        }
    }

    async fn generate(
        &self,
        request: &ImageRequest,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> ImageOutcome {
        if cancel.is_cancelled() {
            return ImageOutcome::unbilled(Error::Cancelled);
        }
        // One request, one response, and nothing in between. Reported anyway, so
        // the status bar has something other than an indeterminate spinner —
        // and `preview` is never called, because this backend declares it has
        // none.
        progress.step(0, 1, Some("waiting on the API"));

        match self.ending {
            Ending::NoBilling => ImageOutcome::unbilled(Error::BillingRequired {
                backend: "Gemini",
                detail: "enable billing on your Google account".into(),
            }),
            // Billed, and the failure carries that: the provider generated
            // before it decided not to hand the result over.
            Ending::Refused => ImageOutcome::new(
                ImageUsage::billed(1),
                Err(Error::Refused { detail: "prohibited_content".into() }),
            ),
            Ending::Image => ImageOutcome::new(
                ImageUsage::billed(1),
                Ok(GeneratedImage {
                    bytes: b"PNG".to_vec(),
                    mime: "image/png".into(),
                    // Deliberately not the requested size. `docs/08-providers.md`
                    // reports `imageSize` being silently ignored, so an adapter
                    // reads the dimensions back and trusts those.
                    width: 1024,
                    height: 1024,
                    seed: None,
                    // Every Gemini image carries one and there is no field to
                    // turn it off (`docs/08-providers.md`).
                    watermark: Some(wobu_imagine::Watermark::SynthId),
                }),
            ),
        }
        .tap_request(request, &self.capabilities(&request.model))
    }
}

/// The adapter's side of the "everything has been negotiated" claim, checked
/// where a real adapter would have had to write a workaround instead.
///
/// Point 1 of the trait's contract is that an adapter sends what the request
/// says or fails — so if a request ever arrived here carrying a negative prompt
/// this backend has no field for, or a shape it does not offer, the honest
/// answer would be `Error::Unsupported` and a bug report. That it cannot is the
/// property `negotiate` exists to provide.
trait TapRequest {
    fn tap_request(self, request: &ImageRequest, caps: &Capabilities) -> Self;
}

impl TapRequest for ImageOutcome {
    fn tap_request(self, request: &ImageRequest, caps: &Capabilities) -> ImageOutcome {
        assert!(
            request.negative.is_empty(),
            "a backend with no negative prompt was handed one: {}",
            request.negative,
        );
        assert!(caps.supports_aspect(request.aspect), "handed {}", request.aspect);
        assert!(request.resolution.fits_in(caps.max_resolution));
        self
    }
}

/// A sink that records what it was told, so the two promises about progress —
/// that it arrives, and that previews do not arrive from a backend that says it
/// has none — can be checked.
#[derive(Default)]
struct Recorder {
    steps: Vec<(u32, u32, Option<String>)>,
    previews: Vec<String>,
}

impl ProgressSink for Recorder {
    fn step(&mut self, done: u32, total: u32, note: Option<&str>) {
        self.steps.push((done, total, note.map(str::to_owned)));
    }

    fn preview(&mut self, image: &str, _step: Option<u32>) {
        self.previews.push(image.to_owned());
    }
}

/// Kael, with a pose reference and two costume references, described well enough
/// that there is a positive prompt and a `never:` to lose.
fn subject() -> Node {
    let mut kael = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
    kael.description = Some(Description::from_sections([
        ("silhouette".to_string(), SectionValue::Text("Tall, narrow, hooded".into())),
        ("never".to_string(), SectionValue::List(vec!["modern firearms".into()])),
    ]));
    kael.asset_links.push(AssetRef::new(new_id(), AssetRole::Pose));
    kael.asset_links.push(AssetRef::new(new_id(), AssetRole::Costume));
    kael.asset_links.push(AssetRef::new(new_id(), AssetRole::Mood));
    kael
}

/// The compiled fragments for that subject under the character-sheet preset,
/// which is what any Generate command holds at the moment it picks a backend.
fn stack(node: &Node) -> Vec<Fragment<'_>> {
    let world = World::new([node]);
    let resolved = resolve(&world, node.id, None).unwrap();
    fragments(&resolved, preset("character_sheet").unwrap(), &Sliders::neutral())
}

/// The references a caller would build from a negotiation, resolving each
/// fragment's asset to bytes it fetched from the store.
fn references(negotiated: &Negotiated<'_>) -> Vec<Reference> {
    negotiated
        .images()
        .buckets()
        .iter()
        .flat_map(|bucket| {
            bucket.kept().iter().filter_map(move |fragment| {
                Reference::from_fragment(
                    *fragment,
                    bucket.bucket(),
                    std::sync::Arc::from(&b"PNG"[..]),
                    "image/png",
                )
            })
        })
        .collect()
}

#[test]
fn one_stack_and_two_backends_produce_two_visibly_different_requests() {
    // The acceptance criterion for #50, end to end and from outside the crate.
    // Neither backend fails, neither logs anything, and the difference between
    // them is a value the UI can render.
    let node = subject();
    let extracted = stack(&node);
    let aspect = AspectRatio::parse(preset("character_sheet").unwrap().aspect).unwrap();

    let comfy = LocalComfy::new();
    let local = negotiate(&extracted, aspect, &comfy.capabilities(comfy.default_model()));
    assert!(local.is_exact(), "a local backend takes everything the stack offered");
    assert_eq!(local.images().kept().count(), 2, "pose and costume; mood was never sendable");

    let gemini = RemoteGemini { ending: Ending::Image };
    let remote = negotiate(&extracted, aspect, &gemini.capabilities(gemini.default_model()));
    assert!(!remote.is_exact());
    let told: Vec<_> =
        remote.downgrades().iter().map(|d| (d.fragment.section(), d.reason)).collect();
    assert_eq!(told, [("never", Downgrade::NotSent), ("pose", Downgrade::MoodboardOnly),]);
    assert_eq!(remote.images().kept().count(), 1, "only the costume reference survives");

    // And the mood reference is in neither report on either backend. It is doing
    // exactly what it was attached to do.
    for negotiated in [&local, &remote] {
        assert!(!negotiated.downgrades().iter().any(|d| d.fragment.section() == "mood"));
        assert_eq!(negotiated.images().dropped().count(), 0);
    }
}

#[test]
fn a_negotiated_request_is_one_the_backend_can_answer_without_substituting_anything() {
    // What the fakes assert from their own side, via `tap_request`: the negative
    // prompt is absent because the fragments that would have compiled into one
    // are absent, not because the adapter remembered to check a flag.
    let node = subject();
    let extracted = stack(&node);
    let aspect = AspectRatio::parse("21:9").unwrap();
    let gemini = RemoteGemini { ending: Ending::Image };
    let caps = gemini.capabilities("gemini-3.1-flash-lite-image");

    let negotiated = negotiate(&extracted, aspect, &caps);
    let request =
        ImageRequest::new("gemini-3.1-flash-lite-image", "a hooded figure", 9, &negotiated)
            .with_references(references(&negotiated));

    // The lite model is 1K only, so the request is sized for it and not for the
    // 4K the pro model would have taken.
    assert_eq!(request.resolution, Resolution::new(1024, 438));
    assert_eq!(request.aspect, aspect);

    let mut sink = Recorder::default();
    let outcome = block_on(gemini.generate(&request, &mut sink, &Cancel::new()));
    let image = outcome.result.unwrap();
    assert_eq!(image.resolution(), Resolution::new(1024, 1024), "read back, not echoed");
    assert_eq!(outcome.usage, ImageUsage::billed(1));
}

#[test]
fn a_backend_that_declares_no_previews_never_sends_one() {
    // The UI reserves a preview surface off `streaming_preview`, so one that is
    // false and then draws is a layout that shifts under the user mid-render —
    // and one that is true and never draws is an empty box for the whole of a
    // paid call.
    let node = subject();
    let extracted = stack(&node);
    let aspect = AspectRatio::parse("1:1").unwrap();

    let gemini = RemoteGemini { ending: Ending::Image };
    let caps = gemini.capabilities("gemini-3-pro-image");
    assert!(!caps.streaming_preview);
    let negotiated = negotiate(&extracted, aspect, &caps);
    let mut sink = Recorder::default();
    block_on(gemini.generate(
        &ImageRequest::new("gemini-3-pro-image", "p", 0, &negotiated),
        &mut sink,
        &Cancel::new(),
    ));
    assert!(sink.previews.is_empty());
    assert_eq!(sink.steps.len(), 1, "still says something, so the bar is not indeterminate");

    let comfy = LocalComfy::new();
    let caps = comfy.capabilities("flux-dev");
    assert!(caps.streaming_preview);
    let negotiated = negotiate(&extracted, aspect, &caps);
    let mut sink = Recorder::default();
    block_on(comfy.generate(
        &ImageRequest::new("flux-dev", "p", 0, &negotiated),
        &mut sink,
        &Cancel::new(),
    ));
    assert_eq!(sink.previews.len(), 4);
    assert_eq!(sink.steps.last().unwrap(), &(4, 4, Some("sampling".to_owned())));
}

#[test]
fn a_cancelled_generation_stops_the_work_rather_than_discarding_the_answer() {
    // The expensive failure, and the one the queue cannot check. A remote render
    // that is abandoned rather than stopped is billed in full; a local one holds
    // a GPU the next job in the queue is waiting for.
    let node = subject();
    let extracted = stack(&node);
    let comfy = LocalComfy::new();
    let caps = comfy.capabilities("flux-dev");
    let negotiated = negotiate(&extracted, AspectRatio::parse("1:1").unwrap(), &caps);

    let cancel = Cancel::new();
    cancel.cancel();
    let outcome = block_on(comfy.generate(
        &ImageRequest::new("flux-dev", "p", 0, &negotiated),
        &mut Discard,
        &cancel,
    ));
    assert!(matches!(outcome.result, Err(Error::Cancelled)));
    assert_eq!(comfy.steps_run.load(Ordering::SeqCst), 0, "not one sampler pass");
    assert!(!outcome.usage.is_billed());
}

#[test]
fn a_failure_after_the_provider_generated_still_reports_what_it_cost() {
    // The reason `ImageOutcome` is not a `Result`. A refusal that arrives after
    // generation has been paid for, and if the usage rode out with the error it
    // would be dropped by the first `?` — leaving #55's ceiling drifting low
    // exactly when the user is hitting limits.
    let node = subject();
    let extracted = stack(&node);
    let gemini = RemoteGemini { ending: Ending::Refused };
    let caps = gemini.capabilities("gemini-3-pro-image");
    let negotiated = negotiate(&extracted, AspectRatio::parse("1:1").unwrap(), &caps);
    let outcome = block_on(gemini.generate(
        &ImageRequest::new("gemini-3-pro-image", "p", 0, &negotiated),
        &mut Discard,
        &Cancel::new(),
    ));
    assert!(outcome.usage.is_billed());
    let error = outcome.result.unwrap_err();
    assert_eq!(error.code(), "provider.bad_response");
    assert!(error.is_retryable(), "the filter is not deterministic, so it could work");

    // Against the failure that happens before anything is generated, which must
    // report zero or the ceiling counts a charge that never happened.
    let gemini = RemoteGemini { ending: Ending::NoBilling };
    let outcome = block_on(gemini.generate(
        &ImageRequest::new("gemini-3-pro-image", "p", 0, &negotiated),
        &mut Discard,
        &Cancel::new(),
    ));
    assert_eq!(outcome.usage, ImageUsage::free());
    assert_eq!(outcome.result.unwrap_err().code(), "provider.billing_required");
}

#[test]
fn the_reference_budget_a_backend_declares_is_the_one_the_request_is_built_to() {
    // The third consequence #50 lists, from outside: the caps come out of the
    // registry #44 owns, the budget cuts what does not fit, and the sentence the
    // Inspector shows is built from the same values the adapter sends.
    let mut kael = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
    for _ in 0..5 {
        kael.asset_links.push(AssetRef::new(new_id(), AssetRole::Costume));
    }
    let extracted = stack(&kael);

    let gemini = RemoteGemini { ending: Ending::Image };
    let caps = gemini.capabilities("gemini-3-pro-image");
    assert_eq!(caps.image_refs.meter(RefBucket::StyleRefs), (RefBucket::StyleRefs, Refs::new(3)));

    let negotiated = negotiate(&extracted, AspectRatio::parse("1:1").unwrap(), &caps);
    let style = negotiated.images().bucket(RefBucket::StyleRefs).unwrap();
    assert_eq!(
        format!("{}/{} {}", style.kept().len(), style.cap().get(), style.bucket().label()),
        "3/3 style refs",
    );

    let request = ImageRequest::new("gemini-3-pro-image", "p", 0, &negotiated)
        .with_references(references(&negotiated));
    assert_eq!(request.in_mechanism(wobu_imagine::ReferenceMechanism::ImagePrompt).count(), 3);
    assert_eq!(request.references.len(), 3, "and nothing else was invented on the way");

    // A local backend has no cap at all, so all five go, and the Inspector has
    // no denominator to print rather than printing `usize::MAX`.
    let comfy = LocalComfy::new();
    let unlimited =
        negotiate(&extracted, AspectRatio::parse("1:1").unwrap(), &comfy.capabilities("flux-dev"));
    let style = unlimited.images().bucket(RefBucket::StyleRefs).unwrap();
    assert_eq!(style.kept().len(), 5);
    assert_eq!(style.cap().limit(), None);
}

#[test]
fn a_backend_is_usable_behind_a_box_dyn_because_that_is_how_a_project_selects_one() {
    // The whole reason for `#[async_trait]`. `project.json` names a backend and
    // the shell holds one of these; a trait that only worked as a concrete type
    // would be a trait nothing could select.
    let backends: Vec<Box<dyn ImageBackend>> =
        vec![Box::new(LocalComfy::new()), Box::new(RemoteGemini { ending: Ending::Image })];
    let ids: Vec<&str> = backends.iter().map(|b| b.id()).collect();
    assert_eq!(ids, ["comfyui", "gemini"]);

    for backend in &backends {
        // Every backend answers for a model it has never heard of, because the
        // Inspector draws a dropdown from this and a project may name a model
        // that has since been removed. `capabilities` returns a value, not an
        // `Option`, so there is no "unknown" branch for a caller to forget.
        let unknown = backend.capabilities("a-model-released-next-month");
        assert!(!unknown.aspect_ratios.is_empty() || !unknown.requires_billing);

        // And a paid backend's answer for it is a real budget rather than
        // `unlimited`: a fourteen-image request to a model that takes six is
        // refused by the provider, after payment. A local backend declares
        // `unlimited` on purpose and by name, which is the case this must not
        // catch.
        if unknown.requires_billing {
            assert_ne!(
                unknown.image_refs,
                ImageBudget::unlimited(),
                "{} handed an unknown paid model an unlimited budget",
                backend.id(),
            );
        }
    }
}
