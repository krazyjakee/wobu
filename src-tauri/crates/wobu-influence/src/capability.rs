//! How many reference images a backend takes, and which of its buckets each of
//! our roles is counted in.
//!
//! Everything that varies per model is declared in [`REGISTRY`] and nowhere
//! else, for the same reason presets and kinds are (`wobu-core`'s `preset.rs`) —
//! a model released next month is a row in a table, not a patch to the budget.
//! `a_model_the_registry_has_never_heard_of_still_behaves` is what holds that
//! line: every accessor reads the struct, so nothing can grow a match on a model
//! id.
//!
//! See the reference-budget table in `docs/08-providers.md`.

use serde::Serialize;
use wobu_core::AssetRole;

/// A count of reference images.
///
/// A newtype for one reason the plain `usize` could not carry: [`UNLIMITED`] has
/// to be expressible and has to be tellable apart from a real cap. A local
/// ComfyUI has no meaningful reference limit, and `influence_resolve` compiles
/// for display before any backend has been chosen, so the Inspector renders a
/// cap that does not exist more often than it renders a tight one. `3/3 style
/// refs` is the sentence with a cap; [`limit`](Self::limit) is what says there is
/// no denominator to print rather than printing `usize::MAX`.
///
/// [`UNLIMITED`]: Self::UNLIMITED
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Refs(usize);

impl Refs {
    /// A cap nothing can exceed. What a local ComfyUI deserves — the same
    /// reasoning as [`Chars::UNLIMITED`](crate::Chars::UNLIMITED), and spelled
    /// the same way so the two budgets stay readable side by side.
    pub const UNLIMITED: Refs = Refs(usize::MAX);

    pub const fn new(count: usize) -> Refs {
        Refs(count)
    }

    pub fn get(self) -> usize {
        self.0
    }

    /// The number to print as a denominator, or `None` when the backend does not
    /// cap this bucket at all.
    pub fn limit(self) -> Option<usize> {
        (self != Refs::UNLIMITED).then_some(self.0)
    }
}

/// One of the categories a backend counts reference images in.
///
/// **This is the backend's vocabulary and not ours.** Ours is [`AssetRole`] —
/// `silhouette`, `palette`, `material`, `mood`, `pose`, `costume`, `full_ref` —
/// which says what a picture is *for*; this says which of the provider's slots it
/// will be counted against. The two are different lengths and different ideas,
/// and [`for_role`](Self::for_role) is the whole of the translation between them.
/// Keeping them one type would mean either inventing provider slots for roles no
/// provider knows about, or losing the distinction between a pose reference and a
/// style reference at the point where it decides which one gets dropped.
///
/// `docs/08-providers.md` sketches this as `RefRole` in its `Capabilities`
/// struct. Renamed here on purpose: two types called something-`Role` sitting one
/// function apart is how a `pose` reference ends up competing for a style slot,
/// which is the exact failure this module exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefBucket {
    /// The general bucket: here is a thing, look at it. Every backend has one,
    /// and it is where the buckets a backend does not separate out end up — see
    /// [`ImageBudget::meter`].
    Objects,
    /// Images of a figure whose identity the backend is asked to preserve.
    Characters,
    /// Images whose *look* is transferred rather than their content.
    StyleRefs,
}

impl RefBucket {
    /// In the column order of the table in `docs/08-providers.md`, which is also
    /// the order the Inspector lists the counters in.
    pub const ALL: [RefBucket; 3] =
        [RefBucket::Objects, RefBucket::Characters, RefBucket::StyleRefs];

