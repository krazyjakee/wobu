//! Fitting the reference images into the backend's per-bucket caps, and the
//! account of what did not fit.

use crate::budget::{DropReason, Dropped};
use crate::capability::{ImageBudget, RefBucket, Refs};
use crate::fragment::Fragment;
use wobu_core::Id;

/// A reference image that is still in the running, with the two numbers the
/// budget needs precomputed.
///
/// `index` is the position in the caller's slice, which is reading order, and it
/// is the only thing the weight sort is ever allowed to move — the same
/// discipline, and for the same reason, as [`compile`](crate::compile)'s
/// candidate next door.
struct Candidate<'a> {
    index: usize,
    fragment: Fragment<'a>,
    weight: f32,
}

/// One of the provider's counting buckets: what survived in it, what was cut
/// from it, and how many it takes.
///
/// The kept images live here because the bucket is what the Inspector reports:
/// `3/3 style refs` is a provider quota sentence. It is not adapter routing.
/// Gemini's wire format is one undifferentiated list, and ComfyUI routes through
/// mechanisms such as ControlNet that cut across these buckets (#86).
#[derive(Debug, Clone, PartialEq)]
pub struct Bucket<'a> {
    bucket: RefBucket,
    cap: Refs,
    kept: Vec<Fragment<'a>>,
    dropped: Vec<Dropped<'a>>,
}

impl<'a> Bucket<'a> {
    pub fn bucket(&self) -> RefBucket {
        self.bucket
    }

    /// How many references this bucket takes. [`Refs::limit`] is `None` when
    /// there is no cap, which is what the Inspector shows a bare count for.
    pub fn cap(&self) -> Refs {
        self.cap
    }

    /// What will be sent in this bucket, in reading order — layer by layer, the
    /// subject's own references last, the same order the layer cards list.
    pub fn kept(&self) -> &[Fragment<'a>] {
        &self.kept
    }

    /// What was left out of this bucket, in reading order. Each entry carries its
    /// whole [`Fragment`], so it knows its layer and its source node: that is
    /// what lets the Inspector put `3/3 style refs · 2 dropped` on the card that
    /// lost them rather than in a list nobody can act on.
    pub fn dropped(&self) -> &[Dropped<'a>] {
        &self.dropped
    }
}

/// Every reference image the stack offered, sorted into the buckets the backend
/// counts them in, with the account of everything that did not fit.
///
/// The fields are private and [`compile_images`] is the only constructor, for the
/// same reason [`CompiledPrompt`](crate::CompiledPrompt)'s are: the images and
/// the report of what was cut are one answer to one question, and a value that
/// could be built from a list of pictures alone would be one that claims nothing
/// was dropped.
///
/// There is exactly one [`Bucket`] per bucket the backend declares, in the column
/// order of the table in `docs/08-providers.md`, and buckets it does not declare
/// are absent — their references were metered as objects
/// ([`ImageBudget::meter`]), so the shape of this report is the shape of the
/// backend's capability. A bucket with nothing in it is still present: `0/3 style
/// refs` is a true and useful thing for the Inspector to be able to say.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledImages<'a> {
    buckets: Vec<Bucket<'a>>,
}

impl<'a> CompiledImages<'a> {
    pub fn buckets(&self) -> &[Bucket<'a>] {
        &self.buckets
    }

    /// One bucket by name, or `None` when this backend does not count that
    /// bucket separately.
    pub fn bucket(&self, bucket: RefBucket) -> Option<&Bucket<'a>> {
        self.buckets.iter().find(|b| b.bucket == bucket)
    }

    /// Every reference that will be sent, bucket by bucket and in reading order
    /// within each. Bucket order is provider counting order, useful for the
    /// Inspector and an `influence_snapshot`; routing is carried separately by
    /// `wobu-imagine`'s `ReferenceMechanism`.
    pub fn kept(&self) -> impl Iterator<Item = Fragment<'a>> + '_ {
        self.buckets.iter().flat_map(|b| b.kept.iter().copied())
    }

    /// Everything left out, in the same order — including the references a user
    /// turned down to nothing, which are reported as
    /// [`Silenced`](DropReason::Silenced) rather than omitted for the reason the
    /// text budget does it: "you turned this card down" and "this did not fit"
    /// send the user to two different controls.
    ///
    /// `moodboard_only` references are not here and never will be. One is doing
    /// exactly what it was attached to do, and reporting it as a casualty would
    /// send the user off to fix something that is not broken.
    pub fn dropped(&self) -> impl Iterator<Item = Dropped<'a>> + '_ {
        self.buckets.iter().flat_map(|b| b.dropped.iter().copied())
    }
}

