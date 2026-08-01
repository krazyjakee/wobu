//! The kind registry.
//!
//! Adding a node kind is meant to be a config change, not a feature — so
//! everything that varies per kind (label, icon, colour, influence layer, the
//! sections Enhance fills in, the link roles the UI offers, whether it nests)
//! is declared here and nowhere else. See `docs/02-data-model.md`.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::link::LinkRole;

/// The seven layers of the influence stack, outermost first.
/// Order is fixed and meaningful — see `docs/04-influence-engine.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Style,
    World,
    Ancestry,
    Culture,
    Place,
    Subject,
    Shot,
}

impl Layer {
    /// Sort position in the stack. Lower is more distant from the subject.
    pub fn order(self) -> u8 {
        match self {
            Layer::Style => 1,
            Layer::World => 2,
            Layer::Ancestry => 3,
            Layer::Culture => 4,
            Layer::Place => 5,
            Layer::Subject => 6,
            Layer::Shot => 7,
        }
    }

    /// The colour used for the layer dot, prompt tinting and reference borders.
    /// Must stay in sync with the tokens in `docs/03-ui-layout.md`.
    pub fn color(self) -> &'static str {
        match self {
            Layer::Style => "#e2a44f",
            Layer::World => "#4fd1c5",
            Layer::Ancestry => "#7bd88f",
            Layer::Culture => "#f28bb4",
            Layer::Place => "#6aa9f5",
            Layer::Subject | Layer::Shot => "#9d7cf5",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Layer::Style => "Style",
            Layer::World => "World",
            Layer::Ancestry => "Ancestry",
            Layer::Culture => "Culture",
            Layer::Place => "Place",
            Layer::Subject => "Subject",
            Layer::Shot => "Shot",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    StyleGuide,
    WorldBible,
    Species,
    Culture,
    Setting,
    Character,
    Creature,
    Prop,
    Environment,
    Vehicle,
}

impl NodeKind {
    pub const ALL: [NodeKind; 10] = [
        NodeKind::StyleGuide,
        NodeKind::WorldBible,
        NodeKind::Species,
        NodeKind::Culture,
        NodeKind::Setting,
        NodeKind::Character,
        NodeKind::Creature,
        NodeKind::Prop,
        NodeKind::Environment,
        NodeKind::Vehicle,
    ];

    /// The wire and frontmatter form: snake_case.
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::StyleGuide => "style_guide",
            NodeKind::WorldBible => "world_bible",
            NodeKind::Species => "species",
            NodeKind::Culture => "culture",
            NodeKind::Setting => "setting",
            NodeKind::Character => "character",
            NodeKind::Creature => "creature",
            NodeKind::Prop => "prop",
            NodeKind::Environment => "environment",
            NodeKind::Vehicle => "vehicle",
        }
    }

    /// The directory under `nodes/` holding this kind. Kebab-case, because every
    /// path segment in a project folder is a slug (`docs/02-data-model.md`).
    pub fn dir(self) -> &'static str {
        match self {
            NodeKind::StyleGuide => "style-guide",
            NodeKind::WorldBible => "world-bible",
            NodeKind::Species => "species",
            NodeKind::Culture => "culture",
            NodeKind::Setting => "setting",
            NodeKind::Character => "character",
            NodeKind::Creature => "creature",
            NodeKind::Prop => "prop",
            NodeKind::Environment => "environment",
            NodeKind::Vehicle => "vehicle",
        }
    }

    pub fn from_dir(dir: &str) -> Option<NodeKind> {
        NodeKind::ALL.into_iter().find(|k| k.dir() == dir)
    }
}

impl std::str::FromStr for NodeKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<NodeKind> {
        NodeKind::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| Error::UnknownKind(s.to_string()))
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a description section holds a paragraph or a list of short items.
/// `palette`, `signature` and `never` are lists; everything else is prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionValueKind {
    Text,
    List,
}

// Serialize only: the registry is compile-time data made of `&'static` slices,
// which cannot be deserialized back into. It travels one way, to the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionDef {
    pub key: &'static str,
    pub label: &'static str,
    pub value_kind: SectionValueKind,
}

/// The control the editor renders for one kind-specific attribute.
///
/// Kept separate from [`SectionValueKind`]: description sections are machine-
/// written prose or lists, while attributes are small human-authored facts and
/// therefore also need numeric and yes/no controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeValueKind {
    Text,
    Number,
    Boolean,
}

