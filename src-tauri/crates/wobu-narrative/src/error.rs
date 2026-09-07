//! Errors that stop a narrative source file being read at all.
//!
//! Deliberately a short list. Anything a writer can be *shown* while they carry
//! on working — a dangling destination, a line with no text yet, a condition
//! over a variable nobody declared — is a [`Diagnostic`](crate::Diagnostic),
//! not an error, because refusing to open a scene until every branch is
//! finished would make the tool unusable for the half-finished state every
//! scene spends most of its life in. What is left here is the set of problems
//! where continuing would mean guessing at what the author meant.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::expr::TypeError;
use crate::state::Name;

pub type Result<T> = std::result::Result<T, Error>;

/// Where in a source file a problem is.
///
/// Carried structurally rather than only formatted into the message so the
/// editor in #157 can put the caret on the offending line instead of asking the
/// author to find it. Lines and columns are 1-based, matching what every text
/// editor shows in its status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    /// The message already carries the location when there is one, because a
    /// caller that only prints `{err}` should still get something a person can
    /// act on. `location` is the same information in a form the editor can use.
    #[error("{message}")]
    Source { location: Option<SourceLocation>, message: String },

    #[error("narrative source is missing its `schema_version`")]
    MissingSchemaVersion,

    #[error(
        "narrative source is schema version {found}; this build reads version {supported}. \
         Open the project with a newer Wobu rather than saving over it."
    )]
    UnsupportedSchemaVersion { found: u32, supported: u32 },

    #[error("`{value}` is not a valid {noun} id")]
    InvalidId { noun: &'static str, value: String },

    #[error(
        "`{0}` is not a usable name: expected a lowercase ASCII letter followed by \
         letters, digits or underscores"
    )]
    InvalidName(String),

    /// Rejected at the point of declaration rather than at the point of use.
    /// YAML reads these bare words as booleans or nulls, so a variable or enum
    /// member called `on` would come back from a round trip as `true` and
    /// quietly stop matching the comparison written against it.
    #[error("`{0}` cannot be used as a name: YAML reads it as a boolean or a null")]
    ReservedName(String),

    #[error("state variable `{0}` is declared more than once")]
    DuplicateVariable(Name),

    #[error("enum variable `{0}` declares no members, so nothing could ever satisfy it")]
    EmptyEnum(Name),

    #[error("`{name}` declares the empty range {min}..={max}")]
    EmptyRange { name: Name, min: i64, max: i64 },

    #[error("the default for `{name}` is not a valid value for it: {source}")]
    InvalidDefault { name: Name, source: TypeError },

    #[error(transparent)]
    Type(#[from] TypeError),
}

impl Error {
    /// Turn a YAML failure into a located one.
    ///
    /// `serde_norway` reports a position for parse and shape failures but not
    /// for every failure, so the location stays optional; a message without one
    /// is still better than a message that claims line 0.
    pub(crate) fn from_yaml(err: &serde_norway::Error) -> Error {
        let location =
            err.location().map(|loc| SourceLocation { line: loc.line(), column: loc.column() });
        let message = match location {
            Some(loc) => format!("{loc}: {err}"),
            None => err.to_string(),
        };
        Error::Source { location, message }
    }
}
