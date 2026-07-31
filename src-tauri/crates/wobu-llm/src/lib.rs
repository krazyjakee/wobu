//! Text providers: asking a model for a structured description and deciding
//! whether the answer is fit to become canon.
//!
//! The schema itself lives in `wobu-core`, generated from the kind registry, so
//! that the request we send and the response we accept cannot describe two
//! different shapes. See `docs/08-providers.md`.
//!
//! [`TextProvider`] is the boundary every vendor sits behind. It is written to
//! the intersection of what Anthropic and Gemini document rather than to either
//! one, and the reasoning for each place they differ is in `provider.rs`. No
//! HTTP client is a dependency of this crate on purpose: the adapters bring
//! their own, and a client declared here would have made its assumptions part of
//! the boundary before either adapter existed.

pub mod cancel;
pub mod error;
pub mod provider;
pub mod validate;

pub use cancel::{Cancel, Cancelled};
pub use error::{Error, Result};
pub use provider::{
    DEFAULT_MAX_OUTPUT_TOKENS, DeltaSink, Discard, EnhanceOutcome, EnhanceRequest, TextProvider,
    Usage,
};
pub use validate::{ValidatedDescription, parse_description, validate_description};
