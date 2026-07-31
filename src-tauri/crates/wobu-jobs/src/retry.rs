//! When to try again, and the one question that decides it: did the attempt
//! that just failed cost the user money?
//!
//! ## What the queue can know, and what it cannot
//!
//! It cannot know what a call costs. Prices are per model, per token, per
//! megapixel, per second of GPU, and they move; a queue that tried to price a
//! retry would be wrong in the expensive direction the week after someone
//! changed a model id.
//!
//! What it *can* know is whether money already moved, because the thing that
//! made the call is the thing that reports the failure and it has the usage
//! figures in its hand. That single bit ([`Billed`]) is enough, and it splits
//! "retryable" into the two decisions it has always secretly been:
//!
//! - **A rate limit, a 503, a refused connection.** Nothing was generated and
//!   nothing was charged. Waiting and trying again spends time, not money.
//!   The queue does this on its own, and should — the alternative is a
//!   transient blip becoming a dialog the user has to dismiss.
//! - **A truncated response, JSON that will not parse, a section the model left
//!   out.** The provider generated and billed for every one of those tokens.
//!   "Try again" here is a decision to spend again, and `Error::is_retryable`
//!   answering `true` means *it could work*, not *go ahead*. So by default the
//!   queue stops and says so ([`Verdict::Hold`]).
//!
//! That is the whole of "never auto-retry something that costs money without
//! saying so", and note that it does not say *never*. A submitter that knows one
//! bad-JSON retry is worth it sets [`RetryPolicy::paid_attempts`], and then the
//! queue does retry — after emitting `job:retry` with `costsMoney: true`, before
//! the wait, so the saying-so happens while the money is still unspent.
//!
//! ## Where the bit comes from
//!
//! [`Failure::from_provider`] takes the error *and* the usage, and prefers the
//! usage: any tokens at all means charged, whatever the error says. Only when
//! there are no figures does it fall back to which half of `wobu_llm::Error` the
//! variant sits in — that enum is already split into "the call" and "the
//! answer", and the split happens to be exactly this one. The fallback is a
//! guess, though, and it is wrong in one real case: a stream that dies mid
//! response has produced billable output while looking like a transport error.
//! That is why usage wins, and why a task holding an `EnhanceOutcome` must pass
//! its `usage` rather than convert the bare error.

use std::time::Duration;

use serde::{Serialize, Serializer};
use wobu_llm::{Error as ProviderError, Usage};

/// Whether an attempt cost the user anything.
///
/// [`Billed::Unknown`] deliberately behaves as [`Billed::Charged`] everywhere:
/// the failure being designed against is spending silently, so uncertainty has
/// to fall on the side of asking. A task that knows nothing was spent has to say
/// so explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Billed {
    /// Nothing left the machine, or the provider refused before doing any work.
    Nothing,
    /// Money moved. Trying again will move some more.
    Charged,
    /// Nobody can tell — a panic, an adapter that reported no figures. Treated
    /// as charged.
    Unknown,
}

impl Billed {
    /// Whether another attempt would be spending the user's money again.
    pub fn spends_money(self) -> bool {
        !matches!(self, Billed::Nothing)
    }
}

/// Why an attempt failed, in the terms the queue reasons about.
///
/// A flattened, serialisable echo of whatever the task's own error type was:
/// `code` is the same stable dotted string `src-tauri/src/error.rs` defines and
/// the frontend already switches on, so a job failure and a command failure
/// reach the UI describable by the same code. This crate does not own that
/// taxonomy and does not try to — the string is passed through.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub code: String,
    /// One sentence for a person. Scrubbed of anything key-shaped at the shell
    /// boundary before it is emitted, the same as every other message.
    pub message: String,
    /// Whether trying the same thing again could plausibly work. Says nothing
    /// about whether it *should* — that is [`Billed`]'s question.
    pub retryable: bool,
    /// The technical remainder, for the log and for someone copying it into an
    /// issue. Never required to understand the failure, because the UI does not
    /// always show it — the same contract as `WobuError::detail`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The provider's own wait hint, when it gave one. Milliseconds on the wire
    /// because `Duration` serialises as a struct nobody on the other side wants
    /// to read.
    #[serde(serialize_with = "millis", skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<Duration>,
    pub billed: Billed,
    /// What it cost, in the task's own words — "1,204 in + 380 out". The queue
    /// cannot price a call, so if the user is going to be asked whether to spend
    /// again, this is the only thing on screen that says what "again" means.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_note: Option<String>,
}

