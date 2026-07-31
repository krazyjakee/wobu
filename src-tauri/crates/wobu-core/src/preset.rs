//! The output preset registry.
//!
//! A preset is the recipe that turns one description into a particular *kind* of
//! sheet: which sections matter, how the shot is framed, what aspect, how many
//! images. Everything that varies per preset is declared in [`REGISTRY`] and
//! nowhere else, for the same reason kinds are (`kind.rs`) — a ninth preset is a
//! row in a table, not a feature. `a_preset_the_registry_has_never_heard_of_still_behaves`
//! is what holds that line: every accessor reads the struct, so nothing can grow
//! a match on `id`.
//!
//! See `docs/04-influence-engine.md`.

use serde::Serialize;

use crate::kind::NodeKind;

/// The `any` column of the preset table: every kind, present and future.
///
/// Spelled as the whole list rather than an empty-slice sentinel so
/// [`Preset::applies_to`] stays a plain membership test. A sentinel would be
/// exactly the special case this registry exists to avoid, and every consumer
/// that reads `kinds` directly — the Inspector's preset dropdown first — would
/// have to know about it too. Deriving from [`NodeKind::ALL`] also means a new
/// kind picks up the universal presets without anyone remembering to.
pub const ANY_KIND: &[NodeKind] = &NodeKind::ALL;

/// How much a preset cares about one description section.
///
/// A multiplier, not a rank: #42 computes `link.weight × section_priority ×
/// user_slider`, so 1.0 is "no opinion" and is what an unmentioned section gets.
// Serialize only, for the same reason as `KindDef`: the registry is compile-time
// data made of `&'static` slices, which cannot be deserialized back into.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionPriority {
    /// A key from the kind registry's section vocabulary. A key no kind declares
    /// is silently ignored forever, which is why
    /// `every_priority_names_a_section_the_kind_registry_declares` exists.
    pub section: &'static str,
    pub weight: f32,
}

const fn priority(section: &'static str, weight: f32) -> SectionPriority {
    SectionPriority { section, weight }
}

/// Everything the app needs to know about an output preset.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    /// The wire form, and what `Generation.preset` records. snake_case, like
    /// kind and section keys.
    ///
    /// A string rather than an enum on purpose. An enum would make every new
    /// preset a code change — a variant, an `as_str` arm, an `ALL` entry — which
    /// is precisely what this registry promises it is not. `Generation.preset`
    /// is already a `String` for the same reason: presets are configuration that
    /// outlives any one build, and an old generation naming a preset we have
    /// since renamed must still deserialize.
    pub id: &'static str,
    pub label: &'static str,
    /// Which kinds offer this preset. Declared here rather than as a field on
    /// `KindDef` because the table in `docs/04-influence-engine.md` is written
    /// preset-first, and because `KindDef` is a bridge contract the frontend
    /// already consumes.
    pub kinds: &'static [NodeKind],
    /// The kinds this is the *default* preset for. A subset of `kinds`, and
    /// every kind is claimed by exactly one preset — both enforced by tests,
    /// since two presets claiming the same kind would resolve by table order and
    /// so change silently when the table is reordered for display.
    pub default_for: &'static [NodeKind],
    /// Sections this preset weights away from 1.0. Only the exceptions are
    /// listed; the rest of the vocabulary is left alone.
    pub priorities: &'static [SectionPriority],
    /// The Shot layer's own text — pose, lighting, background, distance. Written
    /// as prose because it is compiled into the prompt alongside every other
    /// fragment rather than being a parameter the backend understands.
    pub framing: &'static str,
    /// Kept as a string, not an enum: `Capabilities::aspect_ratios` (#50) owns
    /// the aspect vocabulary, and it is the intersection of what each backend
    /// actually accepts, which no core enum could track. Values are checked
    /// against the documented set by `every_aspect_is_one_the_providers_accept`.
    pub aspect: &'static str,
    /// How many images one run of this preset emits.
    pub images: u8,
    /// Named views, in emission order, tagging each image's `view_type`. Empty
    /// for presets whose batch is just variations of one shot.
    ///
    /// Generating these is out of scope here — turnaround's view set is dictated
    /// by the 3D backend and gets its own issue in M7 (`docs/08-providers.md`).
    /// The names are declared now because they are what fixes `images`.
    pub views: &'static [&'static str],
}

