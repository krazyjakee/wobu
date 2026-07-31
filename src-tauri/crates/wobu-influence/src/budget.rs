//! What a prompt is allowed to cost, and the account of what did not fit.

use serde::Serialize;

use crate::fragment::Fragment;

/// The characters one token is assumed to be worth.
///
/// Three, against the four or so that English prose averages under the BPE
/// tokenizers these backends use. The low number is the whole point — see
/// [`Chars::for_token_limit`].
const CHARS_PER_TOKEN: usize = 3;

/// A length in characters — `char`s, not bytes, and emphatically not tokens.
///
/// A newtype and not a `usize` because the number a provider documents is a
/// *token* limit and the only number this crate can actually count is a
/// character count, and the two differ by a factor of three or four. Passed as
/// bare integers they are indistinguishable at the call site, and the mistake
/// that direction produces a prompt several times over the real limit which
/// fails at the provider, after the request has been paid for.
///
/// There is no tokenizer in this workspace and deliberately will not be one:
/// every real BPE implementation is a megabyte of merge tables per model family,
/// and `prompt_compile` runs on every drag of a weight slider and has to stay
/// sub-millisecond (`docs/05-architecture.md`). So the budget is measured in
/// characters, which is an estimate of what a backend actually meters. The
/// direction that estimate errs in is chosen rather than accidental — see
/// [`for_token_limit`](Self::for_token_limit).
///
/// Characters rather than bytes because an em dash is one character of prompt
/// and three bytes of UTF-8, and a budget that shrank when the user typed a
/// nicer dash would be indefensible to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Chars(usize);

impl Chars {
    /// A budget nothing can exceed. What a local ComfyUI deserves — it has no
    /// meaningful cap and inventing one for it would cut fragments for nothing.
    pub const UNLIMITED: Chars = Chars(usize::MAX);

    pub const fn new(count: usize) -> Chars {
        Chars(count)
    }

    /// What a string costs. `chars().count()` and not `len()`, for the reason in
    /// the type docs.
    pub fn of(text: &str) -> Chars {
        Chars(text.chars().count())
    }

    /// The character budget for a backend that states its limit in tokens.
    ///
    /// [`CHARS_PER_TOKEN`] is deliberately the low estimate, because the two
    /// failure directions are not symmetric. Under-filling costs a slightly
    /// thinner prompt, and the drop report then says exactly which fragments
    /// paid for it, so the user can see it and argue. Over-filling costs a
    /// rejected request the user has already been billed for, from a provider
    /// whose error will not name the fragment that was the last straw.
    ///
    /// It is still an estimate and it is not safe against pathological input: a
    /// `palette` section is `#2b2118` strings, which tokenize at nearer 1.5
    /// characters per token, and invented proper nouns are not much better. A
    /// caller holding a real token count for a real model should build a
    /// [`Chars`] from it with [`new`](Self::new) rather than come through here,
    /// and [`CompiledPrompt::overflow`] exists because neither route can promise
    /// the result fits.
    pub fn for_token_limit(tokens: usize) -> Chars {
        Chars(tokens.saturating_mul(CHARS_PER_TOKEN))
    }

    pub fn get(self) -> usize {
        self.0
    }
}

/// What one compilation is allowed to spend on text.
///
/// Two pools and not one, because the positive and negative prompts are separate
/// fields of every request this app will build and are metered separately.
/// Charging the negatives against the positive prompt's limit would drop the
/// subject's costume to make room for a `never` list that was never competing
/// for that space — art that is quietly worse, for a reason nothing on screen
/// could explain.
///
/// The image budget is not here. Backends cap references *per role* against a
/// declared capability this crate cannot see (`docs/08-providers.md`), that cap
/// is the one that actually bites, and it is #44.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub prompt: Chars,
    pub negative: Chars,
}

impl Budget {
    /// A budget nothing can exceed.
    ///
    /// Spelled as a budget rather than as an `Option<Budget>` at the call site
    /// so there is one path through [`compile`](crate::compile) and no
    /// "unbudgeted" mode in which the drop report would mean something
    /// different. `influence_resolve` compiles for display before any backend
    /// has been chosen, and that must not be a second code path.
    pub fn unlimited() -> Budget {
        Budget { prompt: Chars::UNLIMITED, negative: Chars::UNLIMITED }
    }
}

/// Why a fragment the user wrote is not in the compiled prompt.
///
/// An enum and not the `bool` the issue warns against, because the two answers
/// ask the user for different things: `Silenced` means go and put a slider back
/// up, `Budget` means go and write leaner notes upstream — or nothing at all, if
/// the fragment deserved to lose. `SnapshotFragment::dropped` flattens this to a
/// bool on disk, which is right for a record that has to deserialize in five
/// years; it is wrong for the panel whose job is explaining itself.
///
/// Muting is not here yet. It is a layer-level control that removes a whole card
/// rather than a fragment (`Sliders`' docs next door), the Inspector that owns it
/// is #47, and when it lands it belongs in this enum as a third reason filtered
/// ahead of the budget — inventing the variant now would only mean guessing what
/// carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
    /// The fragment carries no weight — a layer card's slider at the bottom, a
    /// link weighted to zero in the world, or a preset priority multiplied
    /// through one of those. It was out before the budget ran and it cost the
    /// budget nothing, so it did not take anything else down with it.
    Silenced,
    /// It did not fit. The lightest go first, so a fragment reported this way was
    /// among the least-weighted of its pool — which is the sentence the Inspector
    /// needs, because "raise this card" is then a real answer.
    Budget,
}