// Serialize only, like `SectionDef`: this is compile-time registry data sent to
// the webview, never project data read back from disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributeDef {
    pub key: &'static str,
    pub label: &'static str,
    pub value_kind: AttributeValueKind,
}

const fn attribute(key: &'static str, label: &'static str) -> AttributeDef {
    AttributeDef { key, label, value_kind: AttributeValueKind::Text }
}

const fn text(key: &'static str, label: &'static str) -> SectionDef {
    SectionDef { key, label, value_kind: SectionValueKind::Text }
}

const fn list(key: &'static str, label: &'static str) -> SectionDef {
    SectionDef { key, label, value_kind: SectionValueKind::List }
}

// The full vocabulary of description sections. Kinds select from these; the key
// is what appears in the `## Description` block on disk and in the LLM tool
// schema, so renaming one is a format change.
const SILHOUETTE: SectionDef = text("silhouette", "Silhouette");
const ANATOMY: SectionDef = text("anatomy", "Anatomy");
const MATERIALS: SectionDef = text("materials", "Materials");
const COSTUME: SectionDef = text("costume", "Costume");
const ORNAMENT: SectionDef = text("ornament", "Ornament");
const ICONOGRAPHY: SectionDef = text("iconography", "Iconography");
const WEAPONS: SectionDef = text("weapons", "Weapon language");
const ARCHITECTURE: SectionDef = text("architecture", "Architecture");
const CLIMATE: SectionDef = text("climate", "Climate");
const LIGHT: SectionDef = text("light", "Ambient light");
const WEAR: SectionDef = text("wear", "Wear & age");
const MEDIUM: SectionDef = text("medium", "Medium");
const RENDERING: SectionDef = text("rendering", "Rendering");
const LINE_QUALITY: SectionDef = text("line_quality", "Line quality");
const LIGHTING: SectionDef = text("lighting", "Lighting model");
const ERA: SectionDef = text("era", "Era");
const TONE: SectionDef = text("tone", "Tone");
const TECH_LEVEL: SectionDef = text("tech_level", "Tech & magic level");
const PALETTE: SectionDef = list("palette", "Palette");
const SIGNATURE: SectionDef = list("signature", "Signature details");
const NEVER: SectionDef = list("never", "Never");

// Small authored facts that do not belong in the generated visual description.
// Kinds opt into these below. Their JSON keys are part of the on-disk format.
const ERA_ATTRIBUTE: AttributeDef = attribute("era", "Era");
const SCALE_ATTRIBUTE: AttributeDef = attribute("scale", "Scale");
const BIOME_ATTRIBUTE: AttributeDef = attribute("biome", "Biome");
const MATERIAL_ATTRIBUTE: AttributeDef = attribute("material", "Primary material");

/// Everything the app needs to know about a node kind.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindDef {
    pub kind: NodeKind,
    pub label: &'static str,
    pub plural: &'static str,
    /// Stable identifier the frontend maps to a glyph — not a glyph itself, so
    /// the icon set can change without touching the backend.
    pub icon: &'static str,
    pub color: &'static str,
    /// Which influence layer nodes of this kind contribute to.
    pub layer: Layer,
    /// Whether `parent_id` nesting within the kind is allowed
    /// (Region → City → District).
    pub nests: bool,
    /// Exactly one instance per project, pinned above the rule in the navigator.
    pub singleton: bool,
    pub dir: &'static str,
    /// Kind-specific facts the Notes tab renders as generated controls.
    pub attributes: &'static [AttributeDef],
    pub sections: &'static [SectionDef],
    /// The link roles the UI offers when adding an influence to this kind.
    pub default_link_roles: &'static [LinkRole],
}

