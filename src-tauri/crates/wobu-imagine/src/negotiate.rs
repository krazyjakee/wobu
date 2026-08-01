//! Fitting what the influence stack wants onto what the backend offers, with a
//! defined answer for every combination of the two.
//!
//! The rule this module exists to hold is the one
//! [#44](https://github.com/krazyjakee/wobu/issues/44) was built around and the
//! one `docs/04-influence-engine.md` states outright: **silently discarding
//! something the user deliberately attached is the worst thing this engine could
//! do.** So negotiation never drops anything quietly. Either the thing is sent,
//! or it is withheld and reported as a [`Downgraded`] carrying the fragment that
//! lost — which is what lets the Inspector put the sentence on the card the
//! reference came from rather than in a log nobody reads.
//!
//! ## Why this runs before the budget rather than after it
//!
//! [`negotiate`] withholds what the backend cannot honour *first*, and then
//! hands the survivors to `wobu_influence::compile_images`. The order matters and
//! is the subtle part: on a backend with no structure adapter, a `pose` reference
//! is not going to be sent, so it must not be counted against the character
//! bucket on the way out. Budgeting first would let a reference that was never
//! going to leave the machine evict one that was, and the evicted one would be
//! reported as a casualty of a budget it did not really lose.
//!
//! It is the same argument `compile_images` already makes about mood-board
//! references — "a mood reference occupies no slot, so attaching one can never
//! be what costs a real reference its place" — applied to the references this
//! backend turns *into* mood-board references.

use wobu_core::FragmentTarget;
use wobu_influence::{CompiledImages, Fragment, compile_images};

use serde::Serialize;

use crate::aspect::{AspectRatio, Resolution};
use crate::capability::Capabilities;

/// Why something the stack offered is not in the request.
///
/// Serialisable and small, because #46 puts it over the bridge and #47 draws it
/// on a layer card. It is deliberately not a string: the UI's wording for "your
/// silhouette reference is mood-board only on this backend" is a product
/// decision that will be rewritten, and rewriting it should not be a change to a
/// Rust crate.
///
/// Each variant has exactly one cause today, which is why
/// [`label`](Self::label) can be a single sentence per variant. A second cause
/// for one of them means splitting the variant rather than widening the
/// sentence, because the sentence is what tells the user which control to go and
/// use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Downgrade {
    /// A structure reference — `silhouette` or `pose` — on a backend with no
    /// structure adapter. It stays in the project and stays on screen; it is
    /// simply never sent, which is what a `mood` reference already is.
    ///
    /// This is the first of the three consequences #50 lists, and the reason it
    /// is data rather than a log line: "a backend with no ControlNet shows
    /// structure references as visibly downgraded to mood-board-only".
    ///
    /// Sending it as an ordinary reference image instead would not be a kinder
    /// version of the same thing. A silhouette handed to a model that cannot use
    /// it as structure is a picture of a black shape on white, and the model
    /// draws one.
    MoodboardOnly,
    /// A negative-prompt fragment on a backend with no negative prompt. There is
    /// nothing it downgrades *to* — Gemini's image API has no field for it — so
    /// the honest answer is that the `never:` list is not being enforced on this
    /// backend, said once, rather than a "without X" clause bolted onto the
    /// positive prompt where it reads as a request for X.
    NotSent,
}

impl Downgrade {
    /// What the Inspector says about it, in one clause.
    pub fn label(self) -> &'static str {
        match self {
            Downgrade::MoodboardOnly => "mood-board only — this backend cannot use it as structure",
            Downgrade::NotSent => "not sent — this backend takes no negative prompt",
        }
    }
}

/// One fragment the backend cannot honour, and why.
///
/// Carries the whole [`Fragment`] for the same reason `wobu_influence::Dropped`
/// does: a fragment already knows its layer and its source node and refuses to
/// be built without them, and a report that named a section but not the card it
/// came from would be a list the user cannot act on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Downgraded<'a> {
    pub fragment: Fragment<'a>,
    pub reason: Downgrade,
}

