//! One request constructor for single-scene generation and affected builds.
use super::*;
use wobu_narrative::{DialogueSlot, Scene};
use wobu_narrative_context::{FrozenContext, Selection};
use wobu_narrative_generation::{
    Candidate, OUTPUT_SCHEMA_VERSION, PROMPT_VERSION, REQUEST_VERSION, SYSTEM, Settings,
    output_schema, prompt,
};

pub struct Input<'a> {
    pub batch: Id,
    pub scene: &'a Scene,
    pub slot: &'a DialogueSlot,
    pub target: Selection,
    pub context: FrozenContext,
    pub graph: String,
    pub provider: &'a str,
    pub model: &'a str,
    pub max_output_tokens: u32,
}
pub fn request(input: Input<'_>) -> CommandResult<FrozenRequest> {
    let variant =
        input.target.variant.and_then(|id| input.slot.variants.iter().find(|v| v.id == id));
    let candidate_variant_id = variant.map(|v| v.id).unwrap_or_default();
    let candidate = Candidate {
        slot_id: input.slot.id,
        variant_id: candidate_variant_id,
        speaker: input.slot.speaker.clone(),
        text: String::new(),
    };
    let request = FrozenRequest {
        version: REQUEST_VERSION,
        source_schema_version: wobu_narrative::SCENE_SCHEMA_VERSION,
        request_id: wobu_core::new_id(),
        batch_id: input.batch,
        target: input.target.clone(),
        candidate_variant_id,
        speaker: input.slot.speaker.clone(),
        expected_scene_hash: wobu_store::project::narrative_generation::target_guard(
            input.scene,
            &input.target,
        ),
        compiled_graph_hash: input.graph,
        expected_text_revision: variant.map(|v| v.text.revision.clone()),
        expected_policy: variant.map(|v| v.text.lifecycle.policy),
        expected_slot_policy: input.slot.policy,
        provider: input.provider.into(),
        model: input.model.into(),
        settings: Settings { max_output_tokens: input.max_output_tokens },
        prompt_version: PROMPT_VERSION,
        output_schema_version: OUTPUT_SCHEMA_VERSION,
        output_schema: output_schema(),
        system: SYSTEM.into(),
        prompt: prompt(&input.context, &candidate),
        context: input.context,
    };
    request.validate().map_err(invalid)?;
    Ok(request)
}
