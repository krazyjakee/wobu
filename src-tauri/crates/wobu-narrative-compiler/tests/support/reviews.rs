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

/// The same explicit host evidence for supporting text assets (#167).
///
/// `dead_code` because this module is included by several test binaries and each
/// uses the half it needs; the alternative is splitting one fixture into two
/// files that would then be free to disagree about what an approval proves.
#[allow(dead_code)]
///
/// Deliberately built from the same [`ReviewContext`] machinery rather than a
/// hand-written stub, so a fixture cannot pass a release gate that real
/// canonical history would fail.
pub fn fixture_text_reviews(
    asset: &wobu_narrative::TextAsset,
) -> BTreeMap<VariantId, TextApprovalEvidence> {
    asset
        .lines()
        .flat_map(|(entry, slot)| {
            slot.variants.iter().map(move |variant| {
                let target =
                    TextTarget { asset: asset.id, entry, slot: slot.id, variant: Some(variant.id) };
                let context = ReviewContext::capture_text(
                    asset,
                    &target,
                    serde_json::json!({}),
                    serde_json::json!({}),
                    serde_json::json!({}),
                    BTreeMap::new(),
                );
                (
                    variant.id,
                    TextApprovalEvidence {
                        binding: TextBinding {
                            target,
                            speaker: slot.speaker.clone(),
                            text_revision: variant.text.revision.clone(),
                            context_revision: context.revision.clone(),
                            state: context.state,
                            approved: true,
                            event_id: asset.id.raw(),
                        },
                        current_context: context.revision,
                    },
                )
            })
        })
        .collect()
}
