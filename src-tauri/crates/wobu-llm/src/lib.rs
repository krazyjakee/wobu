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
//! trait itself still names no HTTP client and no runtime: `reqwest` arrived
//! with the Anthropic adapter and is reachable only from `anthropic.rs`, so a
//! second adapter is free to disagree with every assumption made there.

pub mod anthropic;
pub mod cancel;
pub mod error;
pub mod provider;
pub mod validate;

pub use anthropic::AnthropicProvider;
pub use cancel::{Cancel, Cancelled};
pub use error::{Error, Result};
pub use provider::{
    DEFAULT_MAX_OUTPUT_TOKENS, DeltaSink, Discard, EnhanceOutcome, EnhanceRequest, TextProvider,
    Usage,
};
pub use validate::{ValidatedDescription, parse_description, validate_description};