impl Failure {
    /// A failure that cost nothing and is not worth repeating — the default a
    /// local task reaches for.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Failure {
        Failure {
            code: code.into(),
            message: message.into(),
            retryable: false,
            detail: None,
            retry_after: None,
            billed: Billed::Nothing,
            cost_note: None,
        }
    }

    pub fn retryable(mut self, retryable: bool) -> Failure {
        self.retryable = retryable;
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Failure {
        self.detail = Some(detail.into());
        self
    }

    pub fn billed(mut self, billed: Billed) -> Failure {
        self.billed = billed;
        self
    }

    pub fn after(mut self, retry_after: Duration) -> Failure {
        self.retry_after = Some(retry_after);
        self
    }

    pub fn cost_note(mut self, note: impl Into<String>) -> Failure {
        self.cost_note = Some(note.into());
        self
    }

    /// From a provider error and what that attempt is known to have cost.
    ///
    /// Usage first, error second, and that order is the point: a stream that
    /// dropped after eight hundred tokens is `Unavailable`, which reads as a
    /// free transport blip and is not one. Pass the `usage` off an
    /// `EnhanceOutcome` and the queue gets it right without the adapter having
    /// to think about queues.
    pub fn from_provider(error: &ProviderError, usage: Usage) -> Failure {
        let billed =
            if usage.total_tokens() > 0 { Billed::Charged } else { unbilled_by_kind(error) };
        let retry_after = match error {
            ProviderError::RateLimited { retry_after, .. } => *retry_after,
            _ => None,
        };
        Failure {
            code: error.code().to_owned(),
            // The `#[error(…)]` copy on each variant is already written for a
            // person, and restating it here would leave two versions to drift —
            // the same argument `WobuError::from(StoreError)` makes.
            message: error.to_string(),
            retryable: error.is_retryable(),
            detail: None,
            retry_after,
            billed,
            cost_note: None,
        }
    }
}

impl From<&ProviderError> for Failure {
    /// For the callers that genuinely have no usage figure. Everything holding
    /// an `EnhanceOutcome` has one and should use
    /// [`Failure::from_provider`] instead.
    fn from(error: &ProviderError) -> Failure {
        Failure::from_provider(error, Usage::default())
    }
}

/// Which half of `wobu_llm::Error` a variant lives in, read as a billing guess.
///
/// The enum is already split into "the call" — a key, a quota, a socket, a
/// schema the provider would not take — and "the answer", where the call
/// succeeded and the output is unusable. Everything in the second group was
/// generated, and generation is what providers charge for.
///
/// Exhaustive on purpose: a variant added to that enum should have to state
/// which side it falls on here, because the alternative is a new failure
/// quietly defaulting to free and being retried on the user's card.
fn unbilled_by_kind(error: &ProviderError) -> Billed {
    match error {
        ProviderError::NoKey { .. }
        | ProviderError::BadKey { .. }
        | ProviderError::RateLimited { .. }
        | ProviderError::BillingRequired { .. }
        | ProviderError::ContextTooLong
        | ProviderError::SchemaRejected { .. }
        | ProviderError::Unavailable { .. }
        // Cancelled with no usage reported means the request never got far
        // enough to be charged; an adapter that had figures would have passed
        // them, and then the branch above this one answers instead.
        | ProviderError::Cancelled => Billed::Nothing,

        ProviderError::Truncated
        | ProviderError::NotJson(_)
        | ProviderError::NotAnObject { .. }
        | ProviderError::MissingSection { .. }
        | ProviderError::WrongSectionType { .. }
        | ProviderError::EmptySection { .. }
        | ProviderError::NotAHexColor { .. } => Billed::Charged,
    }
}

