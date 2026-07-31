use std::time::Duration;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// What can go wrong between asking a provider for a description and having one
/// we are willing to write down.
///
/// Two groups, and the split is the thing to keep straight when adding a
/// variant:
///
/// - **The call**: a key, a quota, a socket, a schema the provider would not
///   take. Every vendor spells these differently — a 401 here, a
///   `PERMISSION_DENIED` there — and normalising them is most of what an adapter
///   is for, because the UI has exactly one set of answers and they are keyed on
///   [`Error::code`].
/// - **The answer**: the call succeeded and the model's output is not usable.
///   These are all retryable, because the alternative is putting half-formed or
///   mistyped data into somebody's world, and a node is canon — once written, it
///   feeds every prompt compiled from that entity.
#[derive(Debug, Error)]
pub enum Error {
    // ── the call ─────────────────────────────────────────────────────────
    /// Raised by key resolution rather than by an adapter: an adapter is
    /// constructed with a key, so by the time one exists this has been settled.
    /// It lives here so the enhance path has a single error type end to end.
    #[error("no API key for {provider} on this machine")]
    NoKey { provider: &'static str },

    #[error("{provider} rejected the API key")]
    BadKey { provider: &'static str },

    /// `retry_after` is the provider's own hint, in whatever form it gave it —
    /// a header, a field in the error body. Carried rather than turned into a
    /// sleep here because the job queue (#49) owns backoff and needs to weigh
    /// this against the rest of the queue.
    #[error("{provider} is rate limiting this key")]
    RateLimited { provider: &'static str, retry_after: Option<Duration> },

    #[error("{provider} needs billing enabled on the account before this model will run")]
    BillingRequired { provider: &'static str },

    /// The compiled prompt did not fit. Not retryable: the same stack compiles
    /// to the same prompt, so what has to change is the model or the influence
    /// feeding it.
    #[error("the prompt is longer than this model's context window")]
    ContextTooLong,

    /// The provider refused the schema itself, before generating anything.
    ///
    /// Always our bug, never the user's: Google documents that only a subset of
    /// JSON Schema is accepted, so a section added to the kind registry in a
    /// shape one vendor will not take shows up here and nowhere else. It is not
    /// retryable, and the message is aimed at whoever reads the log.
    #[error("the provider rejected the description schema: {detail}")]
    SchemaRejected { detail: String },

    /// Unreachable, timed out, or a 5xx. The one transport failure worth
    /// telling apart, because it is the one where waiting is the right answer.
    #[error("the provider could not be reached: {detail}")]
    Unavailable { detail: String },

    /// The user stopped it. Never retried — a queue that retries a cancellation
    /// is a queue that bills the user for pressing Stop.
    #[error("the request was cancelled")]
    Cancelled,

    // ── the answer ───────────────────────────────────────────────────────
    /// The stream ended before the document did — a `max_tokens` stop, or a
    /// connection that dropped mid-response.
    ///
    /// Separate from [`Error::NotJson`] on purpose. A truncated response usually
    /// *is* invalid JSON and would be caught anyway, but "usually" is not a
    /// guarantee we can put a node's contents behind, so an adapter that knows
    /// the response was cut short must say so before validating rather than
    /// hoping the parser notices.
    #[error("the response stopped before the description was finished")]
    Truncated,

    #[error("the response was not JSON: {0}")]
    NotJson(String),

    #[error("the response was {found}, not a JSON object")]
    NotAnObject { found: JsonTypeName },

    #[error("`{section}` is missing from the response")]
    MissingSection { section: SectionName },

    #[error("`{section}` should be {expected} but the response had {found}")]
    WrongSectionType { section: SectionName, expected: &'static str, found: JsonTypeName },

    #[error("`{section}` came back empty")]
    EmptySection { section: SectionName },

    #[error("`{section}` contains `{value}`, which is not a #rrggbb colour")]
    NotAHexColor { section: SectionName, value: String },
}

impl Error {
    /// Whether asking the same provider the same question again is worth doing.
    ///
    /// Matched exhaustively rather than returning a blanket answer so that every
    /// new variant has to state which side of the line it falls on. Read
    /// narrowly: this drives both the job queue's retries and the UI's "Try
    /// again", so a `true` that leads to the same failure spends the user's
    /// money to show them the same error.
    ///
    /// It must agree with the retryability of [`Error::code`]'s answer, or the
    /// queue and the button will disagree about the same failure.
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::NoKey { .. }
            | Error::BadKey { .. }
            | Error::BillingRequired { .. }
            | Error::ContextTooLong
            | Error::SchemaRejected { .. }
            | Error::Cancelled => false,

            Error::RateLimited { .. }
            | Error::Unavailable { .. }
            | Error::Truncated
            | Error::NotJson(_)
            | Error::NotAnObject { .. }
            | Error::MissingSection { .. }
            | Error::WrongSectionType { .. }
            | Error::EmptySection { .. }
            | Error::NotAHexColor { .. } => true,
        }
    }

    /// The stable dotted code this failure reaches the UI as.
    ///
    /// These strings are the serde renames on `Code` in `src-tauri/src/error.rs`,
    /// which is where the taxonomy is defined and where the frontend's
    /// `errorSurface` switches on them. They are copied rather than shared
    /// because that enum lives in the Tauri shell and this crate must not depend
    /// on the shell. `every_failure_lands_on_a_code_the_ui_already_knows` pins
    /// the strings; a code renamed there without being renamed here fails that
    /// test rather than silently routing an error nowhere.
    ///
    /// An *answer* failure is `provider.bad_response`, not
    /// `provider.unavailable`: the latter says the service is down, this one
    /// says the model answered badly, and folding them together tells a user to
    /// go and check their network over a request that arrived perfectly well.
    /// Both are retryable, which is the property the job queue acts on.
    ///
    /// [`Error::ContextTooLong`] is the one provider failure never worth
    /// retrying — the stack is too big by construction, so a "Try again" spends
    /// the user's money to fail identically. It has its own code so the UI can
    /// point at the lever that does work, which is the budget in #43.
    pub fn code(&self) -> &'static str {
        match self {
            Error::NoKey { .. } => "provider.no_key",
            Error::BadKey { .. } => "provider.bad_key",
            Error::RateLimited { .. } => "provider.rate_limited",
            Error::BillingRequired { .. } => "provider.billing_required",
            Error::Unavailable { .. } => "provider.unavailable",
            Error::ContextTooLong => "provider.context_too_long",
            // Our schema, our registry, our bug — the taxonomy's word for that
            // is `internal`.
            Error::SchemaRejected { .. } => "internal",
            Error::Cancelled => "cancelled",
            Error::Truncated
            | Error::NotJson(_)
            | Error::NotAnObject { .. }
            | Error::MissingSection { .. }
            | Error::WrongSectionType { .. }
            | Error::EmptySection { .. }
            | Error::NotAHexColor { .. } => "provider.bad_response",
        }
    }
}

/// Section keys are `&'static str` from the kind registry, never user input, so
/// they cost nothing to carry in an error. Mirrors `NodeKindName` in
/// `wobu-core`'s error module.
pub type SectionName = &'static str;

/// A JSON type named for a human: "a string", "an array".
pub type JsonTypeName = &'static str;

/// How a value is described in an error message. Deliberately reads as a noun
/// phrase so the `#[error]` strings above form sentences.
pub(crate) fn json_type_name(value: &serde_json::Value) -> JsonTypeName {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
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
            Error::NoKey { provider: "Anthropic" },
            Error::BadKey { provider: "Anthropic" },
            Error::RateLimited { provider: "Gemini", retry_after: Some(Duration::from_secs(30)) },
            Error::BillingRequired { provider: "Gemini" },
            Error::ContextTooLong,
            Error::SchemaRejected { detail: "unsupported keyword `pattern`".into() },
            Error::Unavailable { detail: "503".into() },
            Error::Cancelled,
            Error::Truncated,
            Error::NotJson("expected value at line 1".into()),
            Error::NotAnObject { found: "an array" },
            Error::MissingSection { section: "never" },
            Error::WrongSectionType {
                section: "silhouette",
                expected: "a string",
                found: "an array",
            },
            Error::EmptySection { section: "anatomy" },
            Error::NotAHexColor { section: "palette", value: "dark ochre".into() },
        ]
    }

