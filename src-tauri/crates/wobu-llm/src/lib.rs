//! Text providers: asking a model for a structured description and deciding
//! whether the answer is fit to become canon.
//!
//! The schema itself lives in `wobu-core`, generated from the kind registry, so
//! that the request we send and the response we accept cannot describe two
//! different shapes. See `docs/08-providers.md`.

pub mod error;
pub mod validate;

pub use error::{Error, Result};
pub use validate::{ValidatedDescription, parse_description, validate_description};