/// How hard to try, and how much of the user's money the queue is allowed to
/// spend doing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Attempts in total, the first one included. `1` means never retry.
    pub max_attempts: u32,
    /// How many of those attempts the queue may start knowing the previous one
    /// was billed.
    ///
    /// Zero by default, which is the whole argument of this module: a paid
    /// retry is a decision, and the default is to hand it to whoever is paying.
    /// Above zero it becomes a decision the *submitter* made in advance — which
    /// is legitimate, one re-ask of a model that produced malformed JSON being
    /// the obvious case — and each one is still announced before it happens.
    pub paid_attempts: u32,
    /// First wait. Doubles per attempt.
    pub base_delay: Duration,
    /// Ceiling on the waits this crate invents. Not a ceiling on the ones a
    /// provider asks for: see [`delay_for`].
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> RetryPolicy {
        RetryPolicy {
            // Four attempts over roughly seven seconds of waiting. Enough to
            // ride out a deploy or a burst limit, few enough that a provider
            // that is genuinely down is reported as down rather than silently
            // retried for a minute while the user watches a spinner.
            max_attempts: 4,
            paid_attempts: 0,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// For work that must not be repeated at all — anything with a side effect
    /// the second run would duplicate.
    pub fn never() -> RetryPolicy {
        RetryPolicy { max_attempts: 1, ..RetryPolicy::default() }
    }

    /// Allow `n` attempts that the queue knows will be billed, each announced
    /// before it happens.
    pub fn with_paid_attempts(mut self, n: u32) -> RetryPolicy {
        self.paid_attempts = n;
        self
    }
}

/// What has been spent on a job so far, in attempts rather than currency.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Attempts {
    /// Attempts started, including the one that just failed.
    pub total: u32,
    /// How many of those the queue started knowing the previous attempt had
    /// been billed.
    pub paid: u32,
}

/// What to do about a failed attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Final. Either the failure is not retryable or the attempts are used up.
    Stop,
    /// Wait this long and go again. Nothing was billed, so the queue does it
    /// without asking.
    Free(Duration),
    /// Wait this long and go again, and it will cost money. Only ever returned
    /// when the submitter asked for it, and always announced first.
    Paid(Duration),
    /// Retryable, and repeating it costs money, and no paid attempts were
    /// allowed. The queue stops and the decision goes to the person paying.
    ///
    /// Distinct from [`Verdict::Stop`] because the two look identical in a log
    /// and are opposites in the UI: one is a dead end, the other is a question
    /// with a "yes" that nobody has asked yet.
    Hold,
}

impl Verdict {
    pub fn delay(self) -> Option<Duration> {
        match self {
            Verdict::Free(d) | Verdict::Paid(d) => Some(d),
            Verdict::Stop | Verdict::Hold => None,
        }
    }

    pub fn costs_money(self) -> bool {
        matches!(self, Verdict::Paid(_))
    }
}

/// The whole retry decision, as a pure function.
///
/// Pure and public so it can be tested without a runtime, a task or a clock —
/// this is the rule about spending other people's money, and it should not need
/// a queue to be asked what it thinks.
pub fn decide(policy: &RetryPolicy, attempts: Attempts, failure: &Failure) -> Verdict {
    if !failure.retryable {
        return Verdict::Stop;
    }
    if attempts.total >= policy.max_attempts {
        return Verdict::Stop;
    }
    let delay = delay_for(policy, attempts.total, failure);
    if !failure.billed.spends_money() {
        return Verdict::Free(delay);
    }
    if attempts.paid < policy.paid_attempts { Verdict::Paid(delay) } else { Verdict::Hold }
}

/// A provider that asks for a wait longer than this is telling us to come back
/// later, not to hold a job open. A job parked for an hour is indistinguishable
/// from a hung one, and the user can resubmit.
const MAX_PROVIDER_WAIT: Duration = Duration::from_secs(300);

