//! The influence edge.

use serde::{Deserialize, Serialize};

use crate::Id;
use crate::kind::Layer;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkRole {
    SpeciesOf,
    MemberOf,
    LocatedIn,
    StyledBy,
    RelatedTo,
}

impl LinkRole {
    pub const ALL: [LinkRole; 5] = [
        LinkRole::SpeciesOf,
        LinkRole::MemberOf,
        LinkRole::LocatedIn,
        LinkRole::StyledBy,
        LinkRole::RelatedTo,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            LinkRole::SpeciesOf => "species_of",
            LinkRole::MemberOf => "member_of",
            LinkRole::LocatedIn => "located_in",
            LinkRole::StyledBy => "styled_by",
            LinkRole::RelatedTo => "related_to",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LinkRole::SpeciesOf => "Species",
            LinkRole::MemberOf => "Member of",
            LinkRole::LocatedIn => "Located in",
            LinkRole::StyledBy => "Styled by",
            LinkRole::RelatedTo => "Related to",
        }
    }

    /// Which stack layer this role feeds. `related_to` is lateral — it is shown
    /// in Relations but the influence engine does not give it a layer of its
    /// own, so it resolves at the subject's level.
    pub fn layer(self) -> Layer {
        match self {
            LinkRole::StyledBy => Layer::Style,
            LinkRole::SpeciesOf => Layer::Ancestry,
            LinkRole::MemberOf => Layer::Culture,
            LinkRole::LocatedIn => Layer::Place,
            LinkRole::RelatedTo => Layer::Subject,
        }
    }
}

impl std::fmt::Display for LinkRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An outgoing influence edge, stored in the owning node's frontmatter — which
/// is why there is no `from_id` here. [`LinkEdge`] adds it for the index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub to_id: Id,
    pub role: LinkRole,
    #[serde(default = "default_weight")]
    pub weight: f32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// The weight an edge carries when it does not say otherwise.
///
/// `pub(crate)` rather than private because [`crate::asset::AssetRef`] uses the
/// same three functions. An asset link and an influence edge are both "how much
/// of this source reaches the compiler", so a project where one defaults to 1.0
/// and the other to 0.5, or where one clamps to a different range, would give
/// two answers to the same question depending on which kind of edge it was.
pub(crate) fn default_weight() -> f32 {
    1.0
}

pub(crate) fn default_enabled() -> bool {
    true
}

/// Weights outside 0.0–1.0 are meaningless to the compiler and can only come
/// from a hand-edited file.
pub(crate) fn clamp_weight(weight: f32) -> f32 {
    weight.clamp(0.0, 1.0)
}

impl Link {
    pub fn new(to_id: Id, role: LinkRole) -> Self {
        Link { to_id, role, weight: default_weight(), enabled: default_enabled() }
    }

    pub fn clamped(mut self) -> Self {
        self.weight = clamp_weight(self.weight);
        self
    }
}

/// A link with both endpoints, as stored in the index for backlink queries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkEdge {
    pub from_id: Id,
    pub to_id: Id,
    pub role: LinkRole,
    pub weight: f32,
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_strings_round_trip_through_serde() {
        for role in LinkRole::ALL {
            let json = serde_json::to_string(&role).unwrap();
            assert_eq!(json, format!("\"{}\"", role.as_str()));
            let back: LinkRole = serde_json::from_str(&json).unwrap();
            assert_eq!(back, role);
        }
    }

    #[test]
    fn links_default_to_full_weight_and_enabled() {
        let link: Link =
            serde_json::from_str(r#"{"toId":"01ARZ3NDEKTSV4RRFFQ69G5FAV","role":"member_of"}"#)
                .unwrap();
        assert_eq!(link.weight, 1.0);
        assert!(link.enabled);
    }

    #[test]
    fn hand_edited_weights_are_clamped() {
        let id = Id::nil();
        assert_eq!(Link { weight: 4.0, ..Link::new(id, LinkRole::MemberOf) }.clamped().weight, 1.0);
        assert_eq!(Link { weight: -1.0, ..Link::new(id, LinkRole::MemberOf) }.clamped().weight, 0.0);
    }
}
