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
    linked_scenes: &[Scene],
) -> ReviewContext {
    rebuild_context(
        scene,
        target,
        ReviewContext {
            version: REVIEW_CONTEXT_VERSION,
            revision: String::new(),
            state,
            inputs: serde_json::json!({"world": world, "schema": schema,
            "characters": characters, "linked_scenes": linked_scenes}),
        },
    )
}

fn rebuild_context(
    scene: &Scene,
    target: &ReviewTarget,
    mut context: ReviewContext,
) -> ReviewContext {
    if context.version == 1 || target.variant.is_none() {
        return ReviewContext::capture(
            scene,
            target,
            context.inputs["world"].clone(),
            context.inputs["schema"].clone(),
            context.inputs["characters"].clone(),
            context.state,
        );
    }
    context.revision.clear();
    if let Some(inputs) = context.inputs.as_object_mut() {
        inputs.remove("dependencies");
    }
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
    let linked_scenes: Vec<Scene> = match context.inputs.get("linked_scenes") {
        Some(value) => match serde_json::from_value(value.clone()) {
            Ok(scenes) => scenes,
            Err(_) => return context,
        },
        None => Vec::new(), // Historical v2 scene receipts predate linked-text context.
    };
    let asset = scene.editorial_text();
    if scene.supporting_text.is_some() && asset.is_none() {
        return context;
    }
    let snapshot = Snapshot {
        scenes: if asset.is_some() { &linked_scenes } else { std::slice::from_ref(scene) },
        texts: &[],
        world: &world,
        schema: &schema,
        characters: &characters,
        producers: &BTreeMap::new(),
        versions: ToolVersions::current(),
    };
    let dependencies = if let Some(asset) = asset {
        wobu_narrative_deps::capture::capture_text_variant(&asset, &snapshot, target.variant)
    } else {
        wobu_narrative_deps::capture::capture_scene_variant(scene, &snapshot, target.variant)
    }
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
    let mut historical = context.clone();
    historical.state = state;
    let rebuilt = rebuild_context(scene, target, historical);
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
            &[],
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