const REGISTRY: &[KindDef] = &[
    KindDef {
        kind: NodeKind::StyleGuide,
        label: "Art Style",
        plural: "Art Style",
        icon: "star",
        color: "#e2a44f",
        layer: Layer::Style,
        nests: false,
        singleton: true,
        dir: "style-guide",
        attributes: &[],
        sections: &[MEDIUM, RENDERING, LINE_QUALITY, LIGHTING, PALETTE, NEVER],
        default_link_roles: &[],
    },
    KindDef {
        kind: NodeKind::WorldBible,
        label: "World Canon",
        plural: "World Canon",
        icon: "globe",
        color: "#4fd1c5",
        layer: Layer::World,
        nests: false,
        singleton: true,
        dir: "world-bible",
        attributes: &[ERA_ATTRIBUTE],
        sections: &[ERA, TONE, TECH_LEVEL, MATERIALS, PALETTE, NEVER],
        default_link_roles: &[LinkRole::StyledBy],
    },
    KindDef {
        kind: NodeKind::Species,
        label: "Species",
        plural: "Species",
        icon: "dna",
        color: "#7bd88f",
        layer: Layer::Ancestry,
        nests: true,
        singleton: false,
        dir: "species",
        attributes: &[SCALE_ATTRIBUTE],
        sections: &[SILHOUETTE, ANATOMY, MATERIALS, PALETTE, SIGNATURE, NEVER],
        default_link_roles: &[LinkRole::SpeciesOf, LinkRole::RelatedTo],
    },
    KindDef {
        kind: NodeKind::Culture,
        label: "Culture",
        plural: "Cultures",
        icon: "banner",
        color: "#f28bb4",
        layer: Layer::Culture,
        nests: true,
        singleton: false,
        dir: "culture",
        attributes: &[MATERIAL_ATTRIBUTE],
        sections: &[COSTUME, ORNAMENT, ICONOGRAPHY, WEAPONS, MATERIALS, PALETTE, NEVER],
        default_link_roles: &[LinkRole::SpeciesOf, LinkRole::LocatedIn, LinkRole::RelatedTo],
    },
    KindDef {
        kind: NodeKind::Setting,
        label: "Setting",
        plural: "Settings",
        icon: "map",
        color: "#6aa9f5",
        layer: Layer::Place,
        nests: true,
        singleton: false,
        dir: "setting",
        attributes: &[ERA_ATTRIBUTE, BIOME_ATTRIBUTE],
        sections: &[CLIMATE, ARCHITECTURE, LIGHT, WEAR, MATERIALS, PALETTE, NEVER],
        default_link_roles: &[LinkRole::MemberOf, LinkRole::RelatedTo],
    },
    KindDef {
        kind: NodeKind::Character,
        label: "Character",
        plural: "Characters",
        icon: "person",
        color: "#9d7cf5",
        layer: Layer::Subject,
        nests: false,
        singleton: false,
        dir: "character",
        attributes: &[SCALE_ATTRIBUTE],
        sections: &[SILHOUETTE, ANATOMY, COSTUME, MATERIALS, PALETTE, SIGNATURE, NEVER],
        default_link_roles: &[LinkRole::SpeciesOf, LinkRole::MemberOf, LinkRole::LocatedIn],
    },
    KindDef {
        kind: NodeKind::Creature,
        label: "Creature",
        plural: "Creatures",
        icon: "paw",
        color: "#9d7cf5",
        layer: Layer::Subject,
        nests: false,
        singleton: false,
        dir: "creature",
        attributes: &[SCALE_ATTRIBUTE, BIOME_ATTRIBUTE],
        sections: &[SILHOUETTE, ANATOMY, MATERIALS, PALETTE, SIGNATURE, NEVER],
        default_link_roles: &[LinkRole::SpeciesOf, LinkRole::LocatedIn],
    },
    KindDef {
        kind: NodeKind::Prop,
        label: "Prop",
        plural: "Props",
        icon: "cube",
        color: "#9d7cf5",
        layer: Layer::Subject,
        nests: true,
        singleton: false,
        dir: "prop",
        attributes: &[SCALE_ATTRIBUTE, MATERIAL_ATTRIBUTE],
        sections: &[SILHOUETTE, MATERIALS, WEAR, PALETTE, SIGNATURE, NEVER],
        default_link_roles: &[LinkRole::MemberOf, LinkRole::LocatedIn],
    },
    KindDef {
        kind: NodeKind::Environment,
        label: "Environment",
        plural: "Environments",
        icon: "mountain",
        color: "#9d7cf5",
        layer: Layer::Subject,
        nests: false,
        singleton: false,
        dir: "environment",
        attributes: &[BIOME_ATTRIBUTE],
        sections: &[ARCHITECTURE, LIGHT, CLIMATE, MATERIALS, PALETTE, SIGNATURE, NEVER],
        default_link_roles: &[LinkRole::LocatedIn, LinkRole::MemberOf],
    },
    KindDef {
        kind: NodeKind::Vehicle,
        label: "Vehicle",
        plural: "Vehicles",
        icon: "ship",
        color: "#9d7cf5",
        layer: Layer::Subject,
        nests: false,
        singleton: false,
        dir: "vehicle",
        attributes: &[SCALE_ATTRIBUTE, MATERIAL_ATTRIBUTE],
        sections: &[SILHOUETTE, MATERIALS, WEAR, PALETTE, SIGNATURE, NEVER],
        default_link_roles: &[LinkRole::MemberOf, LinkRole::LocatedIn],
    },
];

