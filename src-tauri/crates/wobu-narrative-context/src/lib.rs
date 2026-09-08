//! Attributed authoring context from immutable source data. No provider, clock, IO or weights.
//! Conditions use the same checked evaluator as the runtime. Time is declared state;
//! this resolver does not invent event chronology or propagate another character's beliefs.
mod resolve;
mod world;

pub use resolve::resolve;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wobu_narrative::{
    BeatId, DialogueSlotId, EntityId, Name, Scene, SceneId, StateSchema, Value, VariantId,
    WorldDocument,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub scene: SceneId,
    pub beat: BeatId,
    pub slot: DialogueSlotId,
    pub variant: Option<VariantId>,
}

/// Explicit narrative voice only. No appearance sections, notes, visual links or weights.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Character {
    pub id: EntityId,
    pub name: String,
    pub voice: Option<String>,
}

pub struct Input<'a> {
    pub scene: &'a Scene,
    pub world: &'a WorldDocument,
    pub schema: &'a StateSchema,
    pub characters: &'a BTreeMap<EntityId, Character>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub selection: Selection,
    /// Complete state, including any authored time/mission variables. Missing is never false.
    pub state: BTreeMap<Name, Value>,
    /// Estimated input tokens only, using the existing influence engine's three-char heuristic.
    pub token_budget: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fragment {
    pub kind: String,
    pub source: String,
    pub required: bool,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: String,
    pub source: String,
    pub message: String,
    pub blocking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub name: String,
    pub parameters: serde_json::Value,
    /// Pre-condition candidates, including inactive/omitted records; empty is meaningful.
    pub members: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenContext {
    pub version: u32,
    pub options: Options,
    pub fragments: Vec<Fragment>,
    pub omitted: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    /// Source-addressed content hashes, including null hashes for missing lookups.
    pub dependencies: BTreeMap<String, String>,
    pub queries: Vec<Query>,
    /// Canonical provider-neutral input. Future adapters must count real model tokens too.
    pub request: String,
    pub estimated_tokens: usize,
    pub ready: bool,
    pub hash: String,
}

pub fn content_hash(value: &impl Serialize) -> String {
    // All callers supply data structures whose serialization cannot fail.
    blake3::hash(&serde_json::to_vec(value).expect("context data is JSON serializable"))
        .to_hex()
        .to_string()
}
