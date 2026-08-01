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

use crate::{AssetRole, Id};
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
    /// The role that routed a reference image. Older receipts predate this
    /// field; replay can recover their coarse route from `target`, while new
    /// receipts preserve the exact role (notably pose versus silhouette).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_role: Option<AssetRole>,
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

/// One cell in a reconstructable variant grid. Stored under
/// `Generation.params.variation`; a typed shape here prevents four callers from
/// inventing four almost-compatible JSON conventions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationVariation {
    pub grid_id: Id,
    pub index: u16,
    pub total: u16,
    #[serde(flatten)]
    pub value: VariationValue,
}

/// The typed seam between a mesh job receipt and the 3D gallery.
///
/// Stored under `Generation.params.meshOutput` for backwards compatibility:
/// older readers already preserve unknown params, while image output ids keep
/// their existing unambiguous meaning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshOutput {
    pub asset_id: Id,
    #[serde(default)]
    pub turnaround_generation_ids: Vec<Id>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "axis", rename_all = "snake_case")]
pub enum VariationValue {
    Seed { seed: u64 },
    FragmentWeight {
        #[serde(rename = "nodeId")]
        node_id: Id,
        weight: f32,
    },
    Preset { preset: String },
    Aspect { aspect: String },
}

impl Generation {
    /// Generation records are filed by month so a long-lived project does not
    /// end up with one directory holding tens of thousands of entries — which is
    /// what makes a listing over SMB crawl.
    pub fn rel_path(&self) -> String {
        format!("generations/{}/{}.json", self.created_at.format("%Y-%m"), self.id)
    }

    pub fn variation(&self) -> Option<GenerationVariation> {
        self.params
            .get("variation")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
    }

    pub fn mesh_output(&self) -> Option<MeshOutput> {
        self.params
            .get("meshOutput")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generations_are_filed_by_month() {
        let mut g = Generation {
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

        let variation = GenerationVariation {
            grid_id: crate::new_id(),
            index: 1,
            total: 3,
            value: VariationValue::FragmentWeight { node_id: g.node_id, weight: 0.7 },
        };
        g.params.insert("variation".into(), serde_json::to_value(&variation).unwrap());
        assert_eq!(g.variation(), Some(variation));

        let mesh = MeshOutput {
            asset_id: crate::new_id(),
            turnaround_generation_ids: vec![crate::new_id(), crate::new_id()],
        };
        g.params.insert("meshOutput".into(), serde_json::to_value(&mesh).unwrap());
        assert_eq!(g.mesh_output(), Some(mesh));
        assert!(g.output_asset_ids.is_empty(), "image outputs stay unambiguous");
    }
}