/// How long to wait before attempt `attempts_made + 1`.
///
/// Exponential from `base_delay`, and then the provider's own hint is applied as
/// a floor rather than as a replacement. Both halves matter: taking only the
/// hint means a provider that says "100ms" gets hammered on the fourth
/// consecutive failure, and taking only the backoff means ignoring a service
/// that told us exactly when it would be ready and getting another 429 for it.
///
/// `max_delay` caps the number this crate invented. It does not cap the
/// provider's, because that one is not a guess — clamping a documented
/// forty-second wait down to thirty buys a guaranteed second rate limit.
/// [`MAX_PROVIDER_WAIT`] is the only ceiling over it.
fn delay_for(policy: &RetryPolicy, attempts_made: u32, failure: &Failure) -> Duration {
    let doublings = attempts_made.saturating_sub(1).min(16);
    let backoff = policy.base_delay.saturating_mul(1u32 << doublings).min(policy.max_delay);
    match failure.retry_after {
        Some(hint) => hint.max(backoff).min(MAX_PROVIDER_WAIT),
        None => backoff,
    }
}

/// `Option<Duration>` as whole milliseconds. `Duration`'s own representation is
/// `{ secs, nanos }`, which is not a shape any of the TypeScript wants.
fn millis<S: Serializer>(value: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(d) => serializer.serialize_some(&(d.as_millis() as u64)),
        None => serializer.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate_limited(after: Option<Duration>) -> Failure {
        Failure::from_provider(
            &ProviderError::RateLimited { provider: "Anthropic", retry_after: after },
            Usage::default(),
        )
    }

    /// `n` attempts made, none of them knowingly paid for.
    fn made(n: u32) -> Attempts {
        Attempts { total: n, paid: 0 }
    }

    /// A response the model was paid to produce and that cannot be used.
    fn garbage() -> Failure {
        Failure::from_provider(
            &ProviderError::Truncated,
            Usage { input_tokens: 800, cached_input_tokens: 0, output_tokens: 400 },
        )
    }

    #[test]
    fn a_rate_limit_is_retried_on_the_queues_own_initiative() {
        // Nothing was generated, so nothing was charged, so waiting and going
        // again spends time and not money. Making the user click through this
        // would be the app arguing with a blip.
        let verdict = decide(&RetryPolicy::default(), made(1), &rate_limited(None));
        assert!(matches!(verdict, Verdict::Free(_)));
        assert!(!verdict.costs_money());
    }

    #[test]
    fn a_response_that_was_paid_for_and_came_back_broken_is_held_rather_than_repeated() {
        // The regression this whole module exists for. `Truncated.is_retryable()`
        // is true — it *could* work — but the provider already billed for the
        // tokens it produced, and retrying without asking spends the user's
        // money on a hunch.
        assert!(garbage().retryable);
        let verdict = decide(&RetryPolicy::default(), made(1), &garbage());
        assert_eq!(verdict, Verdict::Hold);
        assert_eq!(verdict.delay(), None);
    }

    #[test]
    fn a_paid_retry_happens_only_when_the_submitter_asked_for_one() {
        // "Never without saying so" is not "never". A submitter can decide in
        // advance that one re-ask of a model that produced malformed JSON is
        // worth it; what it cannot do is happen by accident.
        let policy = RetryPolicy::default().with_paid_attempts(1);
        let first = decide(&policy, made(1), &garbage());
        assert!(matches!(first, Verdict::Paid(_)));
        assert!(first.costs_money());
        // And the allowance is spent, rather than being a per-attempt licence.
        let second = decide(&policy, Attempts { total: 2, paid: 1 }, &garbage());
        assert_eq!(second, Verdict::Hold);
    }

    #[test]
    fn an_unknown_cost_is_treated_as_a_cost() {
        // A task that cannot tell must not be read as "free". Silent spending is
        // the failure being designed against, so uncertainty falls towards
        // asking.
        let failure = Failure::new("provider.bad_response", "…")
            .retryable(true)
            .billed(Billed::Unknown);
        assert!(Billed::Unknown.spends_money());
        assert_eq!(decide(&RetryPolicy::default(), Attempts::default(), &failure), Verdict::Hold);
    }

    #[test]
    fn nothing_retryable_survives_the_attempt_ceiling() {
        // Otherwise a provider that is down turns into an unbounded retry loop
        // that looks, from the outside, exactly like the app having hung.
        let policy = RetryPolicy { max_attempts: 3, ..RetryPolicy::default() };
        assert!(matches!(decide(&policy, made(2), &rate_limited(None)), Verdict::Free(_)));
        assert_eq!(decide(&policy, made(3), &rate_limited(None)), Verdict::Stop);
    }

    #[test]
    fn a_cancellation_is_never_retried() {
        // Pressing Stop and being billed for a retry is the specific outcome
        // this prevents. `wobu-llm` says cancellation is not retryable and the
        // queue takes it at its word rather than having a second opinion.
        let failure = Failure::from(&ProviderError::Cancelled);
        assert!(!failure.retryable);
        assert_eq!(decide(&RetryPolicy::default(), Attempts::default(), &failure), Verdict::Stop);
    }

    #[test]
    fn the_wait_doubles_and_then_stops_growing() {
        // A backoff without a ceiling reaches minutes by the fifth attempt,
        // which is a job the user will assume is stuck.
        let policy = RetryPolicy {
            max_attempts: 20,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(8),
            paid_attempts: 0,
        };
        let waits: Vec<Duration> = (1..=6)
            .map(|n| decide(&policy, made(n), &rate_limited(None)).delay().unwrap())
            .collect();
        assert_eq!(
            waits,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(8),
                Duration::from_secs(8),
            ],
        );
    }

    #[test]
    fn a_providers_own_wait_beats_our_ceiling_but_a_short_one_does_not_beat_our_backoff() {
        // Both halves of the hint rule. Google sends `retryDelay: 42s`; clamping
        // that to our 30-second ceiling buys a guaranteed second 429. And a
        // provider that says "100ms" on the fourth consecutive failure is not a
        // reason to stop backing off.
        let policy = RetryPolicy::default();
        let long = rate_limited(Some(Duration::from_secs(42)));
        assert_eq!(
            decide(&policy, made(1), &long).delay(),
            Some(Duration::from_secs(42)),
        );
        let short = rate_limited(Some(Duration::from_millis(100)));
        assert_eq!(
            decide(&policy, made(3), &short).delay(),
            Some(Duration::from_secs(4)),
        );
    }

    #[test]
    fn an_absurd_provider_wait_is_not_honoured_indefinitely() {
        // A job parked for an hour is indistinguishable from a hung one, and the
        // queue slot it is not holding is no comfort to the person watching.
        let hour = rate_limited(Some(Duration::from_secs(3600)));
        assert_eq!(
            decide(&RetryPolicy::default(), Attempts::default(), &hour).delay(),
            Some(MAX_PROVIDER_WAIT),
        );
    }

    #[test]
    fn usage_decides_the_billing_question_before_the_error_variant_does() {
        // The case the error taxonomy alone gets wrong: a stream that dies after
        // eight hundred tokens is `Unavailable`, which reads as a free transport
        // blip and is nothing of the sort.
        let dropped = ProviderError::Unavailable { detail: "connection reset".into() };
        let free = Failure::from_provider(&dropped, Usage::default());
        assert_eq!(free.billed, Billed::Nothing);
        let paid = Failure::from_provider(&dropped, Usage {
            input_tokens: 812,
            cached_input_tokens: 0,
            output_tokens: 90,
        });
        assert_eq!(paid.billed, Billed::Charged);
        assert_eq!(decide(&RetryPolicy::default(), Attempts::default(), &paid), Verdict::Hold);
    }

    #[test]
    fn a_failure_carries_the_same_code_the_ui_already_switches_on() {
        // The job path and the command path have to describe the same failure
        // the same way, or `errorSurface` in `src/lib/api.ts` sends a user to
        // Settings for one and nowhere for the other.
        let failure = Failure::from(&ProviderError::BadKey { provider: "Anthropic" });
        assert_eq!(failure.code, "provider.bad_key");
        assert!(!failure.retryable);
        assert_eq!(failure.billed, Billed::Nothing);
    }

    #[test]
    fn a_failure_serialises_its_wait_as_milliseconds() {
        // `Duration` serialises as `{ secs, nanos }`, which no TypeScript in
        // this repo describes and none should have to.
        let json = serde_json::to_value(rate_limited(Some(Duration::from_millis(1500)))).unwrap();
        assert_eq!(json["retryAfter"], 1500);
        assert_eq!(json["billed"], "nothing");
        assert!(json.get("costNote").is_none(), "an absent note is absent, not null");
    }
}
