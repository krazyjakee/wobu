//! Assets — files on disk, addressed by content.
//!
//! Content addressing is what makes `assets/**` conflict-free on a shared
//! folder: two people importing the same reference write identical bytes to an
//! identical path, so the write can never lose anyone's work
//! (`docs/07-file-shares.md`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Id;
use crate::generation::FragmentTarget;
use crate::link::{clamp_weight, default_enabled, default_weight};

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

    pub fn label(self) -> &'static str {
        match self {
            AssetRole::Silhouette => "Silhouette",
            AssetRole::Palette => "Palette",
            AssetRole::Material => "Material",
            AssetRole::Mood => "Mood",
            AssetRole::Pose => "Pose",
            AssetRole::Costume => "Costume",
            AssetRole::FullRef => "Full reference",
        }
    }

    /// Where an image in this role is routed when the stack is compiled.
    ///
    /// This is the whole reason roles exist, and it lives here rather than in
    /// the adapters because there is only one correct answer per role and it
    /// must not be re-decided per backend. An adapter that owned its own
    /// mapping would eventually route `mood` somewhere, and the first sign of
    /// it would be a private reference on a third party's servers.
    ///
    /// The reasoning, since none of these are arbitrary
    /// (`docs/04-influence-engine.md`):
    ///
    /// - `silhouette` and `pose` describe *shape*, so they belong to the
    ///   structure adapter (ControlNet) rather than to appearance transfer.
    /// - `material`, `costume` and `full_ref` describe *look*, which is style
    ///   transfer (IP-Adapter). `full_ref` is the pinned-reference case that
    ///   locks an entity's appearance across generations.
    /// - `palette` drives the colour-conditioning pass, which takes colours and
    ///   nothing else.
    /// - `mood` is the one that never leaves the machine. It is the board the
    ///   artist keeps for themselves, and the compiler drops it before anything
    ///   is sent.
    pub fn target(self) -> FragmentTarget {
        match self {
            AssetRole::Silhouette | AssetRole::Pose => FragmentTarget::StructureRef,
            AssetRole::Material | AssetRole::Costume | AssetRole::FullRef => {
                FragmentTarget::StyleRef
            }
            AssetRole::Palette => FragmentTarget::Palette,
            AssetRole::Mood => FragmentTarget::MoodboardOnly,
        }
    }

    /// Whether this role can ever reach a backend, or is human-only.
    ///
    /// Derived from [`target`](Self::target) rather than listing the human-only
    /// roles again: two lists of "which roles are private" would be one rename
    /// away from disagreeing, and the direction that disagreement fails in is
    /// somebody's mood board being uploaded.
    pub fn is_conditioning(self) -> bool {
        !matches!(self.target(), FragmentTarget::MoodboardOnly)
    }
}

impl std::fmt::Display for AssetRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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

/// A self-contained GLB in the content-addressed mesh store.
///
/// Separate from [`Asset`] because an image has dimensions, MIME and a derived
/// thumbnail while a mesh has none of those things. Forcing zeroes and nulls
/// into `Asset` would make every consumer guess which fields are real.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshAsset {
    pub id: Id,
    pub hash: String,
    pub rel_path: String,
    pub bytes: u64,
    pub created_at: DateTime<Utc>,
}

/// An asset attached to a node with a role and a weight, stored in the owning
/// node's frontmatter — which is why there is no `node_id` here, exactly as
/// [`crate::Link`] carries no `from_id`. [`AssetLink`] adds it for the index.
///
/// `(asset_id, role)` is the identity: the same picture can legitimately be
/// both a `full_ref` and a `palette` source for one entity, and those are two
/// links because they route to two different adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetRef {
    pub asset_id: Id,
    pub role: AssetRole,
    #[serde(default = "default_weight")]
    pub weight: f32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

impl AssetRef {
    pub fn new(asset_id: Id, role: AssetRole) -> Self {
        AssetRef { asset_id, role, weight: default_weight(), enabled: default_enabled() }
    }

    pub fn clamped(mut self) -> Self {
        self.weight = clamp_weight(self.weight);
        self
    }

    /// Whether this link contributes to what is sent to a backend.
    ///
    /// Both halves matter and neither is the other's business: `enabled` is the
    /// user muting a reference for one generation, `role` decides whether it
    /// was ever sendable at all.
    pub fn is_conditioning(&self) -> bool {
        self.enabled && self.role.is_conditioning()
    }
}

