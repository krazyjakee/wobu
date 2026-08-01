//! Fitting the fragments into the budget, and emitting the two prompts.

use wobu_core::FragmentTarget;

use crate::budget::{Budget, Chars, CompiledPrompt, DropReason, Dropped};
use crate::fragment::Fragment;

/// What joins two fragments in a prompt.
///
/// Comma-space, which is the syntax every image model's prompt convention is
/// written in, and which is why extraction trims each fragment before it gets
/// here (`push_text`). Charged to the budget rather than assumed free — a
/// hundred-fragment stack is two hundred characters of separator, which is a
/// whole fragment's worth of prompt to be wrong by.
const SEPARATOR: &str = ", ";

/// A text fragment that is still in the running, with the two numbers the budget
/// needs precomputed.
///
/// `index` is the position in the caller's slice, which is reading order, and it
/// is the only thing the weight sort is ever allowed to move — see [`compile`].
struct Candidate<'a> {
    index: usize,
    text: &'a str,
    weight: f32,
    cost: usize,
}

/// Compile a resolved stack's fragments into a positive and a negative prompt.
///
/// Takes the slice [`fragments`](crate::fragments) returns — unsorted,
/// unfiltered, zero-weight and `moodboard_only` entries included — and does the
/// filtering, the budgeting and the emission that the doc's compile steps 2, 4
/// and 5 describe (`docs/04-influence-engine.md`).
///
/// **Drop order and emit order are different orders and are kept structurally
/// apart.** Dropping is by weight, ascending: the lightest fragment goes first,
/// and it keeps going until the pool fits. Emitting is in the order the
/// fragments arrived, which is [`fragments`](crate::fragments)' contract — layer
/// by layer, the subject's own fragments last and the shot's framing after them,
/// where a text encoder's recency bias does the most good. Letting the first
/// order become the second produces a prompt that reads specific-to-general,
/// which is art that is subtly wrong forever and never throws. So what the
/// weight sort sorts is a list of *indices*, and the fragments themselves are
/// never reordered at all: the two orders cannot be confused for each other if
/// only one of them is ever materialised.
///
/// What is not budgeted, and why:
///
/// - **`moodboard_only` fragments.** Filtered on [`Fragment::is_sendable`], which
///   `wobu-core` and #42 both route through, and never re-derived from the role
///   here. They cost nothing, appear in neither prompt, and are *not* in the drop
///   report: a mood reference is doing exactly what it was attached to do, and
///   reporting it as a casualty would send the user off to fix something that is
///   not broken.
/// - **Reference images.** They are not text and they do not belong in a prompt
///   string, so a text budget prices them at nothing rather than at a guess.
///   Their budget is the per-role one against the backend's declared capability
///   and it is the tighter of the two — [`compile_images`](crate::compile_images),
///   which is where they are accounted for. Nothing here makes a decision about
///   them, which is why they are not in this drop report either.
/// - **Zero-weight fragments.** Reported as [`DropReason::Silenced`] rather than
///   omitted, because "you turned this down" and "this did not fit" send the user
///   to two different controls. They cost the budget nothing, so turning a card
///   down cannot cost a heavier card its place.
///
/// An empty positive prompt out of this function always means there was nothing
/// to compile, and never that the budget ate it — see
/// [`CompiledPrompt::overflow`]. That distinction is what lets #46 refuse to send
/// a request that would have rendered something nobody described.
pub fn compile<'a>(fragments: &[Fragment<'a>], budget: Budget) -> CompiledPrompt<'a> {
    let mut prompt: Vec<Candidate<'a>> = Vec::new();
    let mut negative: Vec<Candidate<'a>> = Vec::new();
    let mut report: Vec<(usize, DropReason)> = Vec::new();

    for (index, fragment) in fragments.iter().enumerate() {
        let Some(text) = fragment.text() else { continue };
        if !fragment.is_sendable() {
            continue;
        }
        if !fragment.contributes() {
            report.push((index, DropReason::Silenced));
            continue;
        }
        let candidate = Candidate { index, text, weight: fragment.weight(), cost: cost(text) };
        match fragment.target() {
            FragmentTarget::Prompt => prompt.push(candidate),
            FragmentTarget::Negative => negative.push(candidate),
            // The image channels. Written out rather than caught by a wildcard so
            // that a seventh target has to be classified here instead of
            // defaulting into neither prompt and vanishing without a report —
            // which is how a routing change goes wrong with nothing failing.
            // `MoodboardOnly` never reaches this arm; `is_sendable` took it.
            FragmentTarget::StyleRef
            | FragmentTarget::StructureRef
            | FragmentTarget::Palette
            | FragmentTarget::MoodboardOnly => {}
        }
    }

    // The positive prompt may be trimmed to one fragment and no further; the
    // negative may be emptied. Not a symmetry worth having: a request with no
    // negative prompt is an ordinary request, and one with no positive prompt is
    // a picture of nothing that still costs money.
    let prompt = trim(prompt, budget.prompt, 1, &mut report);
    let negative = trim(negative, budget.negative, 0, &mut report);

    // Back into reading order, so the report walks alongside the layer cards
    // rather than in whatever order two pools happened to give up their losses.
    report.sort_unstable_by_key(|(index, _)| *index);
    let dropped = report
        .into_iter()
        .map(|(index, reason)| Dropped { fragment: fragments[index], reason })
        .collect();

    let prompt = join(&prompt);
    let overflow = Chars::of(&prompt).get().saturating_sub(budget.prompt.get());
    CompiledPrompt::new(
        prompt,
        join(&negative),
        dropped,
        (overflow > 0).then(|| Chars::new(overflow)),
    )
}

