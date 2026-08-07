//! Turning one request into the batch of images it actually is.
//!
//! Step 4 of the chain in [`super`], for everything that is not a scene: it
//! reads the [`super::plan`] cells, negotiates the first one to learn what the
//! provider will really accept, and emits one receipt and one request per cell.
//! Nothing here talks to a provider twice — the negotiated resolution of the
//! first cell is what every later cell is measured against.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use serde_json::json;
use tauri::AppHandle;
use wobu_core::{Asset, Generation, Id, Node, new_id};
use wobu_imagine::{ImageBackend, ImageRequest, negotiate};
use wobu_influence::{Budget, compile};

use super::loras::{prompt_with_lora_triggers, resolve_loras};
use super::plan::{
    FragmentKey, GenerationPlanRequest, fragments_for_cell, prepare_generation_plan,
    resolve_generation_stack, snapshot,
};
use super::spend::image_price;
use super::task::{GenerateTask, PlannedBatch, PlannedImage};
use super::{ReceiptPreparation, ReferenceLoader, ReferenceScope, load_references};
use crate::error::{Code, CommandResult, WobuError};

pub(super) struct Prepare {
    pub(super) root: PathBuf,
    pub(super) project_id: Id,
    pub(super) nodes: Vec<Node>,
    pub(super) assets: Vec<Asset>,
    pub(super) request: GenerationPlanRequest,
    pub(super) model: String,
    pub(super) backend: Arc<dyn ImageBackend>,
    pub(super) provider: String,
    pub(super) app: AppHandle,
}

/// The batch planning seam.
///
/// Everything downstream of [`prepare_generation_plan`] — negotiation, prompt
/// compilation, reference loading, pricing and receipt preparation — happens
/// here, from borrowed project state and a borrowed adapter. Nothing in it can
/// reach the running app, which is what lets the same call the Inspector's
/// preview makes be exercised directly by a test.
pub(super) struct BatchPlan<'a> {
    pub(super) root: &'a Path,
    pub(super) nodes: &'a [Node],
    pub(super) assets: &'a [Asset],
    pub(super) request: GenerationPlanRequest,
    pub(super) model: &'a str,
    pub(super) provider: &'a str,
    pub(super) backend: &'a dyn ImageBackend,
}

pub(super) fn prepare(input: Prepare) -> CommandResult<GenerateTask> {
    let planned = plan_batch(BatchPlan {
        root: &input.root,
        nodes: &input.nodes,
        assets: &input.assets,
        request: input.request,
        model: &input.model,
        provider: &input.provider,
        backend: input.backend.as_ref(),
    })?;
    Ok(planned.into_task(input.root, input.project_id, input.backend, input.app))
}

pub(super) fn plan_batch(input: BatchPlan<'_>) -> CommandResult<PlannedBatch> {
    let caps = input.backend.capabilities(input.model);
    let plan = prepare_generation_plan(input.nodes, input.request, &caps)?;
    let stack = resolve_generation_stack(input.nodes, &plan)?;
    let assets: HashMap<Id, &Asset> = input.assets.iter().map(|asset| (asset.id, asset)).collect();
    let loras = resolve_loras(
        input.root,
        input.nodes,
        stack.sources().iter().filter_map(|source| source.node_id()),
        input.model,
        input.backend,
    );
    let batch_size = plan.cells.len();
    let mut plans = Vec::with_capacity(batch_size);
    let mut reference_loader = ReferenceLoader::new();
    for (batch_index, cell) in plan.cells.iter().enumerate() {
        let extracted = fragments_for_cell(&stack, cell, &plan.controls.user_prompt);
        let negotiated = negotiate(&extracted, cell.aspect, &caps);
        let compiled = compile(negotiated.fragments(), Budget::unlimited());
        let compiled_prompt = prompt_with_lora_triggers(compiled.prompt(), &loras.weights);
        let references = load_references(
            &negotiated,
            &assets,
            input.root,
            &mut reference_loader,
            ReferenceScope::Image,
        )?;
        let dropped: Vec<FragmentKey> = compiled
            .dropped()
            .iter()
            .map(|drop| FragmentKey::of(drop.fragment))
            .chain(negotiated.images().dropped().map(|drop| FragmentKey::of(drop.fragment)))
            .chain(negotiated.downgrades().iter().map(|drop| FragmentKey::of(drop.fragment)))
            .collect();
        let resolution = negotiated.resolution();
        let price = image_price(input.provider, input.model, resolution);
        let cost_usd_micros = price.map_or(0, |price| price.per_image_usd_micros);
        let reference_asset_ids =
            references.iter().map(|reference| reference.asset_id).collect::<Vec<_>>();
        let mut params = ReceiptPreparation {
            batch_index,
            batch_size,
            requested_aspect: cell.aspect,
            actual_aspect: negotiated.aspect(),
            resolution,
            negative_prompt_supported: caps.negative_prompt,
            seed_source: cell.seed_source,
            cost_usd_micros,
            reference_asset_ids: &reference_asset_ids,
            loras: &loras.receipts,
            lora_downgrades: &loras.downgrades,
            price,
            controls: plan.controls.receipt_controls(&cell.slider_values),
        }
        .params();
        if let Some(locked_seed) = plan.locked_seed {
            params.insert("lockedSeed".into(), json!(locked_seed));
            params.insert("usedLockedSeed".into(), json!(cell.item.seed == locked_seed));
        }
        if let Some(variation) = &cell.variation {
            params.insert(
                "variation".into(),
                serde_json::to_value(variation).map_err(|error| {
                    WobuError::new(
                        Code::Internal,
                        "The variant grid metadata could not be encoded.",
                    )
                    .with_detail(error.to_string())
                })?,
            );
        }
        let request = ImageRequest::new(
            input.model.to_owned(),
            &compiled_prompt,
            cell.item.seed,
            &negotiated,
        )
        .with_negative(compiled.negative())
        .with_references(references)
        .with_loras(loras.weights.clone());
        plans.push(PlannedImage {
            request,
            cost_usd_micros,
            generation: Generation {
                id: new_id(),
                node_id: plan.subject_id,
                created_at: Utc::now(),
                preset: cell.preset.id.to_owned(),
                view_type: cell.item.view.map(|view| view.view_type.to_owned()),
                user_prompt: plan.controls.user_prompt.clone(),
                compiled_prompt,
                negative_prompt: compiled.negative().to_owned(),
                backend: input.provider.to_owned(),
                model: input.model.to_owned(),
                seed: cell.item.seed,
                params,
                output_asset_ids: Vec::new(),
                influence_snapshot: snapshot(
                    &stack,
                    &extracted,
                    &cell.slider_values,
                    &plan.controls.muted_nodes,
                    &dropped,
                ),
            },
        });
    }

    Ok(PlannedBatch {
        label: format!("Generate {} ×{}", plan.subject_name, plans.len()),
        subject_id: plan.subject_id,
        plans,
        requires_billing: caps.requires_billing,
        archival_replay: false,
    })
}