    /// What the Inspector calls this bucket: the noun in `3/3 style refs`.
    pub fn label(self) -> &'static str {
        match self {
            RefBucket::Objects => "objects",
            RefBucket::Characters => "characters",
            RefBucket::StyleRefs => "style refs",
        }
    }

    /// Which bucket a reference in this role competes in, or `None` for a role
    /// that is never sent anywhere.
    ///
    /// The judgement in this table is the load-bearing part of #44, so here is
    /// the argument for each line:
    ///
    /// - `material`, `costume` and `full_ref` are **style refs**. They are
    ///   exactly the roles whose [`AssetRole::target`] is
    ///   [`StyleRef`](wobu_core::FragmentTarget::StyleRef) — our style-transfer
    ///   channel and the provider's style column are the same channel, which is
    ///   what makes the Inspector's `3/3 style refs` a true sentence rather than
    ///   two words that happen to match. `full_ref` is the arguable one: locking
    ///   an entity's appearance across generations is what a provider's character
    ///   reference is *for*. It is not routed there because a `full_ref` hangs off
    ///   whatever node the user attached it to — a prop, a location, a culture —
    ///   and a bucket that depended on the node's kind would put the same picture
    ///   in two different buckets on two different cards.
    /// - `pose` is a **character**. A pose reference is a photograph of a figure
    ///   holding a pose, which is what the provider's character bucket takes, and
    ///   it is the only role of ours that reliably is one. Something has to be:
    ///   a mapping in which nothing reaches that bucket leaves four slots on
    ///   `gemini-3.1-flash-image` and five on `gemini-3-pro-image` permanently
    ///   unused while references are dropped from a bucket next door.
    /// - `silhouette` is an **object**, not a character, even though it shares the
    ///   structure channel with `pose`. A silhouette is a shape, and a shape is as
    ///   likely to be a prop or a building as a person.
    /// - `palette` is an **object**. Colour is arguably style, but the style
    ///   column is the smallest bucket any of these models has — three, on the one
    ///   model that has it at all — and swatches filed there would evict the
    ///   `full_ref` that pins the subject's appearance. A palette is also a
    ///   picture of a thing rather than an aesthetic to imitate, and the object
    ///   bucket is the general one.
    /// - `mood` has no bucket, because it is never sent. It must not consume a
    ///   slot and must not be reported as a casualty; it is doing exactly what it
    ///   was attached to do.
    ///
    /// The `mood` line is derived from [`AssetRole::is_conditioning`] by test
    /// rather than restated here as a second list of what is private, for the
    /// reason that accessor itself gives: two such lists are one rename away from
    /// disagreeing, and the direction they fail in is somebody's mood board on a
    /// third party's servers.
    pub fn for_role(role: AssetRole) -> Option<RefBucket> {
        match role {
            AssetRole::Material | AssetRole::Costume | AssetRole::FullRef => {
                Some(RefBucket::StyleRefs)
            }
            AssetRole::Pose => Some(RefBucket::Characters),
            AssetRole::Silhouette | AssetRole::Palette => Some(RefBucket::Objects),
            AssetRole::Mood => None,
        }
    }
}

/// What one compilation is allowed to spend on reference images.
///
/// The sibling of [`Budget`](crate::Budget), and the tighter of the two: a
/// five-layer stack can offer more style references than any of these models
/// takes, on its own (`docs/04-influence-engine.md`).
///
/// **A `–` in the providers table is not a zero.** `characters: None` does not
/// mean the backend refuses images of people — it means it does not separate them
/// out, so they are metered as objects along with everything else. Reading the
/// dash as "unsupported" would silently discard references the user deliberately
/// attached, which is the worst thing this engine could do; and the table itself
/// says so, because every model's columns sum to fourteen. Fourteen undivided,
/// ten plus four, six plus five plus three: these are three partitions of one
/// reference budget into progressively more specialised categories, not three
/// different budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageBudget {
    /// Not an `Option`, unlike the other two. Every backend that takes reference
    /// images at all has this bucket, and it is the one the others fall back
    /// into, so a budget in which it could be absent would have nowhere to put
    /// the fallback.
    pub objects: Refs,
    /// `None` is the `–`: metered as objects, not refused.
    pub characters: Option<Refs>,
    /// `None` is the `–`: metered as objects, not refused.
    pub style_refs: Option<Refs>,
}