/// What one fragment's routing target becomes on a given backend.
///
/// Private, and the point of it is the exhaustive `match` in [`route`]: a
/// seventh `FragmentTarget` cannot be added in `wobu-core` without this file
/// failing to compile, which is the only mechanism that makes "negotiation is
/// total" a property rather than an intention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// Goes into the request.
    Send,
    /// Does not go, and the user is told.
    Withheld(Downgrade),
    /// Was never going to be sent, on any backend. Not a downgrade and not a
    /// casualty: it is doing exactly what it was attached to do.
    Private,
}

/// Where a fragment aimed at `target` ends up on a backend with these caps.
///
/// `Private` is stated here rather than left to `Fragment::is_sendable` so the
/// match is exhaustive over the enum, and
/// `routing_agrees_with_wobu_core_about_which_fragments_are_private` pins the
/// two together rather than letting this become a second list of what is
/// private — two such lists are one rename away from disagreeing, and the
/// direction they fail in is somebody's mood board on a third party's servers.
fn route(target: FragmentTarget, caps: &Capabilities) -> Route {
    match target {
        FragmentTarget::Prompt => Route::Send,
        FragmentTarget::Negative => {
            if caps.negative_prompt { Route::Send } else { Route::Withheld(Downgrade::NotSent) }
        }
        // Style transfer is the one image channel every backend has in some
        // form: it is "here is a picture, make it look like this", which is what
        // a reference image *is* on a model with no adapters at all. There is no
        // capability for it and there should not be one — a backend that could
        // not take a reference image would have an empty `image_refs`, and the
        // budget already answers that by keeping nothing.
        FragmentTarget::StyleRef => Route::Send,
        FragmentTarget::StructureRef => {
            if caps.controlnet { Route::Send } else { Route::Withheld(Downgrade::MoodboardOnly) }
        }
        // Colour conditioning has no capability flag either, and that is #44's
        // call rather than a gap here: `RefBucket::for_role` files a `palette`
        // reference in the general object bucket, so on a backend with no colour
        // pass it is sent as an ordinary picture of some colours. It is weaker
        // than a palette adapter and it is not nothing, which is the difference
        // between this and a structure reference.
        FragmentTarget::Palette => Route::Send,
        FragmentTarget::MoodboardOnly => Route::Private,
    }
}

/// What one generation will actually ask for, and what it cost to get there.
///
/// The fields are private and [`negotiate`] is the only constructor, for the
/// same reason `wobu_influence::CompiledPrompt`'s are: the request and the
/// report of what the backend could not honour are one answer to one question. A
/// value that could be built from an aspect ratio alone would be one that claims
/// nothing was downgraded.
#[derive(Debug, Clone, PartialEq)]
pub struct Negotiated<'a> {
    requested_aspect: AspectRatio,
    aspect: AspectRatio,
    resolution: Resolution,
    fragments: Vec<Fragment<'a>>,
    images: CompiledImages<'a>,
    downgrades: Vec<Downgraded<'a>>,
}

impl<'a> Negotiated<'a> {
    /// The shape the request will carry.
    pub fn aspect(&self) -> AspectRatio {
        self.aspect
    }

    /// The shape the preset asked for, when it is not the one being used.
    ///
    /// `None` when nothing was substituted, so `if let Some(wanted)` is the
    /// whole of the UI's check. An aspect substitution is kept apart from
    /// [`downgrades`](Self::downgrades) because it is a different kind of fact
    /// and lands in a different place on screen: a parameter that changed,
    /// shown next to the dropdown, rather than a fragment that was withheld,
    /// shown on the card it came from.
    pub fn requested_aspect(&self) -> Option<AspectRatio> {
        (self.aspect != self.requested_aspect).then_some(self.requested_aspect)
    }

    /// The pixel dimensions: the largest image of [`aspect`](Self::aspect) that
    /// fits under the backend's ceiling.
    pub fn resolution(&self) -> Resolution {
        self.resolution
    }

