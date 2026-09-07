//! The three independent dimensions of a piece of authored text.
//!
//! They are three fields and not one enum, and that is the load-bearing
//! decision in this module. A single `TextState` would have to spell out the
//! product — `GeneratedDraftCurrent`, `LockedApprovedOutOfDate`, and ten more —
//! and every transition would then have to remember to carry the two dimensions
//! it was not changing. It would not merely be verbose; it would lose
//! information the first time somebody wrote the wrong combined variant, and the
//! two cases that matter most are exactly the ones a combined enum gets wrong:
//!
//! - Locking a line must not make it look current again (US-06). Freshness is
//!   derived from what the text depends on, and locking changes none of that.
//! - Unlocking a line must not change the words or discard the approval that
//!   was recorded against them (#151). It changes one dimension only.
//!
//! What sets each dimension is elsewhere. Freshness is *derived* — by the
//! dependency tracking of #168, from source and context that lives outside this
//! crate — so nothing here computes it; the field is a place to record the
//! answer, in the same spirit as `wobu_core::DescriptionState::Stale`.

use serde::{Deserialize, Serialize};

/// Whether generation may write here, and what happened last time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationPolicy {
    /// A model wrote this and nobody has changed it. Eligible to be replaced
    /// outright.
    Generated,
    /// A person wrote or rewrote this. A generation job may propose a
    /// replacement beside it; it may not overwrite it.
    ///
    /// The default, because a slot that arrives with no policy stated came from
    /// somewhere this crate cannot see, and treating unknown provenance as
    /// freely replaceable is the one mistake that destroys work.
    #[default]
    Edited,
    /// Off limits. A locked slot cannot enter a generation job at all, and that
    /// has to hold for a caller that never went near the UI (US-05).
    Locked,
}

/// Whether a reviewer has signed off on this exact wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    #[default]
    Draft,
    /// Approved. The approval is against a specific
    /// [`Revision`](crate::Revision) and the context that was current when it
    /// was given, which is why changing either has to withdraw it.
    Approved,
}

/// Whether what this text was written against still holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    #[default]
    Current,
    /// Something upstream moved. Applies to locked text as much as to generated
    /// text: locked wording that no longer fits its context is a problem a
    /// reviewer has to resolve, not one the lock makes disappear.
    OutOfDate,
}

/// The three dimensions together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ContentLifecycle {
    #[serde(default)]
    pub policy: GenerationPolicy,
    #[serde(default)]
    pub review: ReviewState,
    #[serde(default)]
    pub freshness: Freshness,
}

impl ContentLifecycle {
    /// A line a person just typed: theirs, unreviewed, and current.
    pub fn hand_written() -> ContentLifecycle {
        ContentLifecycle::default()
    }

    /// A line a person typed and immediately locked — US-03's writer, who never
    /// configures a provider at all.
    pub fn hand_written_locked() -> ContentLifecycle {
        ContentLifecycle { policy: GenerationPolicy::Locked, ..ContentLifecycle::default() }
    }

    /// Whether a generation job is allowed to produce a replacement for this.
    ///
    /// The single question the job planner has to ask, named here so that the
    /// answer is defined once rather than re-derived from the policy enum at
    /// each of its call sites.
    pub fn may_generate(self) -> bool {
        self.policy != GenerationPolicy::Locked
    }

    /// Whether this may ship in a release build without a human looking at it
    /// again. Approval alone is not enough: an approval attests to a wording
    /// *and* the context it was read in, and stale context withdraws that.
    pub fn is_release_ready(self) -> bool {
        self.review == ReviewState::Approved && self.freshness == Freshness::Current
    }

    /// Record that something upstream changed.
    ///
    /// Touches freshness and nothing else — in particular not the policy, so a
    /// stale locked line stays locked and visible rather than becoming eligible
    /// for silent replacement.
    pub fn mark_out_of_date(&mut self) {
        self.freshness = Freshness::OutOfDate;
    }

    /// A reviewer affirming that unchanged wording still fits a changed context.
    ///
    /// This is a decision, not an absence of one, which is why it is a method
    /// rather than something the freshness recomputation does on its own.
    pub fn affirm_still_current(&mut self) {
        self.freshness = Freshness::Current;
    }

    /// Change only the policy.
    ///
    /// Named `set_policy` rather than `lock`/`unlock` so that the thing it does
    /// not do is visible at the call site: an unlock that also cleared the
    /// review state would discard an approval nobody withdrew, and a lock that
    /// also cleared the freshness would hide the very mismatch #151 requires
    /// stays visible.
    pub fn set_policy(&mut self, policy: GenerationPolicy) {
        self.policy = policy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locking_does_not_make_stale_text_look_current() {
        let mut lifecycle = ContentLifecycle::default();
        lifecycle.mark_out_of_date();
        lifecycle.set_policy(GenerationPolicy::Locked);
        assert_eq!(lifecycle.freshness, Freshness::OutOfDate);
        assert!(!lifecycle.is_release_ready());
    }

    #[test]
    fn unlocking_keeps_the_approval_and_the_words() {
        let mut lifecycle = ContentLifecycle {
            policy: GenerationPolicy::Locked,
            review: ReviewState::Approved,
            freshness: Freshness::Current,
        };
        lifecycle.set_policy(GenerationPolicy::Edited);
        assert_eq!(lifecycle.review, ReviewState::Approved);
        assert!(lifecycle.is_release_ready());
    }

    #[test]
    fn locked_text_is_never_eligible_for_generation() {
        assert!(!ContentLifecycle::hand_written_locked().may_generate());
        assert!(ContentLifecycle::hand_written().may_generate());
    }

    #[test]
    fn approval_alone_is_not_release_ready() {
        let mut lifecycle =
            ContentLifecycle { review: ReviewState::Approved, ..ContentLifecycle::default() };
        assert!(lifecycle.is_release_ready());
        lifecycle.mark_out_of_date();
        assert!(!lifecycle.is_release_ready());
        lifecycle.affirm_still_current();
        assert!(lifecycle.is_release_ready());
    }

    #[test]
    fn unknown_provenance_defaults_to_the_cautious_policy() {
        // A slot with no policy in the file must not be freely overwritable.
        assert_eq!(GenerationPolicy::default(), GenerationPolicy::Edited);
    }
}
