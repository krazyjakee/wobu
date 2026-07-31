//! Generation records.
//!
//! These are write-once and ULID-named, which is why `generations/**` has no
//! conflict surface at all on a shared folder. The `influence_snapshot` is what
//! makes a result reproducible six months later, after the world has moved on.
//!
//! The engine that *fills* these lands in M5; the record shape is here because
//! the on-disk format has to be stable before anything writes to it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Id;
use crate::kind::Layer;

/// Where a fragment was routed. See `docs/04-influence-engine.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FragmentTarget {
    Prompt,
    Negative,
    StyleRef,
    StructureRef,
    Palette,
    MoodboardOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotFragment {
    pub section: String,
    pub text: Option<String>,
    pub asset_id: Option<Id>,
    pub weight: f32,
    pub target: FragmentTarget,
    /// True when the budget dropped this fragment. Recorded rather than
    /// discarded so the Inspector can report what was lost instead of
    /// truncating silently.
    #[serde(default)]
    pub dropped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotLayer {
    pub layer: Layer,
    pub node_id: Option<Id>,
    pub node_name: String,
    pub weight: f32,
    pub muted: bool,
    pub fragments: Vec<SnapshotFragment>,
}

/// The exact resolved stack, weights and all, as it stood at generation time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfluenceSnapshot {
    pub layers: Vec<SnapshotLayer>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Generation {
    pub id: Id,
    pub node_id: Id,
    pub created_at: DateTime<Utc>,
    /// The output preset this was generated under (`character_sheet`,
    /// `turnaround`, …).
    pub preset: String,
    /// For turnarounds: which of the eight views this is.
    #[serde(default)]
    pub view_type: Option<String>,
    /// Whatever the user typed into the shot box, before compilation.
    #[serde(default)]
    pub user_prompt: String,
    pub compiled_prompt: String,
    pub negative_prompt: String,
    pub backend: String,
    pub model: String,
    pub seed: u64,
    #[serde(default)]
    pub params: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub output_asset_ids: Vec<Id>,
    pub influence_snapshot: InfluenceSnapshot,
}

impl Generation {
    /// Generation records are filed by month so a long-lived project does not
    /// end up with one directory holding tens of thousands of entries — which is
    /// what makes a listing over SMB crawl.
    pub fn rel_path(&self) -> String {
        format!("generations/{}/{}.json", self.created_at.format("%Y-%m"), self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generations_are_filed_by_month() {
        let g = Generation {
            id: Id::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
            node_id: crate::new_id(),
            created_at: "2026-07-31T14:22:11Z".parse::<DateTime<Utc>>().unwrap(),
            preset: "character_sheet".into(),
            view_type: None,
            user_prompt: String::new(),
            compiled_prompt: "…".into(),
            negative_prompt: String::new(),
            backend: "comfyui".into(),
            model: "flux-dev".into(),
            seed: 42,
            params: Default::default(),
            output_asset_ids: vec![],
            influence_snapshot: InfluenceSnapshot { layers: vec![] },
        };
        assert_eq!(g.rel_path(), "generations/2026-07/01ARZ3NDEKTSV4RRFFQ69G5FAV.json");
    }
}