/// The whole registry, in navigator display order.
pub fn kind_registry() -> &'static [KindDef] {
    REGISTRY
}

pub fn kind_def(kind: NodeKind) -> &'static KindDef {
    REGISTRY.iter().find(|d| d.kind == kind).expect("every NodeKind variant has a registry entry")
}

/// The definition behind a section key, whichever kinds declare it.
///
/// The vocabulary check for anything that names sections from outside the
/// registry — output presets weight them (`preset.rs`), and the compiler routes
/// them. A key nothing declares is not an error at any point in the pipeline; it
/// simply never matches a fragment, so the feature that named it is half-working
/// and silent about it. `None` is how a caller's test can catch that.
pub fn section_def(key: &str) -> Option<&'static SectionDef> {
    REGISTRY.iter().flat_map(|d| d.sections).find(|s| s.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_exactly_one_registry_entry() {
        assert_eq!(REGISTRY.len(), NodeKind::ALL.len());
        for kind in NodeKind::ALL {
            let matches = REGISTRY.iter().filter(|d| d.kind == kind).count();
            assert_eq!(matches, 1, "{kind} should have exactly one entry");
        }
    }

    #[test]
    fn registry_agrees_with_the_kind_enum() {
        for def in REGISTRY {
            assert_eq!(def.dir, def.kind.dir(), "{} dir mismatch", def.kind);
            assert_eq!(def.color, def.layer.color(), "{} colour must be its layer's", def.kind);
        }
    }

    #[test]
    fn directories_are_unique_and_slug_safe() {
        let mut seen = std::collections::HashSet::new();
        for def in REGISTRY {
            assert!(seen.insert(def.dir), "duplicate dir {}", def.dir);
            assert_eq!(def.dir, crate::slug::slugify(def.dir).unwrap());
        }
    }

    #[test]
    fn every_kind_can_produce_a_negative_prompt() {
        // `never` drives the negative prompt for every layer, so no kind may omit it.
        for def in REGISTRY {
            assert!(
                def.sections.iter().any(|s| s.key == "never"),
                "{} is missing a `never` section",
                def.kind
            );
        }
    }

    #[test]
    fn every_declared_section_is_findable_by_key() {
        // `section_def` is the vocabulary check other modules validate against,
        // so it has to see every key, not just the ones the first kind declares.
        for def in REGISTRY {
            for section in def.sections {
                assert_eq!(section_def(section.key), Some(section), "{}", section.key);
            }
        }
        assert_eq!(section_def("costume").unwrap().label, "Costume");
        assert!(section_def("").is_none());
        assert!(section_def("Silhouette").is_none(), "keys are the snake_case form");
    }

    #[test]
    fn attribute_keys_are_unique_within_a_kind_and_safe_for_json() {
        for def in REGISTRY {
            let mut seen = std::collections::HashSet::new();
            for attribute in def.attributes {
                assert!(
                    seen.insert(attribute.key),
                    "duplicate {} attribute {}",
                    def.kind,
                    attribute.key
                );
                assert!(!attribute.key.is_empty(), "{} has an empty attribute key", def.kind);
                assert!(
                    attribute
                        .key
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                    "{} attribute {} is not snake_case",
                    def.kind,
                    attribute.key
                );
            }
        }
    }

    #[test]
    fn kind_strings_round_trip() {
        for kind in NodeKind::ALL {
            assert_eq!(kind.as_str().parse::<NodeKind>().unwrap(), kind);
            assert_eq!(NodeKind::from_dir(kind.dir()), Some(kind));
        }
    }
}
