//! The seam between the protocol and the project.
//!
//! Everything an MCP tool can do to a Wobu project is one method on [`World`]
//! or on [`Narrative`], and nothing in this crate knows what a `Project`, a
//! `NodeKind`, an influence layer or a beat is. That is not decoupling for its
//! own sake. It is what makes the guarantee in `lib.rs` checkable: the list of
//! things an agent can reach is the list of methods on these two traits, each
//! fits on one screen, and a new capability cannot appear without a line being
//! added to one of them in a review.
//!
//! Two traits rather than one because the two bodies of source are two: the
//! world model is Markdown nodes with influence edges, the narrative is scene
//! documents with declared state, and the rules about what may be written to
//! each are different. Folding them together would produce one trait long
//! enough that nobody reads it, which is the failure mode the guarantee above
//! exists to avoid.
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

    // ── the other half of the project ─────────────────────────────────────

    /// The authored story in the same project. See [`Narrative`] for why it is
    /// a separate trait rather than another nine methods here.
    fn narrative(&self) -> &dyn Narrative;
}

/// Which scenes a discovery call wants, in the library's own vocabulary.
///
/// Every filter is a string and empty means "not filtered", which is the shape
/// [`Narrative::scenes`]'s implementor already speaks — the scene library the
/// writer uses is driven by exactly these fields, and a second vocabulary here
/// would be a second set of semantics for "act" that could disagree with the
/// one on screen.
///
/// `limit` defaults to twenty-five rather than to zero, because a filter
/// deserialised from `{}` is an agent asking for the first page and not an
/// agent asking for nothing. A key that is not a field is refused rather than
/// dropped: the schema is closed, and a misspelt filter that silently matched
/// the whole project would answer a question nobody asked.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct SceneFilter {
    /// Free text over scene names, beat intent and dialogue.
    pub query: String,
    /// World `EntityId`s. A scene may be in several quests, so `quest` selects
    /// rather than locates.
    pub act: String,
    pub arc: String,
    pub quest: String,
    pub tag: String,
    pub participant: String,
    /// `draft` or `approved`.
    pub review: String,
    /// `current` or `out_of_date`.
    pub freshness: String,
    /// Only scenes with a dialogue slot that has no words in it yet.
    pub missing_text: bool,
    pub offset: usize,
    pub limit: usize,
}

impl Default for SceneFilter {
    fn default() -> SceneFilter {
        SceneFilter {
            query: String::new(),
            act: String::new(),
            arc: String::new(),
            quest: String::new(),
            tag: String::new(),
            participant: String::new(),
            review: String::new(),
            freshness: String::new(),
            missing_text: false,
            offset: 0,
            limit: 25,
        }
    }
}

/// The authored story in one open project, as much of it as MCP is allowed to
/// see.
///
/// A second trait rather than nine more methods on [`World`], and for the
/// reason stated there: the guarantee is that the list of things an agent can
/// reach fits on a screen and cannot grow without a line being added in review.
/// One trait of twenty-one methods does not keep that promise; two traits that
/// each fit on a screen do, and the split is also the honest one — the world
/// model and the narrative are two bodies of source with two file formats and
/// two sets of rules about what may be written.
///
/// The write half is [`create_scene`](Narrative::create_scene) and
/// [`draft_dialogue`](Narrative::draft_dialogue), both additive: one makes a
/// new file, the other fills a slot that is empty. Neither replaces authored
/// wording, and there is no method here that writes a whole scene document —
/// a scene save is guarded by the stamp the reader held, and an agent posting
/// one request at a time holds no such thing.
pub trait Narrative: Send + Sync + 'static {
    /// Scene and asset counts, the declared classifications and how much text
    /// is still missing. The narrative equivalent of [`World::overview`].
    fn overview(&self) -> WorldResult;

    /// Scene rows, filtered and paged. With `query` set the rows also carry the
    /// lines that matched, addressed by beat, slot and variant.
    fn scenes(&self, filter: &SceneFilter) -> WorldResult;

    /// One whole scene document: beats in author order, dialogue slots,
    /// variants, choices, outcomes and tombstones.
    fn scene(&self, id: &str) -> WorldResult;

    /// The declared state variables — name, type, owner, default and range.
    /// What a condition or an effect is allowed to mention.
    fn declared_state(&self) -> WorldResult;

    /// World canon: facts, knowledge claims, relationships, events, quests and
    /// future restrictions, with the diagnostics they currently raise.
    fn canon(&self) -> WorldResult;

    /// Supporting text assets, as summaries.
    fn text_assets(&self) -> WorldResult;

    /// One supporting text asset in full, with its entries.
    fn text_asset(&self, id: &str) -> WorldResult;

    /// What is wrong: with one scene, or with every scene when `scene_id` is
    /// `None`.
    fn diagnostics(&self, scene_id: Option<&str>) -> WorldResult;

    // ── writes, behind the second opt-in ──────────────────────────────────

    fn create_scene(&self, name: &str) -> WorldResult;

    /// Put words into a dialogue slot that has none.
    ///
    /// Refused for a slot that already has a variant, and refused for a locked
    /// slot. The wording is recorded as `Imported`, never as `Human` and never
    /// as `Generated`: a reviewer must not be told a person wrote something
    /// nobody in the project has read, and there is no generation receipt
    /// behind it to point at.
    fn draft_dialogue(&self, scene_id: &str, slot_id: &str, body: &str) -> WorldResult;
}
