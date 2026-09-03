//! What a backend can do, declared per model.
//!
//! One struct, filled in by each adapter, read by the UI and by
//! [`negotiate`](crate::negotiate). Nothing in here is advisory: every field
//! either changes what the user is offered or changes what the request carries,
//! and the two consequences are the same fact seen from both ends.
//!
//! See the capability-negotiation section of `docs/08-providers.md`.

use wobu_core::FragmentTarget;
use wobu_influence::{ImageBudget, Refs};

use crate::aspect::{AspectRatio, Resolution};

/// How a backend applies a reference image.
///
/// Deliberately separate from [`wobu_influence::RefBucket`]. A bucket is how a
/// provider *counts* an image; a mechanism is what the adapter *does* with it.
/// Gemini counts poses in its character quota but has no structure mechanism,
/// while a ComfyUI ControlNet graph can take a pose and a silhouette through the
/// same mechanism even though they occupy different provider buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceMechanism {
    /// An ordinary image prompt: Gemini's inline image blocks or ComfyUI's
    /// IPAdapter. The image contributes content or appearance rather than a
    /// spatial constraint.
    ImagePrompt,
    /// A spatial constraint such as pose or silhouette, normally ControlNet in
    /// ComfyUI.
    Structure,
}

impl ReferenceMechanism {
    pub const ALL: [ReferenceMechanism; 2] =
        [ReferenceMechanism::ImagePrompt, ReferenceMechanism::Structure];

    pub fn label(self) -> &'static str {
        match self {
            ReferenceMechanism::ImagePrompt => "image prompt",
            ReferenceMechanism::Structure => "structure",
        }
    }

    /// The mechanism a routed image target needs. Text and private mood-board
    /// targets have none and therefore never consume a mechanism slot.
    pub fn for_target(target: FragmentTarget) -> Option<ReferenceMechanism> {
        match target {
            FragmentTarget::StyleRef | FragmentTarget::Palette => {
                Some(ReferenceMechanism::ImagePrompt)
            }
            FragmentTarget::StructureRef => Some(ReferenceMechanism::Structure),
            FragmentTarget::Prompt | FragmentTarget::Negative | FragmentTarget::MoodboardOnly => {
                None
            }
        }
    }
}

/// How many references one backend can apply through each mechanism.
///
/// These are independent pools. They are not a replacement for
/// [`ImageBudget`]: after a reference survives this mechanism budget it still
/// has to fit the provider's counting buckets. Keeping both axes is what can
/// express a ControlNet graph with one structure input and no image-prompt
/// input without pretending that poses and silhouettes share a vendor bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceMechanisms {
    pub image_prompt: Refs,
    pub structure: Refs,
}

impl ReferenceMechanisms {
    /// The cap for one mechanism.
    pub fn cap(self, mechanism: ReferenceMechanism) -> Refs {
        match mechanism {
            ReferenceMechanism::ImagePrompt => self.image_prompt,
            ReferenceMechanism::Structure => self.structure,
        }
    }

    /// No shipped graph can apply a reference image.
    pub fn none() -> ReferenceMechanisms {
        ReferenceMechanisms { image_prompt: Refs::new(0), structure: Refs::new(0) }
    }

    /// An ordinary multimodal model: references are capped by its provider
    /// quota, not by a second mechanism limit, and structure is unavailable.
    pub fn image_prompt() -> ReferenceMechanisms {
        ReferenceMechanisms { image_prompt: Refs::UNLIMITED, structure: Refs::new(0) }
    }

    /// An unconstrained local test backend that implements both paths.
    pub fn unlimited() -> ReferenceMechanisms {
        ReferenceMechanisms { image_prompt: Refs::UNLIMITED, structure: Refs::UNLIMITED }
    }
}

/// What one backend, running one model, is able to do.
///
/// **Per model, not per backend**, which is the one change from the sketch in
/// [#50](https://github.com/krazyjakee/wobu/issues/50) that the table in
/// `docs/08-providers.md` forces: `gemini-3.1-flash-lite-image` is 1K only and
/// counts fourteen undifferentiated references, `gemini-3-pro-image` goes to 4K
/// and counts six, five and three. One `Capabilities` for "Gemini" would have to
/// be the worst of all three, which would hide two thirds of the reference
/// budget the user is paying for. So [`capabilities`] takes the model id.
///
/// Every field is public and there is no constructor. This is a declaration in
/// the same sense `wobu_core::Preset` is — an adapter writes it out once, a
/// reviewer reads it against the provider's documentation, and there is nothing
/// in between for a default to hide in.
///
/// [`capabilities`]: crate::ImageBackend::capabilities
#[derive(Debug, Clone, PartialEq)]
pub struct Capabilities {
    /// The largest image this model will produce. Not a suggestion: an aspect
    /// ratio is fitted inside it ([`AspectRatio::fit`]), so this is what decides
    /// the pixel dimensions of every request.
    pub max_resolution: Resolution,

