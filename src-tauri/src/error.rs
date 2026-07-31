//! The error shape that crosses the bridge.
//!
//! `wobu_store::Error` is a Rust error: it carries `PathBuf`s, an
//! `io::Error` source, a `rusqlite::Error`. None of that is useful to a
//! webview, and some of it (absolute paths) is more than the UI should be
//! pasting into a toast. So commands return this instead — a stable machine
//! `code` the frontend can branch on, plus a sentence a human can read.
//!
//! The full taxonomy (per-code copy, retry affordances, secret redaction) is
//! issue #4. This is the minimum that lets the frontend distinguish the cases
//! it already has UI for, without locking that work out later: adding a variant
//! to `Code` is additive, and `errorMessage()` in `src/lib/api.ts` already falls
//! back to `.message` for codes it does not recognise.

use serde::Serialize;
use wobu_store::Error as StoreError;

/// Machine-readable discriminant. Serialises snake_case, matching every other
/// enum in the domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    /// The folder the user picked has no `project.json`.
    NotAProject,
    /// Creating a project over one that already exists.
    AlreadyExists,
    /// Written by a newer Wobu than this build understands.
    SchemaTooNew,
    /// The id is not in the index — usually a stale tab pointing at a node
    /// someone else deleted.
    NoSuchNode,
    /// A node file on disk could not be parsed. Never a reason to overwrite it.
    Malformed,
    /// The project folder is not writable (a read-only share, typically).
    ReadOnly,
    /// A command that needs an open project was called without one.
    NoProjectOpen,
    /// A concurrent writer won the race. `conflict_path` is where our version
    /// was parked, relative to the project root.
    Conflict,
    /// The request itself was wrong — an empty name, a parent cycle, a
    /// singleton created twice. Domain rules, not I/O.
    Invalid,
    /// Filesystem trouble.
    Io,
    /// The local index, or anything else with no better home.
    Internal,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: Code,
    pub message: String,
    /// Only set when `code` is `Conflict`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_path: Option<String>,
}

impl CommandError {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        CommandError { code, message: message.into(), conflict_path: None }
    }

    pub fn no_project_open() -> Self {
        CommandError::new(Code::NoProjectOpen, "No project is open.")
    }

    /// A save that lost the race. The UI has no conflict view until M2
    /// (issue #18), but the outcome is reported honestly now rather than
    /// being flattened into a generic failure — the user's text is on disk
    /// at `conflict_path` either way, and they need to be told that.
    pub fn conflict(conflict_path: String) -> Self {
        CommandError {
            code: Code::Conflict,
            message: format!(
                "Someone else changed this node while you were editing it. \
                 Your version was saved alongside theirs as {conflict_path}."
            ),
            conflict_path: Some(conflict_path),
        }
    }
}

impl From<StoreError> for CommandError {
    fn from(e: StoreError) -> Self {
        // `to_string()` before matching: every variant already carries a
        // written-out `#[error(...)]` message, and duplicating that copy here
        // would leave two versions of it to drift apart.
        let message = e.to_string();
        let code = match &e {
            StoreError::NotAProject(_) => Code::NotAProject,
            StoreError::AlreadyExists(_) => Code::AlreadyExists,
            StoreError::SchemaTooNew { .. } => Code::SchemaTooNew,
            StoreError::NoSuchNode(_) => Code::NoSuchNode,
            StoreError::Malformed { .. } | StoreError::MissingFrontmatter(_) => Code::Malformed,
            StoreError::ReadOnly => Code::ReadOnly,
            StoreError::NoProjectOpen => Code::NoProjectOpen,
            StoreError::Io { .. } => Code::Io,
            StoreError::Core(_) => Code::Invalid,
            StoreError::Yaml(_) | StoreError::Json(_) | StoreError::Sqlite(_) => Code::Internal,
        };
        CommandError::new(code, message)
    }
}

impl From<wobu_core::Error> for CommandError {
    fn from(e: wobu_core::Error) -> Self {
        CommandError::new(Code::Invalid, e.to_string())
    }
}

pub type CommandResult<T> = std::result::Result<T, CommandError>;
