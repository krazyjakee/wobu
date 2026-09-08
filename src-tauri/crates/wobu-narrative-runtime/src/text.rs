//! Host-triggered supporting text: selection, delivery and the repeat state a
//! save has to carry (#167).
//!
//! Scene execution is a cursor the runtime owns. Supporting text is the
//! opposite: the *host* says when — the player walked past the gate, opened the
//! codex, finished the quest — and the runtime answers with the words, or with
//! nothing. That asymmetry is why none of this appears in [`Yield`] and why
//! [`Runtime::deliver_text`] does not touch the scene cursor. A bark fired
//! mid-conversation must not be able to advance, rewind or reselect the line the
//! player is reading, and the way to guarantee that is for the delivery path to
//! have no access to the phase at all.
//!
//! Two further things this path deliberately cannot do:
//!
//! - **It cannot write narrative state.** There are no effects on a text asset
//!   and nowhere to put one. Reading a codex page is not a story event; if a
//!   game wants it to be, the host raises its own flag through a host-owned
//!   variable, which is already how the host tells the narrative anything.
//! - **It cannot invent wording.** A line with no variant matching the current
//!   state is [`Error::NoMatch`], the same refusal a scene slot gets, rather
//!   than silence or a fallback. Silence would be indistinguishable from an
//!   asset the author meant to be inaudible here.
//!
//! ## Determinism
//!
//! [`RepeatPolicy::Shuffle`] is the first place in this runtime where the
//! playthrough seed changes what the player sees. The contract for version 1
//! still holds for scene dialogue — variants are first-match, never sampled —
//! and it holds here too in the sense that matters: given the same graph, seed,
//! state and play counts, every host on every platform picks the same entry.
//! The two primitives below are written out in full rather than pulled from a
//! crate precisely because an engine adapter in C#, GDScript or C++ has to
//! reproduce this exact sequence, and "use whatever RNG you have" would make
//! the same save play differently in Unity and Godot.

use super::*;
use wobu_narrative::{RepeatPolicy, TextKind};

/// How far through its selection order one asset has got, in one playthrough.
///
/// Saved rather than derived, because every derivation available at restore time
/// is wrong: the eligible entry set depends on state that has since changed, and
/// counting from the visit log would make a bark repeat itself after a reload.
/// Four small fields are cheaper than one wrong answer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextProgress {
    /// How many times this asset has been delivered. Never zero for a stored
    /// entry: an asset that has not played has no record at all, so a present
    /// record with `plays: 0` is a tampered or truncated save.
    pub plays: u64,
    /// Which shuffled round the next pick comes from. Unused by the other
    /// policies, and left at zero by them rather than overloaded, so a save
    /// written under one policy stays readable if the author changes it.
    pub round: u64,
    /// How far into the current round's order this asset has got.
    pub position: u64,
    /// The entry that closed the previous round, so a new round does not open
    /// with the line the player just heard. `None` before the first rollover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carry: Option<String>,
}

/// One line of a delivery, already resolved to the wording the host should show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveredLine {
    pub slot: String,
    pub variant: String,
    pub speaker: Speaker,
    pub text: String,
    pub revision: String,
}

/// What the host should play for a trigger.
///
/// The whole entry at once, not a cursor, because supporting text has no
/// branches and nothing to wait for: an ambient exchange is two or three lines
/// the host schedules however its audio system likes, and handing them over one
/// `advance()` at a time would create a second cursor that a save would then
/// have to keep consistent with the first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextDelivery {
    pub asset: String,
    pub kind: TextKind,
    pub entry: String,
    /// In delivery order. For an ambient exchange these carry different
    /// speakers; that is the kind's entire purpose.
    pub lines: Vec<DeliveredLine>,
    /// How many times this asset has now been delivered, including this one.
    /// Exposed so a host can drive its own "first time only" presentation
    /// without keeping a second, divergent counter.
    pub plays: u64,
}

/// The multiplier SplitMix64 adds to its state on every draw.
const GAMMA: u64 = 0x9E37_7989_9F4A_7C15;