    /// Everything that survived, in reading order, ready for
    /// `wobu_influence::compile`.
    ///
    /// **This, and not the slice that went in, is what the prompts are compiled
    /// from.** It is what makes the negative-prompt downgrade real rather than
    /// advisory: on a backend with no negative prompt the `never:` fragments are
    /// not in here, so there is no path by which a compiled negative could reach
    /// a request that cannot carry one. Nothing has to remember to check a flag.
    ///
    /// Mood-board fragments are still here. They were not downgraded — they were
    /// already private — and `compile` and `compile_images` both filter them on
    /// `Fragment::is_sendable` anyway, so removing them here would only cost the
    /// Inspector the rows it draws from this list.
    pub fn fragments(&self) -> &[Fragment<'a>] {
        &self.fragments
    }

    /// The reference images, in the buckets the backend counts them in, with the
    /// account of what did not fit.
    ///
    /// This is the third consequence #50 lists — "per-role reference caps drive
    /// the image budget, so the Inspector can say `3/3 style refs`" — and the
    /// sentence is built from `Bucket::kept().len()`, `Bucket::cap()` and
    /// `RefBucket::label()`, exactly as `wobu-influence`'s own doc example does.
    pub fn images(&self) -> &CompiledImages<'a> {
        &self.images
    }

    /// Every fragment the backend could not honour, in reading order.
    ///
    /// Disjoint from `self.images().dropped()` and the two must not be merged: a
    /// reference here was withheld because of what the *backend* is, and one
    /// there lost a place because of what the *stack* asked for. The first is
    /// fixed by choosing a different backend and the second by raising a layer
    /// card, so a single list would send half its readers to the wrong control.
    pub fn downgrades(&self) -> &[Downgraded<'a>] {
        &self.downgrades
    }

    /// Whether anything at all had to give. What the Generate panel checks
    /// before deciding whether to show a "this backend will do less than you
    /// asked" affordance at all.
    pub fn is_exact(&self) -> bool {
        self.downgrades.is_empty() && self.requested_aspect().is_none()
    }
}