/// One fragment that is not in the prompt, and why.
///
/// Carries the whole [`Fragment`] rather than an id and a section, because a
/// fragment already knows its layer and its source node and refuses to be built
/// without them. A drop report that named a section but not the card it came
/// from would be a list the user cannot act on, which is the failure the whole
/// attribution trail exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dropped<'a> {
    pub fragment: Fragment<'a>,
    pub reason: DropReason,
}

/// The two prompt strings, and the account of everything that is not in them.
///
/// The fields are private and [`compile`](crate::compile) is the only
/// constructor, for the same reason [`Fragment`]'s are: a prompt and its drop
/// report are one answer to one question. A `CompiledPrompt` that could be built
/// from a string alone would be one that claims nothing was cut, and "the
/// Inspector reports what was dropped rather than truncating silently"
/// (`docs/04-influence-engine.md`) only holds while the two cannot be separated.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledPrompt<'a> {
    prompt: String,
    negative: String,
    dropped: Vec<Dropped<'a>>,
    overflow: Option<Chars>,
}

impl<'a> CompiledPrompt<'a> {
    pub(crate) fn new(
        prompt: String,
        negative: String,
        dropped: Vec<Dropped<'a>>,
        overflow: Option<Chars>,
    ) -> CompiledPrompt<'a> {
        CompiledPrompt { prompt, negative, dropped, overflow }
    }

    /// The positive prompt: every layer's prompt text, joined in layer order with
    /// the subject's own last.
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// The negative prompt: every layer's `never` section, joined the same way.
    pub fn negative(&self) -> &str {
        &self.negative
    }

    /// Everything the compiler left out, in reading order — the order the layer
    /// cards list, so the Inspector can walk the two lists together. Each entry
    /// knows its own weight and layer, so a panel that would rather show the
    /// heaviest casualty first can sort it itself.
    pub fn dropped(&self) -> &[Dropped<'a>] {
        &self.dropped
    }

    /// How far over its budget the positive prompt is, or `None` when it fits.
    ///
    /// Only ever `Some` in one situation: the budget could not fit even a single
    /// fragment, and rather than emit nothing the compiler kept the heaviest one
    /// and said so here. An empty positive prompt is not a smaller prompt, it is
    /// a different picture — the backend renders whatever the negatives and the
    /// seed suggest, and bills for it — so the budget is allowed to be wrong out
    /// loud instead of silently right. A caller that must not overrun refuses to
    /// send on this; a caller against a backend whose limit is soft ignores it.
    ///
    /// The negative prompt has no equivalent, because an empty negative prompt is
    /// a legitimate request. It is emptied rather than overrun.
    pub fn overflow(&self) -> Option<Chars> {
        self.overflow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_length_is_counted_in_characters_and_not_in_bytes() {
        // An em dash is one character of prompt and three bytes of UTF-8. Priced
        // by `len()` a stack written with typographic punctuation would lose
        // fragments a plain-ASCII one keeps, and nothing would say why.
        assert_eq!(Chars::of("ash-grey").get(), 8);
        assert_eq!(Chars::of("ash—grey").get(), 8);
        assert_eq!("ash—grey".len(), 10, "which is what would have been charged");
    }

    #[test]
    fn a_token_limit_converts_to_fewer_characters_than_it_sounds_like() {
        // The error direction, pinned. English prose is nearer four characters
        // per token, so this leaves usable prompt length unspent on purpose. The
        // regression this guards is somebody "fixing" the constant upwards to get
        // longer prompts: the symptom of getting it wrong is a rejected paid
        // request, not a shorter prompt.
        assert_eq!(Chars::for_token_limit(1_000).get(), 3_000);
        assert!(Chars::for_token_limit(1_000) < Chars::new(4_000));
        // A backend that declares an absurd limit must not wrap into a tiny one.
        assert_eq!(Chars::for_token_limit(usize::MAX), Chars::UNLIMITED);
    }

    #[test]
    fn an_unlimited_budget_is_one_nothing_can_exceed() {
        // `influence_resolve` compiles for display, before a backend is chosen.
        // That has to be a budget rather than an absent one — see `Budget`.
        let budget = Budget::unlimited();
        assert_eq!(budget.prompt, Chars::UNLIMITED);
        assert_eq!(budget.negative, Chars::UNLIMITED);
        assert!(Chars::of("a prompt of any length at all") < budget.prompt);
    }

    #[test]
    fn a_drop_reason_serialises_for_the_layer_card() {
        // #47 renders these, and #46 puts them over the bridge. snake_case, like
        // every other wire form in the workspace.
        assert_eq!(serde_json::to_string(&DropReason::Silenced).unwrap(), r#""silenced""#);
        assert_eq!(serde_json::to_string(&DropReason::Budget).unwrap(), r#""budget""#);
    }
}