/// Budget a resolved stack's reference images against what the backend takes.
///
/// Takes the same slice [`fragments`](crate::fragments) returns that
/// [`compile`](crate::compile) does — unsorted, unfiltered, zero-weight and
/// `moodboard_only` entries included — and prices the half of it that the text
/// budget deliberately does not (`docs/04-influence-engine.md`, compile step 4).
/// The two run over the same input and neither can drop the other's material, so
/// a stack that loses a style reference keeps every word it had.
///
/// This is the tighter of the two budgets and the one that actually bites: a
/// five-layer stack can offer more style references than `gemini-3-pro-image`
/// takes on its own.
///
/// **Bucket order and reading order are different orders and are kept
/// structurally apart**, exactly as drop order and emit order are in
/// [`compile`](crate::compile). Within a bucket, references are dropped by
/// weight — lightest first — until the bucket fits, and what survives comes back
/// in the order it arrived, because only a list of positions is ever sorted and
/// the fragments themselves are never reordered at all. Ties break towards
/// keeping the *later* reference, which is the one closer to the subject: an
/// `influence_snapshot` has to compile to the same request in five years, and
/// "it happened to be a stable sort" is not a contract anybody wrote down.
///
/// What is not budgeted, and why:
///
/// - **`moodboard_only` references.** Filtered on [`Fragment::is_sendable`],
///   which `wobu-core`, #42 and the text budget all route through, and never
///   re-derived from the role here. A mood reference occupies no slot, so
///   attaching one can never be what costs a real reference its place, and it is
///   not in the report at all.
/// - **Text fragments.** They are not pictures and they take no slot, so this
///   prices them at nothing, makes no decision about them, and leaves them out of
///   this report — the mirror image of what the text budget does with references.
/// - **Zero-weight references.** Reported as [`DropReason::Silenced`], and they
///   cost the budget nothing, so turning a card down cannot cost a heavier card's
///   reference its slot.
///
/// There is no floor: unlike the positive prompt, which is trimmed to one
/// fragment and no further because a request with no prompt still costs money and
/// renders something nobody described, a request with no reference images is an
/// ordinary request.
pub fn compile_images<'a>(fragments: &[Fragment<'a>], budget: ImageBudget) -> CompiledImages<'a> {
    compile_images_for_subjects(fragments, budget, &[])
}

/// Budget a scene while giving every entity a fair chance to keep an identity
/// reference in each provider bucket.
///
/// The ordinary single-subject compiler remains byte-for-byte the same. Scene
/// composition adds two rules: an exact `(asset, role)` duplicate occupies one
/// slot, and each named subject's strongest direct reference is protected before
/// remaining slots are filled by weight. If a provider cap is smaller than the
/// number of protected anchors, the lightest anchors still appear in `dropped`;
/// the scene caller can then refuse explicitly instead of sending a request that
/// silently lost an entity.
pub fn compile_scene_images<'a>(
    fragments: &[Fragment<'a>],
    budget: ImageBudget,
    subjects: &[Id],
) -> CompiledImages<'a> {
    compile_images_for_subjects(fragments, budget, subjects)
}

