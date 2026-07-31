//! Assets — files on disk, addressed by content.
//!
//! Content addressing is what makes `assets/**` conflict-free on a shared
//! folder: two people importing the same reference write identical bytes to an
//! identical path, so the write can never lose anyone's work
//! (`docs/07-file-shares.md`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Id;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    /// Imported by the user as influence.
    Reference,
    /// Produced by a generation.
    Generated,
    /// Pasted or dropped, not yet given a role.
    Upload,
}

/// What an image *is for*, which is what makes image context routable: a
/// `palette` reference goes to colour conditioning, a `pose` reference to a
/// structure adapter, and a `mood` reference is only ever shown to the human.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetRole {
    Silhouette,
    Palette,
    Material,
    Mood,
    Pose,
    Costume,
    FullRef,
}

impl AssetRole {
    pub const ALL: [AssetRole; 7] = [
        AssetRole::Silhouette,
        AssetRole::Palette,
        AssetRole::Material,
        AssetRole::Mood,
        AssetRole::Pose,
        AssetRole::Costume,
        AssetRole::FullRef,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AssetRole::Silhouette => "silhouette",
            AssetRole::Palette => "palette",
            AssetRole::Material => "material",
            AssetRole::Mood => "mood",
            AssetRole::Pose => "pose",
            AssetRole::Costume => "costume",
            AssetRole::FullRef => "full_ref",
        }
    }

    /// Whether this role can ever reach a backend, or is human-only.
    pub fn is_conditioning(self) -> bool {
        !matches!(self, AssetRole::Mood)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: Id,
    /// Lowercase hex BLAKE3 of the file contents. This, not the id, determines
    /// where the file lives.
    pub hash: String,
    pub kind: AssetKind,
    /// Project-relative, always `/`-separated. Absolute paths are never stored —
    /// the same share is mounted at a different path on every machine.
    pub rel_path: String,
    pub thumb_path: Option<String>,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub created_at: DateTime<Utc>,
}

/// Attaches an asset to a node with a role and a weight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetLink {
    pub asset_id: Id,
    pub node_id: Id,
    pub role: AssetRole,
    pub weight: f32,
    pub enabled: bool,
}

/// Where a blob lives inside the project folder, sharded by the first two hex
/// characters of its hash to keep any one directory small enough for an SMB
/// share to list quickly.
pub fn original_path(hash: &str, ext: &str) -> String {
    format!("assets/originals/{}/{}.{}", &hash[..2], hash, ext)
}

pub fn thumb_path(hash: &str) -> String {
    format!("assets/thumbs/{}/{}.webp", &hash[..2], hash)
}

pub fn mesh_path(hash: &str) -> String {
    format!("assets/meshes/{}/{}.glb", &hash[..2], hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_paths_are_sharded_and_relative() {
        let hash = "a3f9c1d2e4b5a6978081726354453627a3f9c1d2e4b5a6978081726354453627";
        assert_eq!(
            original_path(hash, "png"),
            format!("assets/originals/a3/{hash}.png")
        );
        assert_eq!(thumb_path(hash), format!("assets/thumbs/a3/{hash}.webp"));
        assert_eq!(mesh_path(hash), format!("assets/meshes/a3/{hash}.glb"));
        for p in [original_path(hash, "png"), thumb_path(hash), mesh_path(hash)] {
            assert!(!p.starts_with('/'), "must be project-relative");
            assert!(!p.contains('\\'), "must use forward slashes on every platform");
        }
    }

    #[test]
    fn mood_references_never_reach_a_backend() {
        assert!(!AssetRole::Mood.is_conditioning());
        for role in AssetRole::ALL.into_iter().filter(|r| *r != AssetRole::Mood) {
            assert!(role.is_conditioning(), "{} should route somewhere", role.as_str());
        }
    }
}