impl ImageBudget {
    /// A budget nothing can exceed, with all three buckets kept apart.
    ///
    /// Spelled as a budget rather than as an `Option<ImageBudget>` at the call
    /// site for the reason [`Budget::unlimited`](crate::Budget::unlimited) is:
    /// one path through [`compile_images`](crate::compile_images) and no
    /// "unbudgeted" mode in which the report would mean something different.
    /// `influence_resolve` compiles for display before any backend has been
    /// chosen, and that must show the same three counters it will show
    /// afterwards — with no denominators, because there is no cap yet.
    pub fn unlimited() -> ImageBudget {
        ImageBudget {
            objects: Refs::UNLIMITED,
            characters: Some(Refs::UNLIMITED),
            style_refs: Some(Refs::UNLIMITED),
        }
    }

    /// Whether this backend counts `bucket` separately at all. The buckets it
    /// does are the counters the Inspector has to show, so this is also the shape
    /// of the report [`compile_images`](crate::compile_images) builds.
    pub fn declares(self, bucket: RefBucket) -> bool {
        match bucket {
            RefBucket::Objects => true,
            RefBucket::Characters => self.characters.is_some(),
            RefBucket::StyleRefs => self.style_refs.is_some(),
        }
    }

    /// Where this backend counts a reference that wants `bucket`, and how many
    /// that bucket takes.
    ///
    /// One call and not a `declares` then a `cap`, because the two answers have
    /// to agree: a reference re-bucketed into `objects` competes for the object
    /// slots that already exist rather than getting a fresh pool of its own. A
    /// backend that answered fourteen objects *and* fourteen characters would let
    /// a stack send twenty-eight pictures to a model that takes fourteen, and the
    /// failure would land at the provider, after the request was paid for.
    pub fn meter(self, bucket: RefBucket) -> (RefBucket, Refs) {
        let declared = match bucket {
            RefBucket::Objects => Some(self.objects),
            RefBucket::Characters => self.characters,
            RefBucket::StyleRefs => self.style_refs,
        };
        declared.map_or((RefBucket::Objects, self.objects), |cap| (bucket, cap))
    }
}

/// One row of the reference-budget table in `docs/08-providers.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelRefs {
    /// The provider's own model id, verbatim, because that is what a
    /// `Generation.model` records and what an adapter puts on the wire.
    pub model: &'static str,
    pub budget: ImageBudget,
}

const fn refs(count: usize) -> Option<Refs> {
    Some(Refs::new(count))
}

/// The table from `docs/08-providers.md`, in the order it is written there.
///
/// Only image models appear. A text model has no reference budget, and a row
/// claiming otherwise would be answered rather than skipped by
/// [`image_budget`].
const REGISTRY: &[ModelRefs] = &[
    ModelRefs {
        model: "gemini-3.1-flash-lite-image",
        budget: ImageBudget { objects: Refs::new(14), characters: None, style_refs: None },
    },
    ModelRefs {
        model: "gemini-3.1-flash-image",
        budget: ImageBudget { objects: Refs::new(10), characters: refs(4), style_refs: None },
    },
    ModelRefs {
        model: "gemini-3-pro-image",
        budget: ImageBudget { objects: Refs::new(6), characters: refs(5), style_refs: refs(3) },
    },
];

/// Every model whose reference budget is known, in the docs' order.
pub fn model_refs_registry() -> &'static [ModelRefs] {
    REGISTRY
}