fn compile_images_for_subjects<'a>(
    fragments: &[Fragment<'a>],
    budget: ImageBudget,
    subjects: &[Id],
) -> CompiledImages<'a> {
    // The buckets this backend counts separately, which is the shape of the
    // report. A `Vec` of at most three, searched linearly: nothing here may
    // answer differently from one run to the next, and a hashed map iterates in
    // an order that is a trap left for whoever iterates it next (`crate::World`).
    let declared: Vec<RefBucket> =
        RefBucket::ALL.into_iter().filter(|bucket| budget.declares(*bucket)).collect();
    let mut pools: Vec<Vec<Candidate<'a>>> = declared.iter().map(|_| Vec::new()).collect();
    let mut report: Vec<(usize, RefBucket, DropReason)> = Vec::new();

    for (index, fragment) in fragments.iter().enumerate() {
        let Some(role) = fragment.asset_role() else { continue };
        if !fragment.is_sendable() {
            continue;
        }
        // `for_role` answers `None` only for the roles `is_sendable` has already
        // taken, and a test pins the two together. Written as a `let else` rather
        // than an unwrap because a disagreement between them must not turn a
        // panic-free crate into a panicking one at the last step before a request.
        let Some(wanted) = RefBucket::for_role(role) else { continue };
        // `meter` only ever names a bucket the backend declares, and `objects` —
        // the one everything falls back into — is declared by construction, so
        // this cannot miss. Written as a lookup rather than an index arithmetic
        // trick so that if it ever did, it would be one line to find.
        let (metered, _) = budget.meter(wanted);
        let Some(pool) = declared.iter().position(|bucket| *bucket == metered) else { continue };
        if !fragment.contributes() {
            report.push((index, metered, DropReason::Silenced));
            continue;
        }
        if !subjects.is_empty() {
            let duplicate = pools[pool].iter().position(|candidate| {
                candidate.fragment.asset_id() == fragment.asset_id()
                    && candidate.fragment.asset_role() == fragment.asset_role()
            });
            if let Some(position) = duplicate {
                if fragment.weight() >= pools[pool][position].weight {
                    report.push((pools[pool][position].index, metered, DropReason::Budget));
                    pools[pool][position] =
                        Candidate { index, fragment: *fragment, weight: fragment.weight() };
                } else {
                    report.push((index, metered, DropReason::Budget));
                }
                continue;
            }
        }
        pools[pool].push(Candidate { index, fragment: *fragment, weight: fragment.weight() });
    }

    let mut buckets: Vec<Bucket<'a>> = Vec::new();
    for (bucket, pool) in declared.iter().zip(pools) {
        let (_, cap) = budget.meter(*bucket);
        let kept = trim(pool, cap, *bucket, subjects, &mut report);
        buckets.push(Bucket { bucket: *bucket, cap, kept, dropped: Vec::new() });
    }

    // Back into reading order, so each bucket's casualties walk alongside the
    // layer cards rather than in whatever order the silenced and the cut happened
    // to be found in.
    report.sort_unstable_by_key(|(index, _, _)| *index);
    for (index, bucket, reason) in report {
        if let Some(slot) = buckets.iter_mut().find(|b| b.bucket == bucket) {
            slot.dropped.push(Dropped { fragment: fragments[index], reason });
        }
    }

    CompiledImages { buckets }
}