impl Preset {
    pub fn applies_to(&self, kind: NodeKind) -> bool {
        self.kinds.contains(&kind)
    }

    pub fn is_default_for(&self, kind: NodeKind) -> bool {
        self.default_for.contains(&kind)
    }

    /// The multiplier this preset puts on `section`, or 1.0 for a section it has
    /// no opinion about.
    pub fn section_priority(&self, section: &str) -> f32 {
        self.priorities.iter().find(|p| p.section == section).map(|p| p.weight).unwrap_or(1.0)
    }

    /// Whether every image in the batch must share one seed.
    ///
    /// Derived rather than declared: named views are views of *one* object, so a
    /// batch that let the seed drift would return several different objects and
    /// be useless for the thing it was asked for. A batch without named views is
    /// meant to vary — that is what makes ×4 worth looking at.
    pub fn locks_seed(&self) -> bool {
        !self.views.is_empty()
    }
}

/// The eight views Hunyuan3D 3.1 reconstructs from, in the order and spelling
/// that backend names them (`docs/08-providers.md`), so the mesh adapter can pass
/// them straight through with no intermediate mapping.
const TURNAROUND_VIEWS: &[&str] =
    &["front", "left", "right", "back", "top", "bottom", "left_front", "right_front"];

const REGISTRY: &[Preset] = &[
    Preset {
        id: "character_sheet",
        label: "Character sheet",
        kinds: &[NodeKind::Character, NodeKind::Creature],
        default_for: &[NodeKind::Character, NodeKind::Creature],
        // Flat light is the point of the sheet, so a location's ambient light is
        // actively unhelpful here however strongly the stack argues for it.
        priorities: &[
            priority("silhouette", 1.4),
            priority("costume", 1.3),
            priority("anatomy", 1.2),
            priority("light", 0.3),
        ],
        framing: "full body, neutral A-pose, flat even light, plain background, single subject",
        aspect: "3:4",
        images: 4,
        views: &[],
    },
    Preset {
        id: "turnaround",
        label: "Turnaround",
        kinds: &[NodeKind::Character, NodeKind::Creature, NodeKind::Prop],
        default_for: &[],
        // The mirror of the material study: a turnaround is read as shape, and a
        // surface described in loving detail only fights the reconstruction.
        priorities: &[
            priority("silhouette", 1.6),
            priority("anatomy", 1.3),
            priority("materials", 0.6),
            priority("light", 0.2),
        ],
        // Tencent's input guidance — plain background, single object, subject
        // filling over half the frame — is baked in here so a turnaround cannot
        // be generated in a form the 3D stage would reject.
        framing: "single subject centred on a plain background, filling more than half the frame, \
                  flat even light, no props, no cast shadows",
        aspect: "1:1",
        images: 8,
        views: TURNAROUND_VIEWS,
    },
    Preset {
        id: "portrait_study",
        label: "Portrait study",
        kinds: &[NodeKind::Character],
        default_for: &[],
        // At head-and-shoulders distance the full-body read is gone and the face
        // is carrying everything.
        priorities: &[
            priority("anatomy", 1.5),
            priority("signature", 1.4),
            priority("ornament", 1.3),
            priority("silhouette", 0.6),
        ],
        framing: "head and shoulders, three-quarter view, dramatic key light with deep falloff, \
                  plain background",
        aspect: "4:5",
        images: 4,
        views: &[],
    },
    Preset {
        id: "costume_plate",
        label: "Costume plate",
        kinds: &[NodeKind::Character, NodeKind::Culture],
        default_for: &[NodeKind::Culture],
        // There is no wearer in the frame, so anatomy and body silhouette are
        // not merely unimportant, they would put a figure in the shot.
        priorities: &[
            priority("costume", 1.8),
            priority("ornament", 1.5),
            priority("iconography", 1.3),
            priority("silhouette", 0.3),
            priority("anatomy", 0.2),
        ],
        framing: "garments and gear laid flat on a neutral surface, arranged as a costume plate, \
                  no wearer, even overhead light",
        aspect: "3:4",
        images: 2,
        views: &[],
    },
    Preset {
        id: "prop_orthographic",
        label: "Prop orthographic",
        kinds: &[NodeKind::Prop, NodeKind::Vehicle],
        default_for: &[NodeKind::Prop, NodeKind::Vehicle],
        priorities: &[
            priority("silhouette", 1.5),
            priority("materials", 1.2),
            priority("wear", 1.1),
            priority("light", 0.2),
        ],
        framing: "orthographic elevation on neutral ground, flat even light, human scale figure \
                  beside the object for size",
        aspect: "4:3",
        images: 3,
        // Three separate generations rather than one image divided in three: a
        // single frame asked for three elevations reliably comes back with three
        // subtly different objects.
        views: &["front", "side", "top"],
    },
    Preset {
        id: "material_study",
        label: "Material study",
        kinds: ANY_KIND,
        // The fallback default for the kinds no subject-shaped preset covers —
        // the Style Guide, the World Bible and Species all describe surfaces
        // before they describe anything you could point a camera at.
        default_for: &[NodeKind::StyleGuide, NodeKind::WorldBible, NodeKind::Species],
        priorities: &[
            priority("materials", 2.0),
            priority("wear", 1.4),
            priority("palette", 1.3),
            priority("silhouette", 0.3),
            priority("costume", 0.3),
            priority("anatomy", 0.2),
        ],
        framing: "close-up tiling surface swatches, raking light, macro detail, no subject",
        aspect: "1:1",
        images: 6,
        views: &[],
    },
    Preset {
        id: "environment_matte",
        label: "Environment matte",
        kinds: &[NodeKind::Environment, NodeKind::Setting],
        default_for: &[NodeKind::Environment, NodeKind::Setting],
        priorities: &[
            priority("architecture", 1.4),
            priority("climate", 1.4),
            priority("light", 1.3),
            priority("anatomy", 0.3),
        ],
        framing: "wide establishing shot, deep atmospheric perspective, no foreground subject",
        aspect: "21:9",
        images: 3,
        views: &[],
    },
    Preset {
        id: "interior",
        label: "Interior",
        kinds: &[NodeKind::Environment],
        default_for: &[],
        // Indoors the weather is someone else's problem; what sells the room is
        // the light sources in it and what the surfaces have been through.
        priorities: &[
            priority("architecture", 1.5),
            priority("light", 1.5),
            priority("materials", 1.3),
            priority("wear", 1.2),
            priority("climate", 0.5),
        ],
        framing: "eye-level interior view, practical light sources in frame, natural lens, no \
                  fisheye",
        aspect: "16:9",
        images: 3,
        views: &[],
    },
];