/// Drop candidates, lightest first, until the pool fits — or until `floor` of
/// them are left, whichever comes first.
///
/// The sort is a total order rather than a stable sort over the weights alone:
/// ties break towards the earlier fragment, which is the one further out in the
/// stack, because general context is what a subject can most afford to lose.
/// Spelling the tie-break out rather than leaning on `sort_by` being stable is
/// what makes this reproducible — an `influence_snapshot` has to compile to the
/// same prompt in five years, and "it happened to be a stable sort" is not a
/// contract anybody wrote down.
///
/// Survivors come back in reading order because only `order` was ever sorted.
fn trim<'a>(
    candidates: Vec<Candidate<'a>>,
    budget: Chars,
    floor: usize,
    report: &mut Vec<(usize, DropReason)>,
) -> Vec<Candidate<'a>> {
    let mut total: usize = candidates.iter().map(|c| c.cost).sum();
    // Weight and position, sorted as a pair rather than as indices into
    // `candidates`, so the comparator touches no other memory. `total_cmp` and
    // not `partial_cmp().unwrap()`: weights are products of clamped finite
    // numbers today, but a NaN arriving from somewhere would turn a panic-free
    // crate into a panicking one at the last step before the prompt.
    let mut order: Vec<(f32, usize)> =
        candidates.iter().enumerate().map(|(position, c)| (c.weight, position)).collect();
    order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut doomed = vec![false; candidates.len()];
    let mut remaining = candidates.len();
    for (_, position) in order {
        if total <= budget.get() || remaining <= floor {
            break;
        }
        total -= candidates[position].cost;
        doomed[position] = true;
        remaining -= 1;
        report.push((candidates[position].index, DropReason::Budget));
    }

    candidates.into_iter().zip(doomed).filter(|(_, cut)| !cut).map(|(c, _)| c).collect()
}

/// What one fragment adds to a prompt: its own characters plus the separator that
/// will join it to whatever came before.
///
/// The first fragment of a pool is charged for a separator it will never print.
/// Two characters of pessimism per prompt, pointed the same way every other
/// rounding in this module is — "we thought it fit and it did not" is the failure
/// that costs a paid request, so it is the one we do not have.
fn cost(text: &str) -> usize {
    text.chars().count() + SEPARATOR.chars().count()
}

/// Counted rather than testing whether anything has been written yet, so that a
/// fragment whose text is empty still separates its neighbours. Extraction
/// refuses to make one (`push_text`), but this is the one place that assumption
/// would fail silently and produce two fragments run together into a word that
/// is in neither of them.
fn join(candidates: &[Candidate<'_>]) -> String {
    let mut out = String::new();
    for (position, candidate) in candidates.iter().enumerate() {
        if position > 0 {
            out.push_str(SEPARATOR);
        }
        out.push_str(candidate.text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment::FragmentBody;
    use crate::stack::{Origin, Reached, ResolvedSource};
    use wobu_core::Layer;

    fn shot_source() -> ResolvedSource<'static> {
        ResolvedSource {
            layer: Layer::Shot,
            origin: Origin::Shot("Character sheet · 3:4"),
            reached: Reached::Shot,
            distance: 0,
            weight: 1.0,
        }
    }

    fn fragment(target: FragmentTarget) -> Fragment<'static> {
        Fragment::new(&shot_source(), "framing", FragmentBody::Text("full body"), 1.0, target)
    }

    #[test]
    fn only_the_two_text_targets_are_prompt_material() {
        // Stated over every target rather than over the two that matter, so a
        // seventh is classified here deliberately instead of silently landing in
        // neither prompt and neither report. A text fragment routed to an image
        // channel is a misrouting upstream, and it has no prompt to belong to.
        let expected = [
            (FragmentTarget::Prompt, ("full body", "")),
            (FragmentTarget::Negative, ("", "full body")),
            (FragmentTarget::StyleRef, ("", "")),
            (FragmentTarget::StructureRef, ("", "")),
            (FragmentTarget::Palette, ("", "")),
            (FragmentTarget::MoodboardOnly, ("", "")),
        ];
        for (target, (prompt, negative)) in expected {
            let compiled = compile(&[fragment(target)], Budget::unlimited());
            assert_eq!((compiled.prompt(), compiled.negative()), (prompt, negative), "{target:?}");
            assert!(compiled.dropped().is_empty(), "{target:?}");
            assert_eq!(compiled.overflow(), None, "{target:?}");
        }
    }

    #[test]
    fn the_separator_is_charged_to_every_fragment_including_the_first() {
        // Deliberate pessimism, and the only place it is visible: nine characters
        // of text priced at eleven. A budget that priced the joined string
        // exactly would be a budget that had to be recomputed on every drop.
        assert_eq!(cost("full body"), 11);
        assert_eq!(cost(""), 2);

        // And the emitted string is what `overflow` measures, so the two extra
        // characters cannot make a prompt that fits report that it does not.
        let compiled = compile(
            &[fragment(FragmentTarget::Prompt)],
            Budget { prompt: Chars::new(9), negative: Chars::UNLIMITED },
        );
        assert_eq!(compiled.prompt(), "full body");
        assert_eq!(compiled.overflow(), None);
    }
}
