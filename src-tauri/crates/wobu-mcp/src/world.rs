//! The seam between the protocol and the project.
//!
//! Everything an MCP tool can do to a Wobu world is one method on [`World`],
//! and nothing in this crate knows what a `Project`, a `NodeKind` or an
//! influence layer is. That is not decoupling for its own sake. It is what
//! makes the guarantee in `lib.rs` checkable: the list of things an agent can
//! reach is the list of methods on this trait, it fits on one screen, and a new
//! capability cannot appear without a line being added to it in a review.
//!
//! The methods are synchronous and return [`serde_json::Value`], which is
//! unusual enough to justify. Synchronous because the implementation is a
//! `parking_lot::Mutex` around a SQLite index — the shell's own commands are
//! shaped the same way — and the dispatcher moves the whole call onto a
//! blocking thread rather than making every implementor thread its own
//! executor. `Value` because the payloads are the same serde shapes the webview
//! already receives, and re-describing forty fields of `Node` in this crate
//! would be a second definition of the world model that could drift from the
//! first.

use serde::Deserialize;
use serde_json::Value;

/// A tool failed on the world's terms rather than the protocol's.
///
/// This is not a Rust error type for the shell's convenience — it is a message
/// an agent is going to read and act on, so it is written for one. `retryable`
/// is the one machine-readable bit, because "the share is unplugged" and "there
/// is no such node" want opposite behaviour from something that loops.
#[derive(Debug, Clone)]
pub struct WorldError {
    pub message: String,
    pub retryable: bool,
}

impl WorldError {
    pub fn new(message: impl Into<String>) -> WorldError {
        WorldError { message: message.into(), retryable: false }
    }

    pub fn retryable(message: impl Into<String>) -> WorldError {
        WorldError { message: message.into(), retryable: true }
    }
}

impl std::fmt::Display for WorldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub type WorldResult = std::result::Result<Value, WorldError>;

/// The fields an MCP write may touch on an existing node.
///
/// Every field is `Option`, and absent means "leave it alone" — an agent that
/// wanted to clear a summary sends `""`, not a missing key, because the
/// alternative is a partial update silently blanking the fields it did not
/// mention.
///
/// The description is deliberately **not** here. It carries a
/// `DescriptionState` and an `enhanced_from` stamp that the enhance path
/// maintains, and a write that set the prose without them would leave a node
/// claiming to be freshly enhanced from notes it has never seen. An agent that
/// wants to contribute prose writes `notes_raw`, which is what that field is
/// for: the user's messy source notes, never machine-written prose.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodePatch {
    pub name: Option<String>,
    pub summary: Option<String>,
    pub notes_raw: Option<String>,
    pub tags: Option<Vec<String>>,
    pub attributes: Option<serde_json::Map<String, Value>>,
}

impl NodePatch {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.summary.is_none()
            && self.notes_raw.is_none()
            && self.tags.is_none()
            && self.attributes.is_none()
    }
}

/// One open Wobu project, as much of it as MCP is allowed to see.
///
/// Implemented by the Tauri shell against the project that is open in the
/// window, and by the test double in `dispatch`. The read half is safe to call
/// with the server merely enabled; the write half is only ever reached once the
/// *second* opt-in is on, and the dispatcher — not the implementor — is what
/// enforces that.
pub trait World: Send + Sync + 'static {
    /// Project name, path, counts by kind, and whether it is read-only. The
    /// first call an agent makes, and the one that tells it whether there is
    /// anything to talk about at all.
    fn overview(&self) -> WorldResult;

    /// Every node, as summaries. Optionally narrowed to one kind.
    fn list_nodes(&self, kind: Option<&str>) -> WorldResult;

    /// One whole node, including its links and attached asset roles.
    fn get_node(&self, id: &str) -> WorldResult;

    /// Full-text search over the local index. Returns summaries rather than
    /// bare ids, because an agent that got ids would immediately spend a call
    /// per hit finding out what they were.
    fn search_nodes(&self, query: &str, limit: usize) -> WorldResult;

    /// Outgoing and incoming influence edges for one node.
    fn node_links(&self, id: &str) -> WorldResult;

    /// The resolved influence stack for a subject: which nodes reached it,
    /// through what, at what weight.
    fn influence_stack(&self, subject_id: &str, preset: Option<&str>) -> WorldResult;

    /// The compiled positive and negative prompt for a subject — what a
    /// generation would actually send.
    fn compile_prompt(&self, subject_id: &str, preset: Option<&str>) -> WorldResult;

    /// Generation receipts for one node: model, provider, cost, seed, outcome.
    fn list_generations(&self, node_id: &str, limit: usize) -> WorldResult;

    /// One receipt in full.
    fn get_generation(&self, generation_id: &str) -> WorldResult;

    // ── writes, behind the second opt-in ──────────────────────────────────

    fn create_node(&self, kind: &str, name: &str, parent_id: Option<&str>) -> WorldResult;

    fn update_node(&self, id: &str, patch: &NodePatch) -> WorldResult;

    fn link_nodes(
        &self,
        node_id: &str,
        to_id: &str,
        role: &str,
        weight: Option<f32>,
    ) -> WorldResult;
}