/// SplitMix64.
///
/// Ten lines of arithmetic with no dependencies, chosen over a crate because its
/// value here is *reproducibility in another language*: an adapter author can
/// transcribe this function and get identical barks. A better-quality generator
/// that only exists in Rust would be worse at the one job this has.
fn split_mix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(GAMMA);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// FNV-1a over an id string, to fold a ULID into the seed.
///
/// Also written out for transcription. It only has to spread two assets in the
/// same playthrough apart from each other, which any mixing function does; what
/// matters is that every implementation picks the *same* one.
fn fold(id: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The order one round of a shuffle visits its eligible entries in.
///
/// A Fisher-Yates shuffle over the *authored* order, seeded from the
/// playthrough seed, the asset identity and the round number. The modulo below
/// is very slightly biased for list lengths that do not divide 2^64; that is
/// accepted deliberately, because rejection sampling would make the sequence
/// harder to transcribe correctly than the bias is worth for a list of barks.
///
/// The eligible set is an input, so changing state mid-round reshuffles what is
/// left. That is honest rather than ideal: pinning an order taken before a
/// condition changed would mean delivering an entry the author had made
/// ineligible.
fn shuffled(
    eligible: &[String],
    seed: u64,
    asset: &str,
    round: u64,
    carry: Option<&str>,
) -> Vec<String> {
    let mut state = seed ^ fold(asset) ^ round.wrapping_mul(GAMMA);
    let mut order = eligible.to_vec();
    for index in (1..order.len()).rev() {
        let pick = (split_mix64(&mut state) % (index as u64 + 1)) as usize;
        order.swap(index, pick);
    }
    // Never open a round with the entry that closed the previous one. Applied
    // to the whole round rather than at each draw, so the order stays fixed for
    // the round and a save taken halfway through resumes into the same list.
    if order.len() > 1 && carry.is_some_and(|last| order[0] == last) {
        order.swap(0, 1);
    }
    order
}

impl Runtime {
    /// Every host event this graph answers, in stable order.
    ///
    /// Offered so a host can bind its triggers once at load rather than
    /// discovering them by firing events and seeing what comes back.
    pub fn text_events(&self) -> Vec<Name> {
        let mut events: Vec<Name> = self.graph.texts.values().map(|t| t.event.clone()).collect();
        events.sort();
        events.dedup();
        events
    }

    /// The recorded delivery state for every asset that has played.
    pub fn text_progress(&self) -> &BTreeMap<String, TextProgress> {
        &self.saved.texts
    }

    /// Ask what to play for a host event, and record that it played.
    ///
    /// `Ok(None)` means the graph has nothing eligible to say, which is the
    /// ordinary answer for most events most of the time and is deliberately not
    /// an error. `Err` means the content is broken — a line with no applicable
    /// wording, a condition over state the save does not contain — and, like
    /// every other public action, leaves the runtime exactly as it was.
    ///
    /// Assets are considered in stable id order. Because narrative ids are
    /// ULIDs, that is creation order, so when two assets answer the same event
    /// the older one wins. This is arbitrary but fixed; an author who cares
    /// which fires should condition them apart rather than rely on it.
    pub fn deliver_text(&mut self, event: &Name) -> Result<Option<TextDelivery>> {
        let mut next = self.clone();
        next.trace = ExecutionTrace::default();
        match next.select_text(event) {
            Ok(delivery) => {
                *self = next;
                Ok(delivery)
            }
            Err(error) => {
                next.trace.committed = false;
                next.trace.error = Some(error.to_string());
                self.trace = next.trace;
                Err(error)
            }
        }
    }

    fn select_text(&mut self, event: &Name) -> Result<Option<TextDelivery>> {
        for (id, asset) in self.graph.texts.clone() {
            if &asset.event != event {
                continue;
            }
            let site = TraceSite { asset: Some(id.clone()), ..TraceSite::default() };
            if !self.matches_at(asset.when.as_ref(), site.clone())? {
                continue;
            }
            let mut eligible = Vec::new();
            for entry in &asset.entries {
                let site = TraceSite { entry: Some(entry.id.clone()), ..site.clone() };
                if self.matches_at(entry.when.as_ref(), site)? {
                    eligible.push(entry.id.clone());
                }
            }
            if eligible.is_empty() {
                continue;
            }
            let mut progress = self.saved.texts.get(&id).cloned().unwrap_or_default();
            let Some(chosen) = pick(asset.repeat, &eligible, &mut progress, self.saved.seed, &id)
            else {
                // `Once` has run out. Another asset on the same event may still
                // have something to say, so this is a skip, not an answer.
                continue;
            };
            let entry = asset
                .entries
                .iter()
                .find(|candidate| candidate.id == chosen)
                .ok_or_else(|| Error::InvalidState(format!("missing text entry {chosen}")))?;

            let mut lines = Vec::new();
            for slot in &entry.lines {
                let mut selected = None;
                for variant in &slot.variants {
                    let site = TraceSite {
                        slot: Some(slot.id.clone()),
                        variant: Some(variant.id.clone()),
                        entry: Some(entry.id.clone()),
                        ..site.clone()
                    };
                    if self.matches_at(variant.when.as_ref(), site)? {
                        selected = Some(variant);
                        break;
                    }
                }
                let variant = selected.ok_or_else(|| Error::NoMatch(slot.id.clone()))?;
                lines.push(DeliveredLine {
                    slot: slot.id.clone(),
                    variant: variant.id.clone(),
                    speaker: slot.speaker.clone(),
                    text: variant.text.clone(),
                    revision: variant.revision.clone(),
                });
            }

            progress.plays = progress
                .plays
                .checked_add(1)
                .ok_or_else(|| Error::InvalidState("text play counter overflow".into()))?;
            let plays = progress.plays;
            self.saved.texts.insert(id.clone(), progress);
            return Ok(Some(TextDelivery {
                asset: id,
                kind: asset.kind,
                entry: entry.id.clone(),
                lines,
                plays,
            }));
        }
        Ok(None)
    }

    /// Reject a save whose repeat state does not describe this graph.
    ///
    /// Called from `restore` for the same reason visits are checked there: a
    /// counter naming an asset the graph no longer contains is either a save
    /// from another story or an edited file, and executing it would silently
    /// resume a playthrough that never happened.
    pub(crate) fn validate_texts(&self) -> Result<()> {
        for (id, progress) in &self.saved.texts {
            let asset = self
                .graph
                .texts
                .get(id)
                .ok_or_else(|| Error::InvalidState("invalid text history".into()))?;
            let entries = asset.entries.len() as u64;
            if progress.plays == 0
                || progress.position > entries
                || progress
                    .carry
                    .as_ref()
                    .is_some_and(|id| !asset.entries.iter().any(|entry| &entry.id == id))
            {
                return Err(Error::InvalidState("invalid text history".into()));
            }
        }
        Ok(())
    }
}

/// Apply one asset's repeat policy, advancing its progress.
///
/// `None` means the policy has nothing left to offer. Only [`RepeatPolicy::Once`]
/// can say that; the other three always have an answer while anything is
/// eligible, which is why they are safe defaults for content the player may
/// trigger indefinitely.
fn pick(
    policy: RepeatPolicy,
    eligible: &[String],
    progress: &mut TextProgress,
    seed: u64,
    asset: &str,
) -> Option<String> {
    let length = eligible.len() as u64;
    match policy {
        // Deliberately does not advance `position`: prose whose wording is a
        // function of state must give the same answer to the same state every
        // time it is read, or a quest log would change on being reopened.
        RepeatPolicy::First => eligible.first().cloned(),
        RepeatPolicy::Once => {
            // The eligible set can shrink between deliveries, which can put
            // `position` past its end. Treating that as exhausted is the
            // conservative reading: the alternative is repeating an entry the
            // player has already been shown.
            let chosen = eligible.get(progress.position as usize)?.clone();
            progress.position += 1;
            Some(chosen)
        }
        RepeatPolicy::Cycle => {
            let chosen = eligible[(progress.position % length) as usize].clone();
            progress.position = (progress.position + 1) % length;
            Some(chosen)
        }
        RepeatPolicy::Shuffle => {
            if progress.position >= length {
                progress.round += 1;
                progress.position = 0;
            }
            let order = shuffled(eligible, seed, asset, progress.round, progress.carry.as_deref());
            let chosen = order[progress.position as usize].clone();
            progress.position += 1;
            if progress.position >= order.len() as u64 {
                // Remember what closed the round now, while the order is in
                // hand; the next round's opener is chosen against it.
                progress.carry = Some(chosen.clone());
            }
            Some(chosen)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shuffle_is_a_permutation_of_the_eligible_entries() {
        let eligible: Vec<String> = (0..6).map(|n| format!("entry-{n}")).collect();
        let mut order = shuffled(&eligible, 42, "asset", 0, None);
        assert_ne!(
            order, eligible,
            "a six-entry shuffle that reproduces authored order is suspect"
        );
        order.sort();
        assert_eq!(order, eligible);
    }

    #[test]
    fn the_same_seed_asset_and_round_always_shuffle_the_same_way() {
        let eligible: Vec<String> = (0..8).map(|n| format!("entry-{n}")).collect();
        assert_eq!(
            shuffled(&eligible, 7, "asset", 3, None),
            shuffled(&eligible, 7, "asset", 3, None)
        );
        assert_ne!(
            shuffled(&eligible, 7, "asset", 3, None),
            shuffled(&eligible, 7, "asset", 4, None)
        );
        assert_ne!(
            shuffled(&eligible, 7, "asset", 3, None),
            shuffled(&eligible, 8, "asset", 3, None)
        );
        assert_ne!(shuffled(&eligible, 7, "one", 3, None), shuffled(&eligible, 7, "two", 3, None));
    }

    #[test]
    fn a_round_never_opens_with_the_entry_that_closed_the_last_one() {
        let eligible: Vec<String> = (0..4).map(|n| format!("entry-{n}")).collect();
        for round in 0..32 {
            let plain = shuffled(&eligible, 11, "asset", round, None);
            let guarded = shuffled(&eligible, 11, "asset", round, Some(&plain[0]));
            assert_ne!(guarded[0], plain[0]);
        }
    }

    #[test]
    fn a_single_eligible_entry_cannot_be_swapped_away_from() {
        let eligible = vec!["only".to_string()];
        assert_eq!(shuffled(&eligible, 1, "asset", 0, Some("only")), eligible);
    }

    #[test]
    fn shuffle_exhausts_every_entry_before_repeating_any() {
        let eligible: Vec<String> = (0..5).map(|n| format!("entry-{n}")).collect();
        let mut progress = TextProgress::default();
        let mut round: Vec<String> = Vec::new();
        for _ in 0..5 {
            round.push(pick(RepeatPolicy::Shuffle, &eligible, &mut progress, 3, "asset").unwrap());
        }
        let mut sorted = round.clone();
        sorted.sort();
        assert_eq!(sorted, eligible, "a bag round must contain each entry once");

        let next = pick(RepeatPolicy::Shuffle, &eligible, &mut progress, 3, "asset").unwrap();
        assert_ne!(next, round[4], "the new round must not repeat the previous line");
    }

    #[test]
    fn first_always_answers_the_same_way() {
        let eligible: Vec<String> = (0..3).map(|n| format!("entry-{n}")).collect();
        let mut progress = TextProgress::default();
        for _ in 0..4 {
            assert_eq!(
                pick(RepeatPolicy::First, &eligible, &mut progress, 0, "asset").as_deref(),
                Some("entry-0")
            );
        }
        assert_eq!(progress.position, 0);
    }

    #[test]
    fn once_runs_out_and_stays_out() {
        let eligible: Vec<String> = (0..2).map(|n| format!("entry-{n}")).collect();
        let mut progress = TextProgress::default();
        assert_eq!(
            pick(RepeatPolicy::Once, &eligible, &mut progress, 0, "a").as_deref(),
            Some("entry-0")
        );
        assert_eq!(
            pick(RepeatPolicy::Once, &eligible, &mut progress, 0, "a").as_deref(),
            Some("entry-1")
        );
        assert_eq!(pick(RepeatPolicy::Once, &eligible, &mut progress, 0, "a"), None);
        assert_eq!(pick(RepeatPolicy::Once, &eligible, &mut progress, 0, "a"), None);
    }

    #[test]
    fn cycle_wraps_in_authored_order() {
        let eligible: Vec<String> = (0..3).map(|n| format!("entry-{n}")).collect();
        let mut progress = TextProgress::default();
        let taken: Vec<String> = (0..4)
            .map(|_| pick(RepeatPolicy::Cycle, &eligible, &mut progress, 0, "a").unwrap())
            .collect();
        assert_eq!(taken, ["entry-0", "entry-1", "entry-2", "entry-0"]);
    }
}