    /// Which shapes it takes, in the order the dropdown should list them.
    ///
    /// A `Vec` and not a fixed set because ComfyUI's answer is computed — it
    /// will generate any dimensions its checkpoint tolerates — while Gemini's is
    /// a documented list. Order is the backend's preference and is used to break
    /// ties in [`nearest_aspect`](Self::nearest_aspect), so it must not be
    /// sorted for display anywhere else.
    ///
    /// Empty means the model does not take an aspect as a parameter at all,
    /// which is a real state (`docs/08-providers.md` flags credible reports of
    /// Gemini ignoring `aspect_ratio`) and is treated as "whatever you asked for
    /// passes through" rather than "every shape is refused" — a backend that
    /// refused every shape could produce no image and nobody would declare one.
    pub aspect_ratios: Vec<AspectRatio>,

    /// How many reference images the provider counts in each of its buckets.
    ///
    /// Deliberately **not** a map keyed by `wobu_core::AssetRole`, which is what
    /// [#50](https://github.com/krazyjakee/wobu/issues/50) originally sketched.
    /// [#44](https://github.com/krazyjakee/wobu/issues/44) settled this: the
    /// caps are declared in the vocabulary the provider counts in — objects,
    /// characters, style refs — and `wobu-influence` owns the single mapping
    /// from our seven roles onto those three buckets. Keying this by `AssetRole`
    /// would push that judgement into every adapter and let two of them disagree
    /// about which pool a `pose` reference competes in.
    ///
    /// The registry behind it is `wobu_influence::image_budget`, and an adapter
    /// must read this from there rather than restating the numbers: a model
    /// released next month is a row in that table, and a second copy here is a
    /// second thing to forget. A model the registry has never heard of gets the
    /// adapter's most conservative *registered* budget and never
    /// [`ImageBudget::unlimited`] — building a fourteen-image request for a
    /// model that takes six lets the provider be the one to point it out, after
    /// payment. `unlimited` is for a local backend that genuinely has no cap,
    /// and it is asked for by name.
    pub image_refs: ImageBudget,

    /// Which reference paths this adapter can actually reach, and how many
    /// inputs each path has. This is the routing axis; [`image_refs`](Self::image_refs)
    /// remains the provider's independent counting axis.
    pub reference_mechanisms: ReferenceMechanisms,

    /// Whether the backend takes named fine-tunes stacked on the checkpoint.
    ///
    /// ComfyUI-forced, and it is the one field with no negotiation behind it:
    /// nothing the influence stack produces routes to a LoRA, so there is
    /// nothing here to downgrade. It exists because the UI must not show a LoRA
    /// picker for a backend that has none, and a picker that silently did
    /// nothing would be the same failure as a silently dropped reference. When
    /// something in the stack does start asking for one, the answer for a
    /// backend without them is a [`Downgrade`](crate::Downgrade) variant, not a
    /// shrug.
    pub loras: bool,

    /// Whether the backend takes a negative prompt.
    ///
    /// Not in the issue's field list, and added because without it the
    /// negotiation is not total. `never:` is the one section every kind is
    /// required to declare (`wobu-core`), it compiles to
    /// `FragmentTarget::Negative`, and Gemini's image API has no field to put it
    /// in — Imagen's `negativePrompt` is gone with Imagen. Without this field
    /// the only possible behaviour is to drop user-authored canon in silence,
    /// which is the exact failure this issue exists to prevent. ComfyUI has one,
    /// so this is Gemini-forced.
    pub negative_prompt: bool,

    /// Whether a call to this backend, with this model, moves money.
    ///
    /// What a failed job reads to decide whether an attempt that produced
    /// nothing may still have been charged for. Three things it deliberately
    /// does not mean:
    ///
    /// - **Not "is the account able to pay".** A key whose account has no
    ///   billing enabled is `Error::BillingRequired` at call time; it cannot be
    ///   known when capabilities are read.
    /// - **Not "is this free".** A local ComfyUI declares `false` and still
    ///   costs the user electricity and twenty minutes of their GPU. `false`
    ///   means *no money moves at the provider*, which is the only question
    ///   this flag answers.
    /// - **Not a price.** Prices are per model, per size, and they move
    ///   (`docs/08-providers.md`); a number here would be wrong the week after
    ///   somebody changed a model id.
    ///
    /// For ComfyUI it is `false` on every model, and `docs/08-providers.md` says
    /// why that matters: "Local ComfyUI shows no cost — that asymmetry is the
    /// point, and it's a good default."
    pub requires_billing: bool,