/// The whole registry, in the display order of `docs/04-influence-engine.md`.
pub fn preset_registry() -> &'static [Preset] {
    REGISTRY
}

/// The preset an id names, or `None` when nothing does — a generation recorded
/// under a preset that has since been removed, most likely, which the caller has
/// to survive rather than panic on.
pub fn preset(id: &str) -> Option<&'static Preset> {
    REGISTRY.iter().find(|p| p.id == id)
}

/// Every preset offered for `kind`, in registry order. This is the Inspector's
/// preset dropdown.
pub fn presets_for(kind: NodeKind) -> Vec<&'static Preset> {
    REGISTRY.iter().filter(|p| p.applies_to(kind)).collect()
}

/// The preset a node of this kind starts on.
pub fn default_preset(kind: NodeKind) -> &'static Preset {
    REGISTRY
        .iter()
        .find(|p| p.is_default_for(kind))
        .expect("every NodeKind variant is claimed by exactly one preset")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::{kind_registry, section_def};

    #[test]
    fn a_preset_the_registry_has_never_heard_of_still_behaves() {
        // The acceptance criterion for #45, stated as a test: adding a preset is
        // a row in `REGISTRY` and nothing else. Anything that special-cased a
        // preset by id would have to fall through to a default for this one, and
        // it would come out shaped like something it is not.
        // A `const` because `&[priority(..)]` only reaches `'static` in a const
        // context — the same reason `REGISTRY` is one.
        const PRIORITIES: &[SectionPriority] = &[priority("silhouette", 3.0)];
        let ninth = Preset {
            id: "silhouette_study",
            label: "Silhouette study",
            kinds: &[NodeKind::Prop],
            default_for: &[],
            priorities: PRIORITIES,
            framing: "black shape on white ground",
            aspect: "1:1",
            images: 2,
            views: &["front", "back"],
        };

        assert!(preset(ninth.id).is_none(), "the point is that it is not registered");
        assert_eq!(ninth.section_priority("silhouette"), 3.0);
        assert_eq!(ninth.section_priority("materials"), 1.0);
        assert!(ninth.applies_to(NodeKind::Prop));
        assert!(!ninth.applies_to(NodeKind::Character));
        assert!(!ninth.is_default_for(NodeKind::Prop));
        assert!(ninth.locks_seed());
    }

    #[test]
    fn every_priority_names_a_section_the_kind_registry_declares() {
        // A priority on a key no kind declares is not an error anywhere — it just
        // never matches a fragment, so the preset is half-working and silent
        // about it. This is the only place a typo in that column can be caught.
        for p in REGISTRY {
            for sp in p.priorities {
                assert!(
                    section_def(sp.section).is_some(),
                    "{} weights `{}`, which no kind declares",
                    p.id,
                    sp.section
                );
            }
        }
    }

    #[test]
    fn priorities_are_positive_and_mention_each_section_once() {
        // Zero would be a mute, which is the user's control and not a preset's,
        // and a duplicated key would leave the second one dead.
        for p in REGISTRY {
            let mut seen = std::collections::HashSet::new();
            for sp in p.priorities {
                assert!(sp.weight > 0.0, "{} weights `{}` at {}", p.id, sp.section, sp.weight);
                assert!(seen.insert(sp.section), "{} weights `{}` twice", p.id, sp.section);
            }
        }
    }

    #[test]
    fn the_engine_documents_material_study_and_turnaround_as_opposites() {
        // `docs/04-influence-engine.md`: "a material study boosts `materials` and
        // drops `silhouette`; a turnaround does the reverse." That sentence is the
        // worked example every other preset's priorities were written against, so
        // it is the one pair worth pinning.
        let material = preset("material_study").unwrap();
        let turnaround = preset("turnaround").unwrap();
        assert!(material.section_priority("materials") > 1.0);
        assert!(material.section_priority("silhouette") < 1.0);
        assert!(turnaround.section_priority("silhouette") > 1.0);
        assert!(turnaround.section_priority("materials") < 1.0);
    }

    #[test]
    fn ids_are_unique_and_in_the_wire_form() {
        // `id` is written into every generation record, so it is a format detail:
        // snake_case like kind and section keys, and never reused.
        let mut seen = std::collections::HashSet::new();
        for p in REGISTRY {
            assert!(seen.insert(p.id), "duplicate preset id {}", p.id);
            assert!(!p.id.is_empty());
            assert!(
                p.id.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{} is not snake_case",
                p.id
            );
            assert_eq!(preset(p.id), Some(p), "{} should be findable by id", p.id);
        }
    }

    #[test]
    fn every_kind_can_generate_something() {
        // A kind with no preset has a dead Generate button and no way to tell
        // why. The `any` column is what guarantees this for the kinds that have
        // no subject-shaped preset of their own.
        for def in kind_registry() {
            let offered = presets_for(def.kind);
            assert!(!offered.is_empty(), "{} is offered no preset", def.kind);
            assert!(
                offered.iter().any(|p| p.id == "material_study"),
                "{} should still be offered the `any` preset",
                def.kind
            );
        }
    }

    #[test]
    fn every_kind_has_exactly_one_default_preset() {
        // Two presets claiming a kind would resolve by table order, so reordering
        // the registry for display would quietly change what users get.
        for def in kind_registry() {
            let claims: Vec<&str> =
                REGISTRY.iter().filter(|p| p.is_default_for(def.kind)).map(|p| p.id).collect();
            assert_eq!(claims.len(), 1, "{} is claimed by {claims:?}", def.kind);
            assert_eq!(default_preset(def.kind).id, claims[0]);
        }
    }

    #[test]
    fn a_preset_is_only_the_default_for_kinds_it_applies_to() {
        for p in REGISTRY {
            for kind in p.default_for {
                assert!(p.applies_to(*kind), "{} defaults for {kind} but excludes it", p.id);
            }
        }
    }

    #[test]
    fn named_views_are_one_image_each_and_share_a_seed() {
        // The two halves of the same fact. A turnaround of eight views that only
        // emitted four would silently drop half the 3D backend's input, and one
        // that let the seed drift would hand it eight different objects.
        for p in REGISTRY {
            if p.views.is_empty() {
                assert!(!p.locks_seed(), "{} varies, so it must not lock the seed", p.id);
                continue;
            }
            assert_eq!(
                p.views.len(),
                p.images as usize,
                "{} names {} views but emits {} images",
                p.id,
                p.views.len(),
                p.images
            );
            assert!(p.locks_seed(), "{} is views of one object", p.id);
            let mut seen = std::collections::HashSet::new();
            for view in p.views {
                assert!(seen.insert(view), "{} names view `{view}` twice", p.id);
            }
        }
    }

    #[test]
    fn every_preset_emits_at_least_one_image() {
        for p in REGISTRY {
            assert!(p.images > 0, "{} emits nothing", p.id);
        }
    }

    #[test]
    fn every_aspect_is_one_the_providers_accept() {
        // The intersection both Google image docs agree on (`docs/08-providers.md`).
        // A preset asking for a ratio outside it either fails late or, worse, is
        // silently ignored and returns a square.
        const SUPPORTED: &[&str] =
            &["1:1", "3:2", "2:3", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9"];
        for p in REGISTRY {
            assert!(SUPPORTED.contains(&p.aspect), "{} asks for {}", p.id, p.aspect);
        }
    }

    #[test]
    fn every_preset_says_something_about_framing() {
        // Framing text is the Shot layer's entire contribution. A preset without
        // it is a preset that only reweights, which is not what the dropdown says.
        for p in REGISTRY {
            assert!(!p.framing.trim().is_empty(), "{} has no framing text", p.id);
            assert!(!p.label.trim().is_empty(), "{} has no label", p.id);
        }
    }

    #[test]
    fn the_table_in_the_docs_is_the_registry() {
        // `docs/04-influence-engine.md` lists these eight and the kinds each is
        // for. Dropping one is a documentation change as much as a code one.
        let ids: Vec<&str> = REGISTRY.iter().map(|p| p.id).collect();
        assert_eq!(ids, [
            "character_sheet",
            "turnaround",
            "portrait_study",
            "costume_plate",
            "prop_orthographic",
            "material_study",
            "environment_matte",
            "interior",
        ]);
        assert_eq!(preset("turnaround").unwrap().views.len(), 8, "the 3D backend wants eight");
    }
}
