use thiserror::Error;

use std::time::Duration;

pub type Result<T> = std::result::Result<T, Error>;

/// What can go wrong between asking a backend for a picture and having one we
/// are willing to write into `assets/`.
///
/// The same two groups as `wobu_llm::Error`, and the split means the same thing,
/// which is the point of keeping them side by side — the job queue reads it as
/// a billing guess ([`wobu_jobs::Billed`], `retry.rs`):
///
/// - **The call**: a key, a quota, a socket, a ComfyUI that is not running.
///   Nothing was generated, so nothing was charged. Every vendor spells these
///   differently and normalising them is most of what an adapter is for.
/// - **The answer**: the call succeeded and what came back is not an image we
///   can keep. A provider that generated pixels has billed for them whether or
///   not we could read them, which is why these sit on the other side of the
///   line from a refused connection.
///
/// There is no variant for "this backend cannot do what the request asks" that
/// an adapter is expected to raise routinely. [`Error::Unsupported`] exists, and
/// it is `internal`: [`negotiate`](crate::negotiate) is what stops a request
/// naming a capability the backend does not have, so reaching it means the
/// negotiation and the adapter disagree, and that is our bug rather than the
/// user's.
#[derive(Debug, Error)]
pub enum Error {
    // ── the call ─────────────────────────────────────────────────────────
    /// Raised by key resolution rather than by an adapter, exactly as in
    /// `wobu-llm`: an adapter is constructed with whatever credentials it
    /// needs, so by the time one exists this has been settled. It lives here so
    /// the generate path has one error type end to end.
    #[error("no API key for {backend} on this machine")]
    NoKey { backend: &'static str },

    #[error("{backend} rejected the API key")]
    BadKey { backend: &'static str },

    /// `retry_after` is the backend's own hint in whatever form it gave it.
    /// Carried rather than slept on here because the queue owns backoff and has
    /// to weigh this against the rest of the queue.
    #[error("{backend} is rate limiting this key")]
    RateLimited { backend: &'static str, retry_after: Option<Duration> },

    /// The account has no billing enabled, which for Gemini image generation is
    /// the failure a working key produces on its first Generate — text is free
    /// there and images are not (`docs/08-providers.md`). Distinct from
    /// [`Capabilities::requires_billing`](crate::Capabilities::requires_billing),
    /// which says the call *costs* money; this says the account cannot pay.
    #[error("{backend} needs billing enabled on the account before this model will run")]
    BillingRequired { backend: &'static str },

    /// Unreachable, timed out, or a 5xx — and also the ComfyUI that is running
    /// happily and has never heard of the checkpoint the request names. Both
    /// are "the backend is not able to serve this right now, and waiting is a
    /// sensible thing to do", which is the property the queue acts on; in the
    /// checkpoint case the waiting is the user installing it, and "Try again"
    /// is then the right button. `detail` is what tells them which of the two
    /// it was, so it must name the model when it is the second.
    #[error("the backend could not be reached: {detail}")]
    Unavailable { detail: String },

    /// The user stopped it. Never retried — a queue that retries a cancellation
    /// is a queue that bills the user for pressing Stop.
    #[error("the generation was cancelled")]
    Cancelled,

    /// The request asks for something this backend cannot do.
    ///
    /// Always our bug, never the user's, and the same shape as
    /// `wobu_llm::Error::SchemaRejected`: [`Capabilities`](crate::Capabilities)
    /// declares what a backend takes and [`negotiate`](crate::negotiate) is a
    /// total function over it, so an adapter that has to raise this has been
    /// handed a request the negotiation should have reshaped. The message is
    /// aimed at whoever reads the log.
    #[error("this backend cannot honour the request: {detail}")]
    Unsupported { detail: String },

    // ── the answer ───────────────────────────────────────────────────────
    /// A content filter declined to generate. Its own variant because it is the
    /// one image failure with nothing wrong on our side and nothing broken on
    /// theirs, and the only useful answer is to say which words it objected to.
    ///
    /// Retryable, which is not obvious. Google's blocking is not deterministic —
    /// the same prompt does pass on a later attempt — so "it could work" is
    /// true. Whether it is *worth* doing is [`ImageUsage`](crate::ImageUsage)'s
    /// question, and a refusal that was billed is held by the queue for the
    /// person paying to decide, which is the behaviour we want.
    #[error("the backend refused to generate this image: {detail}")]
    Refused { detail: String },

    /// The call finished, reported no error, and produced nothing. Separate
    /// from [`Error::Refused`] because a silent empty result and a stated
    /// refusal send the user to two different places — the second one is a
    /// prompt to edit and the first one is a bug report.
    #[error("the backend returned no image")]
    NoImage,

    /// Bytes arrived and are not an image we can decode. `wobu-imagine` does no
    /// IO and does not decode; an adapter raises this when the container is
    /// unreadable at the point it reads back the dimensions, which
    /// `docs/08-providers.md` requires it to do rather than trusting the ones it
    /// asked for.
    #[error("the backend returned something that is not a readable image: {detail}")]
    NotAnImage { detail: String },
}

impl Error {
    /// Whether asking the same backend the same question again could work.
    ///
    /// Matched exhaustively so every new variant has to state which side of the
    /// line it falls on. Says nothing about whether a retry *should* happen —
    /// that is [`ImageUsage`](crate::ImageUsage)'s bit, and the queue weighs the
    /// two together.
    ///
    /// It must agree with the retryability of [`Error::code`]'s answer, or the
    /// queue and the "Try again" button will disagree about the same failure.
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::NoKey { .. }
            | Error::BadKey { .. }
            | Error::BillingRequired { .. }
            | Error::Unsupported { .. }
            | Error::Cancelled => false,

            Error::RateLimited { .. }
            | Error::Unavailable { .. }
            | Error::Refused { .. }
            | Error::NoImage
            | Error::NotAnImage { .. } => true,
        }
    }

    /// The stable dotted code this failure reaches the UI as.
    ///
    /// These strings are the serde renames on `Code` in `src-tauri/src/error.rs`,
    /// which reserved the whole `provider.` group for `wobu-llm` and
    /// `wobu-imagine` in advance so that neither would invent its own shapes.
    /// They are copied rather than shared because that enum lives in the Tauri
    /// shell and this crate must not depend on the shell —
    /// `every_failure_lands_on_a_code_the_ui_already_knows` is what notices a
    /// drift, and it notices it here rather than in front of a user.
    ///
    /// Nothing here lands on `asset.not_an_image` despite [`Error::NotAnImage`]
    /// reading like it should: that code is for a file the *user* imported and
    /// can convert, and pointing it at a provider's response would offer them a
    /// fix for somebody else's bug.
    pub fn code(&self) -> &'static str {
        match self {
            Error::NoKey { .. } => "provider.no_key",
            Error::BadKey { .. } => "provider.bad_key",
            Error::RateLimited { .. } => "provider.rate_limited",
            Error::BillingRequired { .. } => "provider.billing_required",
            Error::Unavailable { .. } => "provider.unavailable",
            // Our capabilities, our negotiation, our bug — the taxonomy's word
            // for that is `internal`.
            Error::Unsupported { .. } => "internal",
            Error::Cancelled => "cancelled",
            Error::Refused { .. } | Error::NoImage | Error::NotAnImage { .. } => {
                "provider.bad_response"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One of every variant. Hand-listed, which is safe because `code` and
    /// `is_retryable` are exhaustive matches: a new variant cannot compile
    /// without answering both questions. What this list is for is checking the
    /// answers, which the compiler cannot.
    fn one_of_each() -> Vec<Error> {
        vec![
            Error::NoKey { backend: "Gemini" },
            Error::BadKey { backend: "Gemini" },
            Error::RateLimited { backend: "Gemini", retry_after: Some(Duration::from_secs(30)) },
            Error::BillingRequired { backend: "Gemini" },
            Error::Unavailable { detail: "connection refused on 127.0.0.1:8188".into() },
            Error::Unsupported { detail: "21:9 is not in this backend's aspect ratios".into() },
            Error::Cancelled,
            Error::Refused { detail: "prohibited_content".into() },
            Error::NoImage,
            Error::NotAnImage { detail: "no PNG signature".into() },
        ]
    }

    /// The codes `Code` in `src-tauri/src/error.rs` defines, and which of them
    /// are retryable. Copied deliberately, and copied to match `wobu-llm`'s own
    /// copy of the same table: this crate cannot see that enum, so the copy is
    /// the only thing that can notice a drift, and two provider crates that
    /// disagreed with each other about one code would be worse than either
    /// disagreeing with the shell.
    const KNOWN_CODES: &[(&str, bool)] = &[
        ("provider.no_key", false),
        ("provider.bad_key", false),
        ("provider.billing_required", false),
        ("provider.rate_limited", true),
        ("provider.unavailable", true),
        ("provider.bad_response", true),
        ("provider.context_too_long", false),
        ("cancelled", false),
        ("internal", false),
    ];

    #[test]
    fn every_failure_lands_on_a_code_the_ui_already_knows() {
        // The regression: an adapter raising an error the command layer has no
        // code for, which reaches the webview as `internal` and tells the user
        // nothing about a key they could paste in Settings.
        for error in one_of_each() {
            assert!(
                KNOWN_CODES.iter().any(|(code, _)| *code == error.code()),
                "{error} lands on {}, which no code defines",
                error.code(),
            );
        }
    }

    #[test]
    fn retryability_agrees_with_the_code_it_lands_on() {
        // The queue reads `is_retryable`, the "Try again" button reads the
        // code's own `retryable`. If they disagree, one of them is lying to the
        // user about a call that costs money.
        for error in one_of_each() {
            let (_, code_is_retryable) =
                KNOWN_CODES.iter().find(|(code, _)| *code == error.code()).unwrap();
            assert_eq!(
                error.is_retryable(),
                *code_is_retryable,
                "{error} is retryable={} but its code {} says {code_is_retryable}",
                error.is_retryable(),
                error.code(),
            );
        }
    }

    #[test]
    fn a_cancelled_generation_is_never_retried() {
        // Pressing Stop and being billed for a retry is the specific outcome
        // this exists to prevent — and an image retry costs whole cents rather
        // than fractions of one, so it is the more expensive version of the
        // failure `wobu-llm` guards against with the same test.
        assert!(!Error::Cancelled.is_retryable());
        assert_eq!(Error::Cancelled.code(), "cancelled");
    }

    #[test]
    fn a_refusal_is_worth_another_call_and_a_missing_key_is_not() {
        // Both arrive as a 4xx from Gemini and the difference is not in the
        // status code. Lumping them together produces either a dead end for a
        // prompt that would pass on the next attempt, or a key being hammered.
        assert!(Error::Refused { detail: "safety".into() }.is_retryable());
        assert!(!Error::NoKey { backend: "Gemini" }.is_retryable());
        assert!(!Error::BillingRequired { backend: "Gemini" }.is_retryable());
    }

    #[test]
    fn a_request_this_backend_cannot_honour_is_reported_as_our_bug() {
        // `Unsupported` is unreachable if `negotiate` is doing its job, so it
        // must not offer the user a "Try again" for a request that will be
        // reshaped identically and refused identically.
        let error = Error::Unsupported { detail: "no controlnet".into() };
        assert_eq!(error.code(), "internal");
        assert!(!error.is_retryable());
    }

    #[test]
    fn error_messages_do_not_name_the_key() {
        // `redact::scrub` at the command boundary is the real guard, but an
        // error that carries a key at all has already put it in a log line.
        for error in one_of_each() {
            let message = error.to_string();
            assert!(!message.contains("sk-"), "{message}");
            assert!(!message.to_lowercase().contains("x-goog-api-key"), "{message}");
        }
    }
}