/// The reference budget for a model id, or `None` when nothing in the registry
/// names it.
///
/// `None` rather than a default, and emphatically not
/// [`ImageBudget::unlimited`]: a caller that has picked a model we do not know is
/// in a different situation from one that has picked no model at all, and quietly
/// handing it an unlimited budget would build a request with more references than
/// the provider takes and let the provider be the one to say so. A local backend
/// with no meaningful cap asks for [`ImageBudget::unlimited`] by name.
pub fn image_budget(model: &str) -> Option<ImageBudget> {
    REGISTRY.iter().find(|m| m.model == model).map(|m| m.budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_is_counted_in_the_bucket_this_module_says_it_is() {
        // The mapping longhand, because it is the judgement call of #44 and a
        // diff on it is meant to be read and argued with. Getting a line wrong
        // throws nothing: a `pose` reference filed as a style ref would evict the
        // `full_ref` that pins the subject's appearance, and the only symptom is
        // art that is wrong for a reason nothing on screen could explain.
        let expected = [
            (AssetRole::Silhouette, Some(RefBucket::Objects)),
            (AssetRole::Palette, Some(RefBucket::Objects)),
            (AssetRole::Material, Some(RefBucket::StyleRefs)),
            (AssetRole::Mood, None),
            (AssetRole::Pose, Some(RefBucket::Characters)),
            (AssetRole::Costume, Some(RefBucket::StyleRefs)),
            (AssetRole::FullRef, Some(RefBucket::StyleRefs)),
        ];
        assert_eq!(expected.len(), AssetRole::ALL.len(), "a new role needs a bucket here");
        for (role, bucket) in expected {
            assert_eq!(RefBucket::for_role(role), bucket, "{role}");
        }

        // And every bucket a model declares is reachable. A mapping in which
        // nothing lands in `characters` leaves five slots of `gemini-3-pro-image`
        // permanently unused while references are dropped from the bucket next
        // door — a silent loss, which is the one failure this issue is about.
        for bucket in RefBucket::ALL {
            assert!(
                AssetRole::ALL.iter().any(|role| RefBucket::for_role(*role) == Some(bucket)),
                "no role ever reaches {bucket:?}"
            );
        }
    }

    #[test]
    fn a_role_that_may_never_be_sent_has_no_bucket_to_compete_for() {
        // The privacy property at the budgeting layer, and stated as an
        // equivalence with `wobu-core`'s own answer rather than as a second list
        // of which roles are private. A mood reference that had a bucket would
        // occupy a slot a sendable reference needed, and would then be reported
        // as a casualty of a budget it was never in.
        for role in AssetRole::ALL {
            assert_eq!(
                RefBucket::for_role(role).is_some(),
                role.is_conditioning(),
                "{role} disagrees with wobu-core about whether it is ever sent"
            );
        }
        assert_eq!(RefBucket::for_role(AssetRole::Mood), None);
    }

    #[test]
    fn a_dash_in_the_providers_table_means_metered_as_objects_and_not_refused() {
        // The other half of #44's judgement. Read as "unsupported", a character
        // reference sent to `gemini-3.1-flash-lite-image` would be discarded —
        // and the model takes it perfectly well, it just does not have a separate
        // category for it. Wrong in this direction the engine silently drops
        // references the user deliberately attached.
        let lite = image_budget("gemini-3.1-flash-lite-image").unwrap();
        assert_eq!(lite.meter(RefBucket::Characters), (RefBucket::Objects, Refs::new(14)));
        assert_eq!(lite.meter(RefBucket::StyleRefs), (RefBucket::Objects, Refs::new(14)));
        assert_eq!(lite.meter(RefBucket::Objects), (RefBucket::Objects, Refs::new(14)));

        // Metered as objects means competing for the object slots that already
        // exist, not being handed a pool of the same size. Fourteen is the whole
        // budget for that model, however the references are divided up.
        assert!(!lite.declares(RefBucket::Characters));
        assert!(!lite.declares(RefBucket::StyleRefs));
        assert!(lite.declares(RefBucket::Objects));

        let flash = image_budget("gemini-3.1-flash-image").unwrap();
        assert_eq!(flash.meter(RefBucket::Characters), (RefBucket::Characters, Refs::new(4)));
        assert_eq!(flash.meter(RefBucket::StyleRefs), (RefBucket::Objects, Refs::new(10)));
    }

    #[test]
    fn the_table_in_the_providers_doc_is_the_registry() {
        // `docs/08-providers.md` lists these three and these numbers, and they
        // feed the image budget by name. A row edited here without the doc, or
        // the other way round, is a request built against a cap nobody agreed to.
        let rows: Vec<(&str, usize, Option<usize>, Option<usize>)> = REGISTRY
            .iter()
            .map(|m| {
                (
                    m.model,
                    m.budget.objects.get(),
                    m.budget.characters.map(Refs::get),
                    m.budget.style_refs.map(Refs::get),
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                ("gemini-3.1-flash-lite-image", 14, None, None),
                ("gemini-3.1-flash-image", 10, Some(4), None),
                ("gemini-3-pro-image", 6, Some(5), Some(3)),
            ]
        );

        // And the reading that makes the dash mean "metered as objects": each row
        // is a partition of one fourteen-image budget into progressively more
        // specialised categories. Not an invariant future models are held to —
        // it is the evidence for how this table is read, pinned so that a fourth
        // row that breaks it has to be argued about rather than pasted in.
        for row in REGISTRY {
            let total = row.budget.objects.get()
                + row.budget.characters.map_or(0, Refs::get)
                + row.budget.style_refs.map_or(0, Refs::get);
            assert_eq!(total, 14, "{} totals {total}", row.model);
        }
    }

    #[test]
    fn a_model_the_registry_has_never_heard_of_still_behaves() {
        // The same acceptance criterion `wobu-core`'s preset registry holds
        // itself to (#45): a model released next month is a row, not a patch.
        // Anything that special-cased a model id would have to fall through to a
        // default for this one, and the request would go out shaped for a model
        // it is not for.
        let next_month =
            ImageBudget { objects: Refs::new(20), characters: refs(8), style_refs: None };
        assert!(image_budget("gemini-4-ultra-image").is_none(), "the point is it is unregistered");
        assert_eq!(next_month.meter(RefBucket::Objects), (RefBucket::Objects, Refs::new(20)));
        assert_eq!(next_month.meter(RefBucket::Characters), (RefBucket::Characters, Refs::new(8)));
        assert_eq!(next_month.meter(RefBucket::StyleRefs), (RefBucket::Objects, Refs::new(20)));
        assert!(!next_month.declares(RefBucket::StyleRefs));
    }

    #[test]
    fn an_unlimited_budget_is_one_nothing_can_exceed_and_prints_no_denominator() {
        // `influence_resolve` compiles for display, before a backend is chosen,
        // and a local ComfyUI never gets a cap at all. Both have to be a budget
        // rather than an absent one — and the Inspector has to be able to tell
        // that there is no number to put after the slash, instead of rendering
        // `3/18446744073709551615 style refs`.
        let budget = ImageBudget::unlimited();
        for bucket in RefBucket::ALL {
            assert!(budget.declares(bucket), "{bucket:?} must still be its own counter");
            let (metered, cap) = budget.meter(bucket);
            assert_eq!(metered, bucket, "nothing falls back when everything is declared");
            assert_eq!(cap, Refs::UNLIMITED);
            assert_eq!(cap.limit(), None);
        }
        assert_eq!(Refs::new(3).limit(), Some(3));
    }

    #[test]
    fn a_bucket_serialises_for_the_layer_card_and_reads_for_the_user() {
        // #47 renders these and #46 puts them over the bridge. snake_case on the
        // wire like every other wire form in the workspace; the label is the noun
        // in `3/3 style refs`, which is prose and not an identifier.
        assert_eq!(serde_json::to_string(&RefBucket::StyleRefs).unwrap(), r#""style_refs""#);
        assert_eq!(serde_json::to_string(&RefBucket::Objects).unwrap(), r#""objects""#);
        assert_eq!(serde_json::to_string(&RefBucket::Characters).unwrap(), r#""characters""#);
        assert_eq!(RefBucket::StyleRefs.label(), "style refs");
        assert_eq!(RefBucket::Objects.label(), "objects");
        assert_eq!(RefBucket::Characters.label(), "characters");
    }
}
