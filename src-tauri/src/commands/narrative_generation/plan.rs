use super::*;
use serde::Deserialize;
use std::collections::BTreeMap;
use wobu_narrative::{Name, SceneId, VarType, VariantId};
use wobu_narrative_context::{Options, Selection};
use wobu_narrative_generation::{
    Candidate, MAX_BATCH, OUTPUT_SCHEMA_VERSION, PROMPT_VERSION, SYSTEM, Settings, VERSION,
    output_schema, prompt,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanInput {
    pub scene: SceneId,
    /// Missing means all empty slots/empty authored variants in this scene.
    pub selection: Option<Selection>,
    pub state: BTreeMap<Name, wobu_narrative::Value>,
    pub commands: BTreeMap<Name, Vec<VarType>>,
    pub token_budget: u32,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Skipped {
    pub target: Selection,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub id: Id,
    pub requests: Vec<FrozenRequest>,
    pub skipped: Vec<Skipped>,
    pub provider: String,
    pub model: String,
}

pub fn build(
    project: &Project,
    input: PlanInput,
    provider: &str,
    model: &str,
) -> CommandResult<Plan> {
    if project.is_read_only() {
        return Err(wobu_store::Error::ReadOnly.into());
    }
    let fingerprint = project.narrative_fingerprint()?;
    let report = super::super::narrative_preview::compile_project(project, input.commands)?;
    let graph = report
        .graph
        .ok_or_else(|| invalid("Repair compiler errors before generating dialogue."))?;
    let file = project.load_scene(input.scene)?;
    let scene_hash =
        file.stamp.as_ref().ok_or_else(|| invalid("Save the scene first."))?.hash.clone();
    if input.selection.as_ref().is_some_and(|selection| selection.scene != input.scene) {
        return Err(invalid("Selected line belongs to another scene."));
    }
    let mut plan = Plan {
        id: wobu_core::new_id(),
        requests: Vec::new(),
        skipped: Vec::new(),
        provider: provider.into(),
        model: model.into(),
    };
    let mut targets = Vec::new();
    for beat in &file.scene.beats {
        for slot in &beat.dialogue {
            if let Some(selected) = &input.selection {
                if selected.beat == beat.id && selected.slot == slot.id {
                    targets.push((selected.clone(), slot));
                }
            } else if slot.variants.is_empty() {
                targets.push((
                    Selection { scene: input.scene, beat: beat.id, slot: slot.id, variant: None },
                    slot,
                ));
            } else {
                for variant in &slot.variants {
                    if variant.text.body.trim().is_empty() {
                        targets.push((
                            Selection {
                                scene: input.scene,
                                beat: beat.id,
                                slot: slot.id,
                                variant: Some(variant.id),
                            },
                            slot,
                        ));
                    }
                }
            }
        }
    }
    if input.selection.is_some() && targets.is_empty() {
        return Err(invalid("Selected dialogue slot no longer exists."));
    }
    for (target, slot) in targets {
        let variant = target
            .variant
            .map(|id| {
                slot.variants
                    .iter()
                    .find(|v| v.id == id)
                    .ok_or_else(|| invalid("Selected variant no longer exists."))
            })
            .transpose()?;
        if variant.is_none() && !slot.variants.is_empty() {
            return Err(invalid(
                "Choose an existing variant; generation cannot append a new branch to a populated slot.",
            ));
        }
        if slot.policy == wobu_narrative::GenerationPolicy::Locked
            || variant.is_some_and(|v| !v.text.lifecycle.may_generate())
        {
            plan.skipped.push(Skipped { target, reason: "Locked dialogue is excluded.".into() });
            continue;
        }
        if plan.requests.len() == MAX_BATCH {
            return Err(invalid(
                "A generation batch may contain at most 32 lines. Narrow the selection.",
            ));
        }
        // Resolve from the same saved source read; double fingerprint checks below catch races.
        let context = super::super::narrative_context::capture(
            project,
            Options {
                selection: target.clone(),
                state: input.state.clone(),
                token_budget: input.token_budget,
            },
            || {},
        )?;
        if !context.ready {
            plan.skipped.push(Skipped {
                target,
                reason: context
                    .diagnostics
                    .iter()
                    .filter(|d| d.blocking)
                    .map(|d| d.message.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            });
            continue;
        }
        let candidate_variant_id = variant.map(|v| v.id).unwrap_or_else(VariantId::new);
        let candidate = Candidate {
            slot_id: slot.id,
            variant_id: candidate_variant_id,
            speaker: slot.speaker.clone(),
            text: String::new(),
        };
        let request = FrozenRequest {
            version: VERSION,
            source_schema_version: wobu_narrative::SOURCE_SCHEMA_VERSION,
            request_id: wobu_core::new_id(),
            batch_id: plan.id,
            target,
            candidate_variant_id,
            speaker: slot.speaker.clone(),
            expected_scene_hash: scene_hash.clone(),
            compiled_graph_hash: graph.hash(),
            expected_text_revision: variant.map(|v| v.text.revision.clone()),
            expected_policy: variant.map(|v| v.text.lifecycle.policy),
            expected_slot_policy: slot.policy,
            provider: provider.into(),
            model: model.into(),
            settings: Settings { max_output_tokens: input.max_output_tokens },
            prompt_version: PROMPT_VERSION,
            output_schema_version: OUTPUT_SCHEMA_VERSION,
            output_schema: output_schema(),
            system: SYSTEM.into(),
            prompt: prompt(&context, &candidate),
            context,
        };
        request.validate().map_err(invalid)?;
        plan.requests.push(request);
    }
    if fingerprint != project.narrative_fingerprint()? {
        return Err(invalid("Source changed during generation planning. Plan again."));
    }
    Ok(plan)
}