/// Drop references, lightest first, until the bucket holds no more than `cap` of
/// them.
///
/// The sort is a total order rather than a stable sort over the weights alone,
/// and it is the same comparator [`compile`](crate::compile)'s `trim` uses:
/// ascending by weight, ties broken towards the earlier fragment, and the earlier
/// fragment is the one further out in the stack. Dropping from that end means a
/// tie is resolved in favour of the reference closer to the subject, which is the
/// more specific picture. `total_cmp` and not `partial_cmp().unwrap()`: weights
/// are products of clamped finite numbers today, but a NaN arriving from
/// somewhere would panic at the last step before the request.
///
/// Survivors come back in reading order because only `order` was ever sorted.
fn trim<'a>(
    candidates: Vec<Candidate<'a>>,
    cap: Refs,
    bucket: RefBucket,
    subjects: &[Id],
    report: &mut Vec<(usize, RefBucket, DropReason)>,
) -> Vec<Fragment<'a>> {
    let mut order: Vec<(f32, usize)> =
        candidates.iter().enumerate().map(|(position, c)| (c.weight, position)).collect();
    order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut protected = vec![false; candidates.len()];
    for subject in subjects {
        let best = candidates
            .iter()
            .enumerate()
            .filter(|(_, candidate)| candidate.fragment.node_id() == Some(*subject))
            .max_by(|left, right| {
                left.1.weight.total_cmp(&right.1.weight).then(left.0.cmp(&right.0))
            })
            .map(|(position, _)| position);
        if let Some(position) = best {
            protected[position] = true;
        }
    }

    // Unprotected references pay first. Protected anchors are considered only
    // when the provider cap makes retaining one per subject impossible, which
    // is the explicit refusal seam documented on `compile_scene_images`.
    order.sort_by_key(|(_, position)| protected[*position]);
    let mut doomed = vec![false; candidates.len()];
    let mut remaining = candidates.len();
    for (_, position) in order {
        if remaining <= cap.get() {
            break;
        }
        doomed[position] = true;
        remaining -= 1;
        report.push((candidates[position].index, bucket, DropReason::Budget));
    }

    candidates.into_iter().zip(doomed).filter(|(_, cut)| !cut).map(|(c, _)| c.fragment).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::image_budget;
    use crate::fragment::FragmentBody;
    use crate::stack::{Origin, Reached, ResolvedSource};
    use wobu_core::{AssetRole, Id, Layer, Node, NodeKind};

    fn shot_source() -> ResolvedSource<'static> {
        ResolvedSource {
            layer: Layer::Shot,
            origin: Origin::Shot("Character sheet · 3:4"),
            reached: Reached::Shot,
            distance: 0,
            weight: 1.0,
        }
    }

    fn reference(role: AssetRole) -> Fragment<'static> {
        Fragment::new(
            &shot_source(),
            role.as_str(),
            FragmentBody::Asset { id: Id::nil(), role },
            1.0,
            role.target(),
        )
    }

    fn subject_reference<'a>(node: &'a Node, asset: Id, weight: f32) -> Fragment<'a> {
        let source = ResolvedSource {
            layer: Layer::Subject,
            origin: Origin::Node(node),
            reached: Reached::Subject,
            distance: 0,
            weight: 1.0,
        };
        Fragment::new(
            &source,
            AssetRole::FullRef.as_str(),
            FragmentBody::Asset { id: asset, role: AssetRole::FullRef },
            weight,
            AssetRole::FullRef.target(),
        )
    }

    #[test]
    fn every_role_that_can_be_sent_lands_in_a_bucket_and_mood_lands_in_none() {
        // Stated over the whole role vocabulary rather than over the ones that
        // matter, so a role added later is classified here deliberately instead
        // of falling out of every bucket and vanishing with nothing to report it.
        // Against the pro model, which is the only one that declares all three.
        let pro = image_budget("gemini-3-pro-image").unwrap();
        let expected = [
            (AssetRole::Silhouette, Some(RefBucket::Objects)),
            (AssetRole::Palette, Some(RefBucket::Objects)),
            (AssetRole::Material, Some(RefBucket::StyleRefs)),
            (AssetRole::Mood, None),
            (AssetRole::Pose, Some(RefBucket::Characters)),
            (AssetRole::Costume, Some(RefBucket::StyleRefs)),
            (AssetRole::FullRef, Some(RefBucket::StyleRefs)),
        ];
        for (role, bucket) in expected {
            let compiled = compile_images(&[reference(role)], pro);
            let landed: Vec<RefBucket> = compiled
                .buckets()
                .iter()
                .filter(|b| !b.kept().is_empty())
                .map(|b| b.bucket())
                .collect();
            assert_eq!(landed, bucket.into_iter().collect::<Vec<_>>(), "{role}");
            assert_eq!(compiled.dropped().count(), 0, "{role} was not dropped, it was routed");
        }
    }

    #[test]
    fn a_text_fragment_takes_no_slot_and_is_not_in_this_report() {
        // The mirror of what the text budget does with references: this one makes
        // no decision about prose at all. A prompt fragment counted as an object
        // would evict a picture the user attached, for a sentence that was never
        // competing with it.
        let words = Fragment::new(
            &shot_source(),
            "framing",
            FragmentBody::Text("full body"),
            1.0,
            wobu_core::FragmentTarget::Prompt,
        );
        let compiled = compile_images(&[words], image_budget("gemini-3-pro-image").unwrap());
        assert_eq!(compiled.kept().count(), 0);
        assert_eq!(compiled.dropped().count(), 0);
        assert_eq!(compiled.buckets().len(), 3, "the counters are the backend's, not the stack's");
    }

    #[test]
    fn a_scene_protects_one_direct_reference_per_subject_before_filling_by_weight() {
        let first = Node::new(NodeKind::Character, "First").unwrap();
        let second = Node::new(NodeKind::Character, "Second").unwrap();
        let first_best = Id::from(1_u128);
        let first_extra = Id::from(2_u128);
        let second_only = Id::from(3_u128);
        let refs = [
            subject_reference(&first, first_best, 1.0),
            subject_reference(&first, first_extra, 0.9),
            subject_reference(&second, second_only, 0.1),
        ];
        let budget = ImageBudget {
            objects: Refs::new(0),
            characters: Some(Refs::new(0)),
            style_refs: Some(Refs::new(2)),
        };

        let compiled = compile_scene_images(&refs, budget, &[first.id, second.id]);
        let kept: Vec<_> = compiled.kept().filter_map(Fragment::asset_id).collect();

        assert_eq!(kept, [first_best, second_only]);
        assert_eq!(compiled.dropped().count(), 1);
    }
}
