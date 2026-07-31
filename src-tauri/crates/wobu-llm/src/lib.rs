//! Text providers: asking a model for a structured description and deciding
//! whether the answer is fit to become canon.
//!
//! The schema itself lives in `wobu-core`, generated from the kind registry, so
//! that the request we send and the response we accept cannot describe two
//! different shapes. See `docs/08-providers.md`.
//!
//! [`TextProvider`] is the boundary every vendor sits behind. It is written to
//! the intersection of what Anthropic and Gemini document rather than to either
//! one, and the reasoning for each place they differ is in `provider.rs`. The
//! trait itself still names no HTTP client and no runtime: `reqwest` is reached
//! only from the adapters, and nothing in `provider.rs` knows it exists.
//!
//! Both adapters landed against the same trait without widening it. What they
//! turned out to share — SSE framing, and reading a body under a cancellation —
//! is in `stream.rs` rather than copied into each, because a second copy is a
//! second place for the same subtle bug to be fixed in only one of them. What
//! they do not share is what the payloads mean, which is each adapter's
//! `wire.rs`.

pub mod anthropic;
pub mod cancel;
pub mod error;
pub mod gemini;
pub mod provider;
pub(crate) mod stream;
pub mod validate;

pub use anthropic::AnthropicProvider;
pub use gemini::GeminiProvider;
pub use cancel::{Cancel, Cancelled};
pub use error::{Error, Result};
pub use provider::{
    DEFAULT_MAX_OUTPUT_TOKENS, DeltaSink, Discard, EnhanceOutcome, EnhanceRequest, TextProvider,
    Usage,
};
pub use validate::{ValidatedDescription, parse_description, validate_description};