/// Reconcile a resolved, extracted stack with what a backend can do.
///
/// Takes the same slice `wobu_influence::fragments` returns that `compile` and
/// `compile_images` do — unsorted, unfiltered, mood-board entries included — and
/// answers, for every one of them, whether this backend will receive it. Then it
/// budgets the survivors, settles the aspect ratio, and sizes the image.
///
/// Total by construction, in both directions:
///
/// - Over what the stack wants, because [`route`] matches every
///   `FragmentTarget` exhaustively and the compiler will not let a new one
///   through.
/// - Over what the backend offers, because every capability that can be absent
///   has a stated answer for its absence and none of those answers is "drop it".
///   A backend that declared nothing at all — no ratios, no structure, no
///   negative prompt, an empty reference budget — still produces a request, and
///   still produces the list of everything it could not take.
///
/// `aspect` is the preset's, parsed from `Preset::aspect`. It is separate from
/// the fragments because it is not one: a preset's framing text is a fragment
/// and its aspect is a parameter, and `docs/04-influence-engine.md` keeps them
/// apart for the same reason.
pub fn negotiate<'a>(
    fragments: &[Fragment<'a>],
    aspect: AspectRatio,
    caps: &Capabilities,
) -> Negotiated<'a> {
    let mut kept: Vec<Fragment<'a>> = Vec::with_capacity(fragments.len());
    let mut downgrades: Vec<Downgraded<'a>> = Vec::new();

    for fragment in fragments {
        match route(fragment.target(), caps) {
            Route::Send | Route::Private => kept.push(*fragment),
            Route::Withheld(reason) => {
                downgrades.push(Downgraded { fragment: *fragment, reason })
            }
        }
    }

    // After the withholding and never before it — see the module header. A
    // reference the backend is not going to receive must not be counted against
    // a bucket on its way out, or it evicts one that would have been sent.
    let images = compile_images(&kept, caps.image_refs);
    let settled = caps.nearest_aspect(aspect);

    Negotiated {
        requested_aspect: aspect,
        aspect: settled,
        resolution: settled.fit(caps.max_resolution),
        fragments: kept,
        images,
        downgrades,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aspect::Resolution;
    use wobu_core::{AssetRole, Id, Layer};
    use wobu_influence::{
        FragmentBody, ImageBudget, Origin, Reached, RefBucket, ResolvedSource, image_budget,
    };

    fn source(layer: Layer) -> ResolvedSource<'static> {
        ResolvedSource {
            layer,
            origin: Origin::Shot("Character sheet · 3:4"),
            reached: Reached::Shot,
            distance: 0,
            weight: 1.0,
        }
    }

    fn reference(layer: Layer, role: AssetRole, weight: f32) -> Fragment<'static> {
        Fragment::new(
            &source(layer),
            role.as_str(),
            FragmentBody::Asset { id: Id::nil(), role },
            weight,
            role.target(),
        )
    }

    fn text(section: &'static str, body: &'static str, target: FragmentTarget) -> Fragment<'static> {
        Fragment::new(&source(Layer::Subject), section, FragmentBody::Text(body), 1.0, target)
    }

    fn remote() -> Capabilities {
        Capabilities {
            max_resolution: Resolution::new(4096, 4096),
            aspect_ratios: AspectRatio::ALL.to_vec(),
            image_refs: image_budget("gemini-3-pro-image").unwrap(),
            controlnet: false,
            loras: false,
            negative_prompt: false,
            requires_billing: true,
            streaming_preview: false,
        }
    }

    fn local() -> Capabilities {
        Capabilities {
            max_resolution: Resolution::new(2048, 2048),
            aspect_ratios: vec![],
            image_refs: ImageBudget::unlimited(),
            controlnet: true,
            loras: true,
            negative_prompt: true,
            requires_billing: false,
            streaming_preview: true,
        }
    }

    fn aspect(text: &str) -> AspectRatio {
        AspectRatio::parse(text).unwrap()
    }

    #[test]
    fn a_backend_with_no_structure_adapter_downgrades_those_references_to_moodboard_only() {
        // The first consequence #50 exists for, and the whole reason it is
        // modelled as data: the alternative is a log line the user never sees,
        // followed by art that is missing the pose they attached with nothing on
        // screen to explain it.
        let stack = [
            reference(Layer::Subject, AssetRole::Pose, 1.0),
            reference(Layer::Subject, AssetRole::Silhouette, 1.0),
            reference(Layer::Style, AssetRole::Material, 1.0),
        ];

        let downgraded = negotiate(&stack, aspect("3:4"), &remote());
        let withheld: Vec<_> = downgraded
            .downgrades()
            .iter()
            .map(|d| (d.fragment.section(), d.reason))
            .collect();
        assert_eq!(withheld, [
            ("pose", Downgrade::MoodboardOnly),
            ("silhouette", Downgrade::MoodboardOnly),
        ]);
        assert!(!downgraded.is_exact());
        assert_eq!(downgraded.images().kept().count(), 1, "only the material reference is sent");

        // And the same stack on a backend that has one sends all three, with
        // nothing to report. Two backends, one stack, two visibly different
        // answers is the acceptance criterion.
        let full = negotiate(&stack, aspect("3:4"), &local());
        assert!(full.downgrades().is_empty());
        assert_eq!(full.images().kept().count(), 3);
        assert!(full.is_exact());
    }

    #[test]
    fn a_withheld_structure_reference_does_not_cost_a_sent_one_its_slot() {
        // The ordering regression, and the expensive one. `gemini-3-pro-image`
        // takes three style refs; four are offered, and two `pose` references
        // are attached that this backend will never receive. Budget first and
        // those two are metered against the character bucket — harmless here —
        // but the general failure is a reference that is not going anywhere
        // evicting one that is, and the evicted one is then reported as a
        // casualty of a budget it did not really lose.
        let mut stack: Vec<Fragment<'static>> = Vec::new();
        for _ in 0..2 {
            stack.push(reference(Layer::Subject, AssetRole::Pose, 1.0));
        }
        for weight in [0.2, 0.4, 0.6, 0.8] {
            stack.push(reference(Layer::Style, AssetRole::Material, weight));
        }

        let negotiated = negotiate(&stack, aspect("3:4"), &remote());
        let style = negotiated.images().bucket(RefBucket::StyleRefs).unwrap();
        assert_eq!(style.kept().len(), 3, "3/3 style refs");
        assert_eq!(style.dropped().len(), 1, "the lightest style ref, and only it");
        assert_eq!(style.dropped()[0].fragment.weight(), 0.2);

        // The two poses are downgrades, not budget casualties, and they are in
        // neither the character bucket nor the drop report. A structure
        // reference on this backend is a mood-board reference, and
        // `compile_images` is explicit that a mood-board reference "occupies no
        // slot" and "is not in the report at all".
        assert_eq!(negotiated.downgrades().len(), 2);
        assert_eq!(
            negotiated.images().bucket(RefBucket::Characters).unwrap().kept().len(),
            0,
            "nothing this backend will not receive may hold a character slot",
        );
        assert_eq!(negotiated.images().dropped().count(), 1);
    }

    #[test]
    fn the_inspector_can_say_three_of_three_style_refs() {
        // The third consequence #50 lists, spelled as the sentence itself so
        // that a change to `Bucket`'s accessors or to `RefBucket::label` shows
        // up here as prose rather than as a compile error somebody papers over.
        let stack: Vec<Fragment<'static>> =
            (0..5).map(|_| reference(Layer::Style, AssetRole::Costume, 1.0)).collect();
        let negotiated = negotiate(&stack, aspect("3:4"), &remote());
        let style = negotiated.images().bucket(RefBucket::StyleRefs).unwrap();
        assert_eq!(
            format!("{}/{} {}", style.kept().len(), style.cap().get(), style.bucket().label()),
            "3/3 style refs",
        );

        // On the local backend there is no denominator to print, and the
        // Inspector has to be able to tell that apart from a very large one.
        let unlimited = negotiate(&stack, aspect("3:4"), &local());
        let style = unlimited.images().bucket(RefBucket::StyleRefs).unwrap();
        assert_eq!(style.cap().limit(), None);
        assert_eq!(style.kept().len(), 5);
    }

    #[test]
    fn a_backend_with_no_negative_prompt_says_so_instead_of_dropping_the_never_section() {
        // `never:` is the one section every kind is required to declare, it is
        // canon the user wrote, and Gemini's image API has nowhere to put it.
        // Without this the only possible behaviour is to lose it in silence.
        let stack = [
            text("silhouette", "tall, narrow, hooded", FragmentTarget::Prompt),
            text("never", "modern firearms", FragmentTarget::Negative),
        ];

        let negotiated = negotiate(&stack, aspect("3:4"), &remote());
        assert_eq!(negotiated.downgrades().len(), 1);
        assert_eq!(negotiated.downgrades()[0].reason, Downgrade::NotSent);
        assert_eq!(negotiated.downgrades()[0].fragment.section(), "never");

        // And the enforcement is structural rather than a flag somebody has to
        // check: the fragment is not in what the prompts are compiled from, so
        // there is no route by which a negative could reach a request that
        // cannot carry one.
        let sections: Vec<_> = negotiated.fragments().iter().map(|f| f.section()).collect();
        assert_eq!(sections, ["silhouette"]);

        let local = negotiate(&stack, aspect("3:4"), &local());
        assert!(local.downgrades().is_empty());
        assert_eq!(local.fragments().len(), 2);
    }

    #[test]
    fn an_aspect_the_backend_will_not_produce_is_substituted_and_reported() {
        // A preset's aspect is fixed by the preset and the user did not pick it,
        // so refusing outright would be a dead Generate button with no lever.
        // Substituting in silence would be an environment matte that comes back
        // square for a reason nothing on screen explains.
        let backend = Capabilities { aspect_ratios: vec![aspect("16:9")], ..remote() };
        let negotiated = negotiate(&[], aspect("21:9"), &backend);
        assert_eq!(negotiated.aspect(), aspect("16:9"));
        assert_eq!(negotiated.requested_aspect(), Some(aspect("21:9")));
        assert_eq!(negotiated.resolution(), Resolution::new(4096, 2304));
        assert!(!negotiated.is_exact(), "a substituted parameter is not an exact match");

        // Nothing to say when nothing changed. `requested_aspect` is an
        // `Option` precisely so the UI's check is the presence of a value
        // rather than an inequality it has to remember to write.
        let exact = negotiate(&[], aspect("16:9"), &backend);
        assert_eq!(exact.requested_aspect(), None);
        assert!(exact.is_exact());
    }

    #[test]
    fn routing_agrees_with_wobu_core_about_which_fragments_are_private() {
        // The privacy property at the negotiation layer, stated as an
        // equivalence with `Fragment::is_sendable` rather than as a second list
        // of what is private. Two such lists are one rename away from
        // disagreeing, and the direction they fail in is somebody's mood board
        // on a third party's servers.
        for role in AssetRole::ALL {
            let fragment = reference(Layer::Subject, role, 1.0);
            let private = route(fragment.target(), &local()) == Route::Private;
            assert_eq!(!private, fragment.is_sendable(), "{role}");
        }
        assert_eq!(route(FragmentTarget::MoodboardOnly, &local()), Route::Private);
        assert_eq!(route(FragmentTarget::MoodboardOnly, &remote()), Route::Private);
    }

    #[test]
    fn a_moodboard_reference_is_never_reported_as_a_casualty() {
        // It is doing exactly what it was attached to do. Reporting it would
        // send the user off to fix something that is not broken, and — on the
        // backend with no structure adapter — would be indistinguishable from
        // the pose reference next to it that genuinely did lose something.
        let stack = [
            reference(Layer::Subject, AssetRole::Mood, 1.0),
            reference(Layer::Subject, AssetRole::Pose, 1.0),
        ];
        for caps in [local(), remote()] {
            let negotiated = negotiate(&stack, aspect("3:4"), &caps);
            assert!(
                !negotiated.downgrades().iter().any(|d| d.fragment.section() == "mood"),
                "a mood reference was reported as a downgrade",
            );
            assert_eq!(negotiated.images().dropped().count(), 0);
        }
    }

    #[test]
    fn a_backend_that_offers_nothing_still_produces_a_request_and_a_full_account() {
        // Totality, from the other end. Every capability off at once is not a
        // configuration anyone would ship, and it is the case that proves there
        // is no combination with an undefined answer — which is the property
        // that stops the next adapter from inventing one.
        let bare = Capabilities {
            max_resolution: Resolution::new(512, 512),
            aspect_ratios: vec![aspect("1:1")],
            image_refs: ImageBudget {
                objects: wobu_influence::Refs::new(0),
                characters: None,
                style_refs: None,
            },
            controlnet: false,
            loras: false,
            negative_prompt: false,
            requires_billing: false,
            streaming_preview: false,
        };
        let stack = [
            text("silhouette", "tall", FragmentTarget::Prompt),
            text("never", "modern firearms", FragmentTarget::Negative),
            reference(Layer::Subject, AssetRole::Pose, 1.0),
            reference(Layer::Subject, AssetRole::Mood, 1.0),
            reference(Layer::Style, AssetRole::Material, 1.0),
            reference(Layer::Subject, AssetRole::Palette, 1.0),
        ];

        let negotiated = negotiate(&stack, aspect("21:9"), &bare);
        assert_eq!(negotiated.aspect(), aspect("1:1"));
        assert_eq!(negotiated.resolution(), Resolution::new(512, 512));
        assert_eq!(negotiated.images().kept().count(), 0, "a zero budget keeps nothing");

        // Six fragments in, and every one of them accounted for exactly once:
        // one prompt fragment sent, one mood reference doing its job, two
        // withheld by the backend, two dropped by a budget of zero.
        assert_eq!(negotiated.downgrades().len(), 2);
        assert_eq!(negotiated.images().dropped().count(), 2);
        let accounted = negotiated.downgrades().len()
            + negotiated.images().dropped().count()
            + negotiated.images().kept().count()
            + 2; // the positive prompt fragment, and the mood reference
        assert_eq!(accounted, stack.len(), "a fragment went missing between the two reports");
    }
}