    /// Whether images of the work in flight arrive before it finishes.
    ///
    /// `true` for ComfyUI, which sends latent previews per sampler step over its
    /// websocket; `false` for Gemini, which returns one inline base64 image and
    /// nothing before it. It is a promise in both directions: a backend that
    /// declares `false` must never call
    /// [`ProgressSink::preview`](crate::ProgressSink::preview), so the UI can
    /// decline to reserve a preview surface rather than showing an empty one for
    /// the whole of a paid call.
    pub streaming_preview: bool,
}

impl Capabilities {
    /// Whether this shape appears in the dropdown.
    ///
    /// The negative of this is the second consequence the issue asks for:
    /// "aspect ratios the backend doesn't support don't appear in the dropdown".
    /// An empty [`aspect_ratios`](Self::aspect_ratios) supports everything,
    /// because it means the parameter is not taken rather than that every value
    /// is refused.
    pub fn supports_aspect(&self, aspect: AspectRatio) -> bool {
        self.aspect_ratios.is_empty() || self.aspect_ratios.contains(&aspect)
    }

    /// The shape this backend will actually produce when asked for `aspect`.
    ///
    /// Returns `aspect` unchanged when it is supported, and otherwise the
    /// closest thing offered, by [`AspectRatio::distance`]. Total on purpose:
    /// a preset's aspect is fixed by the preset and the user did not choose it,
    /// so refusing to generate a character sheet because a backend has no `3:4`
    /// would be a dead end with no lever. The substitution is reported —
    /// [`Negotiated::requested_aspect`](crate::Negotiated::requested_aspect) —
    /// which is what keeps this from being a silent change.
    ///
    /// Ties go to the earlier entry, so declaration order is a preference and
    /// not decoration: a backend that lists `4:3` before `3:2` is saying which
    /// it would rather be asked for.
    pub fn nearest_aspect(&self, aspect: AspectRatio) -> AspectRatio {
        if self.supports_aspect(aspect) {
            return aspect;
        }
        self.aspect_ratios
            .iter()
            .copied()
            // `total_cmp` and not `partial_cmp().unwrap()`: a NaN cannot arise
            // from two positive finite ratios today, but this is the last thing
            // between a preset and a request and it must not be able to panic.
            .min_by(|a, b| aspect.distance(*a).total_cmp(&aspect.distance(*b)))
            .unwrap_or(aspect)
    }

