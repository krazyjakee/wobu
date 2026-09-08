//! Versioned review fingerprints share the dependency vocabulary. Frozen full
//! inputs remain available for historical receipt verification.
use std::collections::BTreeMap;

use serde_json::Value as Json;
use wobu_narrative::review::{REVIEW_CONTEXT_VERSION, ReviewContext, ReviewTarget};
use wobu_narrative::{Name, Scene, StateDocument, Value, WorldDocument};
use wobu_narrative_context::Character;
use wobu_narrative_deps::{Snapshot, ToolVersions};

pub(super) fn capture_context(
    scene: &Scene,
    target: &ReviewTarget,
    world: Json,
    schema: Json,
    characters: Json,
    state: BTreeMap<Name, Value>,
) -> ReviewContext {
    capture_context_version(scene, target, world, schema, characters, state, REVIEW_CONTEXT_VERSION)
}

pub(super) fn capture_context_version(
    scene: &Scene,
    target: &ReviewTarget,
    world: Json,
    schema: Json,
    characters: Json,
    state: BTreeMap<Name, Value>,
    version: u32,
) -> ReviewContext {
    if version == 1 || target.variant.is_none() {
        return ReviewContext::capture(scene, target, world, schema, characters, state);
    }
    // The event already freezes the source as event.after. Repeating the entire
    // scene for each target creates quadratic serialization in manual saves.
    let context = ReviewContext {
        version: 2,
        revision: String::new(),
        state,
        inputs: serde_json::json!({"world": world, "schema": schema, "characters": characters}),
    };
    let Ok(world) = serde_json::from_value::<WorldDocument>(context.inputs["world"].clone()) else {
        return context;
    };
    let Ok(schema) = serde_json::from_value::<StateDocument>(context.inputs["schema"].clone())
    else {
        return context;
    };
    let Ok(schema) = schema.schema() else { return context };
    let characters: BTreeMap<_, Character> = context.inputs["characters"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .filter_map(|v| serde_json::from_value::<Character>(v.clone()).ok())
        .map(|c| (c.id, c))
        .collect();
    let snapshot = Snapshot {
        scenes: std::slice::from_ref(scene),
        texts: &[],
        world: &world,
        schema: &schema,
        characters: &characters,
        producers: &BTreeMap::new(),
        versions: ToolVersions::current(),
    };
    let dependencies =
        wobu_narrative_deps::capture::capture_scene_variant(scene, &snapshot, target.variant)
            .into_iter()
            .find(|d| target.variant == Some(d.variant()));
    match dependencies {
        Some(dependencies) => context
            .with_dependencies(serde_json::to_value(dependencies).expect("dependencies serialize")),
        None => context,
    }
}

/// Verify historical wording against the versions recorded then. Upgrading the
/// toolchain changes current readiness, not the validity of an old decision.
pub(super) fn historical_context(
    context: &ReviewContext,
    scene: &Scene,
    target: &ReviewTarget,
    state: BTreeMap<Name, Value>,
) -> ReviewContext {
    let rebuilt = capture_context_version(
        scene,
        target,
        context.inputs["world"].clone(),
        context.inputs["schema"].clone(),
        context.inputs["characters"].clone(),
        state,
        context.version,
    );
    if context.version != 2 || !rebuilt.inputs["dependencies"].is_object() {
        return rebuilt;
    }
    let mut dependencies = rebuilt.inputs["dependencies"].clone();
    dependencies["versions"] = context.inputs["dependencies"]["versions"].clone();
    rebuilt.with_dependencies(dependencies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wobu_narrative::{Beat, DialogueSlot, Speaker, Text, Variant};

    #[test]
    fn an_old_toolchain_changes_readiness_without_invalidating_the_historical_receipt() {
        let mut scene = Scene::new("Harbour");
        let mut beat = Beat::new("Arrival");
        let mut slot = DialogueSlot::new(Speaker::Narrator);
        let variant = Variant::new(Text::written("Welcome."));
        let target = ReviewTarget {
            scene: scene.id,
            beat: beat.id,
            slot: slot.id,
            variant: Some(variant.id),
        };
        slot.variants.push(variant);
        beat.dialogue.push(slot);
        scene.beats.push(beat);
        let current = capture_context(
            &scene,
            &target,
            serde_json::to_value(WorldDocument::default()).unwrap(),
            serde_json::to_value(StateDocument::new(vec![])).unwrap(),
            serde_json::json!({}),
            BTreeMap::new(),
        );
        let mut dependencies = current.inputs["dependencies"].clone();
        dependencies["versions"]["prompt"] = serde_json::json!(0);
        let old = current.clone().with_dependencies(dependencies);
        assert!(old.valid());
        assert_ne!(current.revision, old.revision);
        assert_eq!(
            historical_context(&old, &scene, &target, old.state.clone()).revision,
            old.revision
        );
        let mut damaged = old.clone();
        damaged.inputs["world"] = serde_json::json!("invalid frozen world");
        assert!(!historical_context(&damaged, &scene, &target, damaged.state.clone()).valid());
    }
}
