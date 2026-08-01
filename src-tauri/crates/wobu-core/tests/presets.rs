//! The preset registry as its consumers see it.
//!
//! `preset.rs`'s own tests hold the table honest; these hold the *surface*
//! honest. Fragment weighting (#42), the compile commands (#46) and the
//! Inspector's shot controls (#47) all reach the registry from outside the
//! crate, and a lookup that is correct but not public is no use to any of them.

use wobu_core::NodeKind;
use wobu_core::kind::kind_def;
use wobu_core::preset::{default_preset, preset, preset_registry, presets_for};

#[test]
fn the_inspector_can_build_its_dropdown_for_every_kind() {
    // What #47 does: list the presets for the selected node's kind and open on
    // the default. A default missing from its own dropdown would render as an
    // empty selection.
    for kind in NodeKind::ALL {
        let offered = presets_for(kind);
        let default = default_preset(kind);
        assert!(
            offered.iter().any(|p| p.id == default.id),
            "{kind} defaults to {} but is not offered it",
            default.id
        );
    }
}

#[test]
fn presets_for_preserves_registry_order() {
    // The dropdown order is the table order in `docs/04-influence-engine.md`, not
    // whatever a filter happens to produce.
    let ids = |kind| presets_for(kind).iter().map(|p| p.id).collect::<Vec<_>>();
    assert_eq!(
        ids(NodeKind::Character),
        ["character_sheet", "turnaround", "portrait_study", "costume_plate", "material_study"]
    );
    assert_eq!(ids(NodeKind::Environment), ["material_study", "environment_matte", "interior"]);
}

#[test]
fn the_any_column_is_offered_to_kinds_that_did_not_ask_for_it() {
    // `material_study` names no kinds of its own. If "any" were ever narrowed to
    // the subject kinds, the Style Guide and World Bible would lose their only
    // preset and their Generate button with it.
    for kind in [NodeKind::StyleGuide, NodeKind::WorldBible, NodeKind::Species, NodeKind::Culture] {
        assert!(
            presets_for(kind).iter().any(|p| p.id == "material_study"),
            "{kind} should be offered the material study"
        );
    }
}

#[test]
fn weighting_a_fragment_needs_no_knowledge_of_which_preset_it_is() {
    // The whole of #42's read of this registry: multiply by whatever the preset
    // says about the section, 1.0 if it says nothing. No branch on preset id.
    let sheet = preset("character_sheet").unwrap();
    let material = preset("material_study").unwrap();

    let weigh = |p: &wobu_core::preset::Preset, section: &str| 0.8 * p.section_priority(section);
    assert!(weigh(material, "materials") > weigh(sheet, "materials"));
    assert!(weigh(sheet, "silhouette") > weigh(material, "silhouette"));
    assert_eq!(weigh(sheet, "iconography"), 0.8, "an unmentioned section passes through");
    assert_eq!(weigh(sheet, "no_such_section"), 0.8, "so does one that does not exist");
}

#[test]
fn a_presets_priorities_land_on_sections_its_kinds_actually_have() {
    // Weaker than the crate-internal check that every key exists *somewhere*: a
    // preset whose boosts all name sections its own kinds never declare would
    // pass that one and still do nothing. At least one boost must bite.
    for p in preset_registry() {
        let boosts: Vec<&str> =
            p.priorities.iter().filter(|sp| sp.weight > 1.0).map(|sp| sp.section).collect();
        if boosts.is_empty() {
            continue;
        }
        let declares =
            |kind: &NodeKind| kind_def(*kind).sections.iter().any(|s| boosts.contains(&s.key));
        let lands = p.kinds.iter().any(declares);
        assert!(lands, "{} boosts {boosts:?}, none of which its kinds declare", p.id);
    }
}

#[test]
fn unknown_preset_ids_are_a_none_rather_than_a_panic() {
    // A generation recorded under a preset that has since been renamed still has
    // to open. `Generation.preset` is a free string precisely because of this.
    assert!(preset("character_sheet").is_some());
    assert!(preset("charactersheet").is_none());
    assert!(preset("").is_none());
}

#[test]
fn presets_cross_the_bridge_as_camel_case_json() {
    // The frontend consumes this the way it consumes the kind registry. Renaming
    // a field is a bridge change, so the wire form is pinned here.
    let value = serde_json::to_value(preset("prop_orthographic").unwrap()).unwrap();
    let object = value.as_object().unwrap();
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "aspect",
            "defaultFor",
            "framing",
            "id",
            "images",
            "kinds",
            "label",
            "priorities",
            "views",
        ]
    );
    assert_eq!(value["kinds"], serde_json::json!(["prop", "vehicle"]));
    assert_eq!(value["views"], serde_json::json!(["front", "side", "top"]));
    assert_eq!(value["priorities"][0], serde_json::json!({"section": "silhouette", "weight": 1.5}));
}