    /// The pixel dimensions a request for `aspect` gets: the largest image of
    /// the shape this backend will actually produce that fits under its ceiling.
    pub fn resolution_for(&self, aspect: AspectRatio) -> Resolution {
        self.nearest_aspect(aspect).fit(self.max_resolution)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wobu_influence::{RefBucket, Refs, image_budget};

    /// Gemini's pro image model, as `docs/08-providers.md` describes it: paid,
    /// no structure adapter, no LoRAs, no negative prompt, no previews.
    fn remote() -> Capabilities {
        Capabilities {
            max_resolution: Resolution::new(4096, 4096),
            aspect_ratios: AspectRatio::ALL.to_vec(),
            image_refs: image_budget("gemini-3-pro-image").unwrap(),
            reference_mechanisms: ReferenceMechanisms::image_prompt(),
            loras: false,
            negative_prompt: false,
            requires_billing: true,
            streaming_preview: false,
        }
    }

    /// A local ComfyUI: everything the remote one has not, and none of the cost.
    fn local() -> Capabilities {
        Capabilities {
            max_resolution: Resolution::new(2048, 2048),
            aspect_ratios: vec![],
            image_refs: ImageBudget::unlimited(),
            reference_mechanisms: ReferenceMechanisms::unlimited(),
            loras: true,
            negative_prompt: true,
            requires_billing: false,
            streaming_preview: true,
        }
    }

    #[test]
    fn a_shape_the_backend_does_not_offer_is_not_in_its_dropdown() {
        // The second of the three consequences #50 lists. A ratio left in the
        // dropdown that the backend does not take is either a late failure or,
        // per `docs/08-providers.md`, silently ignored and returned as a square.
        let narrow =
            Capabilities { aspect_ratios: vec![AspectRatio::parse("1:1").unwrap()], ..remote() };
        assert!(narrow.supports_aspect(AspectRatio::parse("1:1").unwrap()));
        assert!(!narrow.supports_aspect(AspectRatio::parse("21:9").unwrap()));
        assert!(remote().supports_aspect(AspectRatio::parse("21:9").unwrap()));
    }

    #[test]
    fn a_backend_that_takes_no_aspect_parameter_offers_every_shape() {
        // Not the same as refusing every shape, which is what an empty list
        // reads as if nobody says otherwise — and a backend that refused every
        // shape could produce no image at all, so nothing would ever declare it.
        // ComfyUI is this case: it takes width and height, not a ratio.
        let comfy = local();
        for aspect in AspectRatio::ALL {
            assert!(comfy.supports_aspect(aspect), "{aspect}");
            assert_eq!(comfy.nearest_aspect(aspect), aspect, "{aspect} must pass through");
        }
    }

    #[test]
    fn the_nearest_shape_is_nearest_and_ties_follow_the_backend_s_own_order() {
        // A preset's aspect is fixed by the preset, so this substitution is the
        // difference between a working Generate button and a dead one. Getting
        // it wrong is worse than a dead button: an environment matte silently
        // rendered square is a wrong picture nothing on screen explains.
        let wide = AspectRatio::parse("21:9").unwrap();
        let backend = Capabilities {
            aspect_ratios: vec![
                AspectRatio::parse("1:1").unwrap(),
                AspectRatio::parse("16:9").unwrap(),
                AspectRatio::parse("3:4").unwrap(),
            ],
            ..remote()
        };
        assert_eq!(backend.nearest_aspect(wide), AspectRatio::parse("16:9").unwrap());

        // Ties: `4:3` and `3:4` are the same distance from square in log space,
        // and the backend listed `4:3` first. Resolved any other way, the same
        // request would produce a landscape on one build and a portrait on the
        // next, and an `influence_snapshot` would stop reproducing.
        let square = AspectRatio::parse("1:1").unwrap();
        let either = Capabilities {
            aspect_ratios: vec![
                AspectRatio::parse("4:3").unwrap(),
                AspectRatio::parse("3:4").unwrap(),
            ],
            ..remote()
        };
        assert_eq!(either.nearest_aspect(square), AspectRatio::parse("4:3").unwrap());
    }

    #[test]
    fn the_resolution_is_the_ceiling_at_the_shape_that_will_actually_be_produced() {
        // Not at the shape that was asked for. A `21:9` request on a backend
        // that only does `16:9` must be sized `16:9`, or the request carries a
        // ratio and a set of dimensions that disagree and the backend picks one.
        let backend =
            Capabilities { aspect_ratios: vec![AspectRatio::parse("16:9").unwrap()], ..remote() };
        let wide = AspectRatio::parse("21:9").unwrap();
        assert_eq!(backend.resolution_for(wide), Resolution::new(4096, 2304));
        assert_eq!(local().resolution_for(wide), Resolution::new(2048, 877));
    }

    #[test]
    fn the_reference_caps_are_the_registry_s_and_are_never_restated_here() {
        // #44's line, held from this side: `image_refs` is a value read out of
        // `wobu_influence::image_budget`, so a model released next month is a
        // row in that table and not an edit to an adapter. A `Capabilities` that
        // spelled the numbers out would be a second copy, and the copy that is
        // wrong is the one that builds a fourteen-image request for a model that
        // takes six.
        let pro = remote().image_refs;
        assert_eq!(pro, image_budget("gemini-3-pro-image").unwrap());
        assert_eq!(pro.meter(RefBucket::StyleRefs), (RefBucket::StyleRefs, Refs::new(3)));

        // And the local backend asks for `unlimited` by name rather than getting
        // it by accident from an unregistered model id.
        assert_eq!(local().image_refs, ImageBudget::unlimited());
        assert_eq!(local().image_refs.meter(RefBucket::StyleRefs).1.limit(), None);
    }

    #[test]
    fn reference_mechanisms_route_targets_and_do_not_restate_provider_caps() {
        assert_eq!(
            ReferenceMechanism::for_target(FragmentTarget::StyleRef),
            Some(ReferenceMechanism::ImagePrompt),
        );
        assert_eq!(
            ReferenceMechanism::for_target(FragmentTarget::Palette),
            Some(ReferenceMechanism::ImagePrompt),
        );
        assert_eq!(
            ReferenceMechanism::for_target(FragmentTarget::StructureRef),
            Some(ReferenceMechanism::Structure),
        );
        assert_eq!(ReferenceMechanism::for_target(FragmentTarget::MoodboardOnly), None);

        let gemini = remote();
        assert_eq!(gemini.reference_mechanisms.image_prompt.limit(), None);
        assert_eq!(gemini.reference_mechanisms.structure, Refs::new(0));
        assert_eq!(gemini.image_refs.style_refs, Some(Refs::new(3)));
    }

    #[test]
    fn only_a_remote_backend_requires_billing() {
        // `requires_billing` is what tells a failed job whether the provider
        // may have charged for an attempt that produced nothing.
        assert!(remote().requires_billing);
        assert!(!local().requires_billing);
    }
}