    /// The codes `Code` in `src-tauri/src/error.rs` defines, and which of them
    /// `Code::retryable` returns true for. Copied deliberately: this crate
    /// cannot see that enum, so the copy is the only thing that can notice a
    /// drift, and it notices it here rather than in front of a user.
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
    fn a_cancelled_request_is_never_retried() {
        // Pressing Stop and being billed for a retry is the specific outcome
        // this exists to prevent.
        assert!(!Error::Cancelled.is_retryable());
        assert_eq!(Error::Cancelled.code(), "cancelled");
    }

    #[test]
    fn a_rejected_key_is_not_retried_but_a_rate_limit_is() {
        // Both are 4xx from most providers and the difference is not in the
        // status code, so an adapter that lumps them together produces either a
        // hammered key or a dead end.
        assert!(!Error::BadKey { provider: "Anthropic" }.is_retryable());
        assert!(
            Error::RateLimited { provider: "Anthropic", retry_after: None }.is_retryable()
        );
    }

    #[test]
    fn a_truncated_response_never_reaches_a_node_but_is_worth_another_call() {
        // A response cut short at `max_tokens` is a wasted call, not bad data —
        // provided it is an error at all. If it were `Ok`, the half-written
        // description would be the thing that got saved.
        assert!(Error::Truncated.is_retryable());
        assert_eq!(Error::Truncated.code(), "provider.bad_response");
    }

    #[test]
    fn error_messages_do_not_name_the_key() {
        // `redact::scrub` at the command boundary is the real guard, but an
        // error that carries a key at all has already put it in a log line.
        for error in one_of_each() {
            let message = error.to_string();
            assert!(!message.contains("sk-"), "{message}");
            assert!(!message.to_lowercase().contains("x-api-key"), "{message}");
        }
    }
}