/// An asset link with both endpoints, as stored in the index so that "every
/// asset on this node in role X" and "every node using this asset" can be
/// answered without opening a node file.
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
        assert_eq!(original_path(hash, "png"), format!("assets/originals/a3/{hash}.png"));
        assert_eq!(thumb_path(hash), format!("assets/thumbs/a3/{hash}.webp"));
        assert_eq!(mesh_path(hash), format!("assets/meshes/a3/{hash}.glb"));
        for p in [original_path(hash, "png"), thumb_path(hash), mesh_path(hash)] {
            assert!(!p.starts_with('/'), "must be project-relative");
            assert!(!p.contains('\\'), "must use forward slashes on every platform");
        }
    }

    #[test]
    fn mood_references_never_reach_a_backend() {
        // The one that would be a privacy incident rather than a bug. A mood
        // board is what an artist collects for themselves, and every other role
        // is on a path that ends at somebody else's servers, so `mood` must be
        // the *only* role that is ever sendable — not merely one that happens
        // to be excluded today.
        assert_eq!(AssetRole::Mood.target(), FragmentTarget::MoodboardOnly);
        assert!(!AssetRole::Mood.is_conditioning());
        assert!(!AssetRef::new(Id::nil(), AssetRole::Mood).is_conditioning());

        for role in AssetRole::ALL {
            let private = role.target() == FragmentTarget::MoodboardOnly;
            assert_eq!(
                private,
                role == AssetRole::Mood,
                "{role} routes to moodboard_only and only mood may"
            );
            assert_eq!(!private, role.is_conditioning(), "{role} disagrees with its own target");
        }
    }

    #[test]
    fn every_role_routes_to_the_target_the_engine_expects() {
        // The mapping is the contract the adapters in M5/M6 are built against —
        // a `pose` reference arriving at style transfer instead of at the
        // structure adapter is a wrong picture, not an error, so nothing
        // downstream would report it.
        let expected = [
            (AssetRole::Silhouette, FragmentTarget::StructureRef),
            (AssetRole::Pose, FragmentTarget::StructureRef),
            (AssetRole::Material, FragmentTarget::StyleRef),
            (AssetRole::Costume, FragmentTarget::StyleRef),
            (AssetRole::FullRef, FragmentTarget::StyleRef),
            (AssetRole::Palette, FragmentTarget::Palette),
            (AssetRole::Mood, FragmentTarget::MoodboardOnly),
        ];
        assert_eq!(expected.len(), AssetRole::ALL.len(), "a new role needs a target here");
        for (role, target) in expected {
            assert_eq!(role.target(), target, "{role}");
        }

        // No image role may route to text. `prompt` and `negative` take words,
        // and a picture handed to either would be dropped in silence.
        for role in AssetRole::ALL {
            assert!(
                !matches!(role.target(), FragmentTarget::Prompt | FragmentTarget::Negative),
                "{role} routes an image into a text slot"
            );
        }
    }

    #[test]
    fn a_disabled_link_is_not_conditioning_whatever_its_role() {
        // Muting a reference for one generation and a role that may never be
        // sent are different questions, and the compiler has to respect both.
        let mut link = AssetRef::new(Id::nil(), AssetRole::Pose);
        assert!(link.is_conditioning());
        link.enabled = false;
        assert!(!link.is_conditioning());
    }

    #[test]
    fn role_strings_round_trip_through_serde() {
        // These strings are in frontmatter people hand-edit and in the union in
        // `src/lib/api.ts`; a rename breaks both at once and neither loudly.
        for role in AssetRole::ALL {
            let json = serde_json::to_string(&role).unwrap();
            assert_eq!(json, format!("\"{}\"", role.as_str()));
            assert_eq!(serde_json::from_str::<AssetRole>(&json).unwrap(), role);
        }
        assert_eq!(AssetRole::FullRef.as_str(), "full_ref");
    }

    #[test]
    fn asset_links_default_to_full_weight_and_enabled() {
        // The same defaults `Link` has, from the same functions — a file that
        // omits them must mean the same thing for both kinds of edge.
        let link: AssetRef =
            serde_json::from_str(r#"{"assetId":"01ARZ3NDEKTSV4RRFFQ69G5FAV","role":"palette"}"#)
                .unwrap();
        assert_eq!(link.weight, 1.0);
        assert!(link.enabled);
        assert_eq!(link, AssetRef::new(link.asset_id, AssetRole::Palette));
    }

    #[test]
    fn hand_edited_asset_weights_are_clamped() {
        let id = Id::nil();
        assert_eq!(
            AssetRef { weight: 4.0, ..AssetRef::new(id, AssetRole::Pose) }.clamped().weight,
            1.0
        );
        assert_eq!(
            AssetRef { weight: -1.0, ..AssetRef::new(id, AssetRole::Pose) }.clamped().weight,
            0.0
        );
    }
}
