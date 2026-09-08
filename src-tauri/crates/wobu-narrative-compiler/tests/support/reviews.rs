use std::collections::BTreeMap;
use wobu_narrative::{Scene, VariantId, review::*};

/// Explicit host evidence for pure compiler/package fixtures. Canonical receipt
/// verification is exercised by store tests, not simulated by writable scene flags.
pub fn fixture_reviews(scene: &Scene) -> BTreeMap<VariantId, ApprovalEvidence> {
    scene
        .dialogue_slots()
        .flat_map(|(beat, slot)| {
            slot.variants.iter().map(move |variant| {
                let target = ReviewTarget {
                    scene: scene.id,
                    beat,
                    slot: slot.id,
                    variant: Some(variant.id),
                };
                let context = ReviewContext::capture(
                    scene,
                    &target,
                    serde_json::json!({}),
                    serde_json::json!({}),
                    serde_json::json!({}),
                    BTreeMap::new(),
                );
                (
                    variant.id,
                    ApprovalEvidence {
                        binding: ReviewBinding {
                            target,
                            speaker: slot.speaker.clone(),
                            text_revision: variant.text.revision.clone(),
                            context_revision: context.revision.clone(),
                            state: context.state,
                            approved: true,
                            event_id: scene.id.raw(),
                        },
                        current_context: context.revision,
                    },
                )
            })
        })
        .collect()
}
