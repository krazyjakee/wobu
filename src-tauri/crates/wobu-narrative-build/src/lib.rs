//! Pure decisions for explicit, resumable authoring builds. No provider or filesystem access.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use wobu_core::Id;
use wobu_narrative::{GenerationPolicy, Name, Value, VarType, VariantId};
use wobu_narrative_context::Selection;
use wobu_narrative_generation::FrozenRequest;

pub const VERSION: u32 = 1;
pub const MAX_ITEMS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Missing,
    Affected,
    AllSelected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub scope: Scope,
    /// Empty selects the whole project. Otherwise only these authored containers.
    #[serde(default)]
    pub containers: BTreeSet<wobu_narrative::SceneId>,
    #[serde(default)]
    pub state: BTreeMap<Name, Value>,
    #[serde(default)]
    pub commands: BTreeMap<Name, Vec<VarType>>,
    pub token_budget: u32,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Generate,
    Propose,
    Locked,
    Blocked,
}
pub fn action(
    asset: Option<GenerationPolicy>,
    slot: GenerationPolicy,
    text: Option<GenerationPolicy>,
) -> Action {
    let policies = [asset, Some(slot), text];
    if policies.contains(&Some(GenerationPolicy::Locked)) {
        Action::Locked
    } else if policies.contains(&Some(GenerationPolicy::Edited)) {
        Action::Propose
    } else {
        Action::Generate
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: Id,
    pub target: Selection,
    pub asset: bool,
    pub label: String,
    pub action: Action,
    pub reasons: Vec<wobu_narrative_deps::Explanation>,
    pub diagnostics: Vec<String>,
    pub request_id: Option<Id>,
    pub reusable: bool,
    /// Configuration identity and state belong to each item; a bounded variant
    /// planning pass can later supply them without creating another executor.
    pub candidate_variant_id: Option<VariantId>,
    pub state: BTreeMap<Name, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub version: u32,
    pub id: Id,
    pub scope: Scope,
    pub provider: String,
    pub model: String,
    pub items: Vec<Item>,
    pub diagnostics: Vec<String>,
}
#[derive(Debug, thiserror::Error)]
#[error("Invalid narrative build: {0}")]
pub struct Invalid(pub &'static str);
impl Build {
    pub fn validate(&self) -> Result<(), Invalid> {
        if self.version != VERSION
            || self.items.len() > MAX_ITEMS
            || self.provider.is_empty()
            || self.model.is_empty()
        {
            return Err(Invalid("version, settings or item limit"));
        }
        let mut ids = BTreeSet::new();
        for item in &self.items {
            if !ids.insert(item.id)
                || (matches!(item.action, Action::Generate | Action::Propose)
                    != item.request_id.is_some())
            {
                return Err(Invalid("item identity or eligibility"));
            }
        }
        Ok(())
    }
}

/// Actual authorized input, independent of batch/request IDs and unrelated
/// container bytes. Empty slots reuse the successful request's reserved identity.
/// Callers must validate the original request/output and recheck current guards.
pub fn reuse_key(request: &FrozenRequest) -> String {
    let input = serde_json::json!({
        "version": 1, "request_version":request.version,
        "source_schema":request.source_schema_version,
        "target":request.target, "speaker":request.speaker,
        "text_revision":request.expected_text_revision,
        "policy":request.expected_policy, "slot_policy":request.expected_slot_policy,
        "source_guard":request.expected_scene_hash,
        "analysis_policy":request.analysis.as_ref().map(|a|&a.policy_guard),
        "context":wobu_narrative_generation::context_key(&request.context),
        "provider":request.provider, "model":request.model, "settings":request.settings,
        "prompt_version":request.prompt_version,"output_schema_version":request.output_schema_version,
        "schema":request.output_schema,"system":request.system
    });
    blake3::hash(input.to_string().as_bytes()).to_hex().to_string()
}
