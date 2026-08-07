//! Answering "what would this generate?" without generating it.
//!
//! The Inspector's live reference report and the aspect-ratio picker both read
//! the same plan a batch would run and negotiate the same first cell, so the
//! references, the price and the output size on screen are the ones that would
//! be spent. Nothing here queues a job or reserves against the ceiling.

use serde::Serialize;
use tauri::State;
use wobu_core::{Id, Node};
use wobu_imagine::{AspectRatio, Capabilities, ImageBackend, negotiate};
use wobu_influence::RefBucket;

use super::plan::{
    GenerateShot, GenerateSlider, GenerationPlanRequest, SeedIntent, VariantGrid,
    fragments_for_cell, locked_seed_of, prepare_generation_plan, resolve_generation_stack,
    resolve_seed,
};
use super::spend::{CostEstimate, cost_estimate_prices, image_price};
use super::{BackendPurpose, image_backend, selected_image_provider, unprobed_image_backend};
use crate::error::{Code, CommandResult, WobuError};
use crate::machine::MachineSettings;
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceBucketReport {
    pub(super) bucket: RefBucket,
    pub(super) label: &'static str,
    pub(super) kept: usize,
    pub(super) limit: Option<usize>,
    pub(super) dropped: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceLayerReport {
    pub(super) node_id: Option<Id>,
    pub(super) layer: wobu_core::Layer,
    pub(super) kept: usize,
    pub(super) dropped: usize,
    pub(super) reasons: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageReferenceReport {
    pub(super) buckets: Vec<ReferenceBucketReport>,
    pub(super) layers: Vec<ReferenceLayerReport>,
    pub(super) cost: Option<CostEstimate>,
    pub(super) locked_seed: Option<u64>,
}

/// The provider-owned aspect choices and the exact shape a generation would use.
///
/// ComfyUI deliberately reports an empty `Capabilities::aspect_ratios`: it can
/// accept arbitrary dimensions. The UI still needs a bounded, validated
/// vocabulary, so flexible backends offer `AspectRatio::ALL` while retaining a
/// flag that explains the policy. This keeps arbitrary text out of generation
/// requests without pretending a local backend has a vendor-enforced list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerationCapabilities {
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) aspect_ratios: Vec<AspectRatio>,
    pub(super) flexible_aspect: bool,
    pub(super) previews: Vec<ImageAspectPreview>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAspectPreview {
    pub(super) requested_aspect: AspectRatio,
    pub(super) actual_aspect: AspectRatio,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) substituted: bool,
}

/// Preview the active image provider's negotiated aspect before anything is
/// submitted to the job queue.
#[tauri::command]
pub async fn image_generation_capabilities(
    state: State<'_, AppState>,
    machine: State<'_, MachineSettings>,
    model: Option<String>,
) -> CommandResult<ImageGenerationCapabilities> {
    let selection = state.with(|project| selected_image_provider(project))?;
    let backend = image_backend(&selection.provider, &machine, BackendPurpose::Preview).await?;
    let model = selection.model(model, backend.default_model());
    Ok(aspect_capability_view(selection.provider, model.clone(), backend.capabilities(&model)))
}

pub(super) fn aspect_capability_view(
    provider: String,
    model: String,
    capabilities: Capabilities,
) -> ImageGenerationCapabilities {
    let flexible_aspect = capabilities.aspect_ratios.is_empty();
    let aspect_ratios = if flexible_aspect {
        AspectRatio::ALL.to_vec()
    } else {
        capabilities.aspect_ratios.clone()
    };
    let previews = AspectRatio::ALL
        .into_iter()
        .map(|requested_aspect| {
            // These are the same helpers execution's `negotiate` path uses;
            // the preview cannot grow a second nearest-ratio or fitting rule.
            let actual_aspect = capabilities.nearest_aspect(requested_aspect);
            let resolution = capabilities.resolution_for(requested_aspect);
            ImageAspectPreview {
                requested_aspect,
                actual_aspect,
                width: resolution.width,
                height: resolution.height,
                substituted: requested_aspect != actual_aspect,
            }
        })
        .collect();
    ImageGenerationCapabilities { provider, model, aspect_ratios, flexible_aspect, previews }
}

/// Provider-aware reference quotas and every image that negotiation withholds.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri exposes these as named bridge arguments.
pub fn image_reference_report(
    state: State<'_, AppState>,
    machine: State<'_, MachineSettings>,
    subject_id: Id,
    preset: Option<String>,
    sliders: Option<Vec<GenerateSlider>>,
    shot: Option<GenerateShot>,
    aspect: Option<String>,
    model: Option<String>,
    seed: Option<u64>,
    grid: Option<VariantGrid>,
) -> CommandResult<ImageReferenceReport> {
    let (nodes, selection) = state.with(|project| {
        let selection = selected_image_provider(project)?;
        Ok((project.world_nodes()?.to_vec(), selection))
    })?;
    let backend = unprobed_image_backend(&selection.provider, &machine, BackendPurpose::Preview)?;
    let model = selection.model(model, backend.default_model());
    let locked_seed = locked_seed_of(&nodes, subject_id);
    let (seed, seed_source) = resolve_seed(seed, locked_seed, SeedIntent::Estimate);
    reference_report_for_plan(
        &nodes,
        &selection.provider,
        &model,
        backend.as_ref(),
        GenerationPlanRequest {
            subject_id,
            preset_id: preset,
            sliders: sliders.unwrap_or_default(),
            shot: shot.unwrap_or_default(),
            aspect,
            seed,
            seed_source,
            locked_seed,
            grid,
        },
    )
}

/// The preview half of the shared planning path.
///
/// Everything here is read off the same [`PreparedGenerationPlan`] execution
/// runs on, and the reference budget is read off the same *cell*: the first
/// one, which is the image `plan_batch` will send first. A grid varies one axis
/// per cell and a named-view preset varies the view, so one report can only be
/// true about one of them — and reporting the viewless, gridless fragment set
/// instead would be a budget for an image this preset never sends.
pub(super) fn reference_report_for_plan(
    nodes: &[Node],
    provider: &str,
    model: &str,
    backend: &dyn ImageBackend,
    request: GenerationPlanRequest,
) -> CommandResult<ImageReferenceReport> {
    let locked_seed = request.locked_seed;
    let caps = backend.capabilities(model);
    let plan = prepare_generation_plan(nodes, request, &caps)?;
    let stack = resolve_generation_stack(nodes, &plan)?;
    let mut prices = Vec::with_capacity(plan.cells.len());
    let mut first = None;
    for cell in &plan.cells {
        let extracted = fragments_for_cell(&stack, cell, &plan.controls.user_prompt);
        let negotiated = negotiate(&extracted, cell.aspect, &caps);
        prices.extend(image_price(provider, model, negotiated.resolution()));
        if first.is_none() {
            first = Some(negotiated);
        }
    }
    let negotiated = first.ok_or_else(|| {
        WobuError::new(Code::Internal, "This preset produced no images to estimate.")
    })?;
    let cost = cost_estimate_prices(prices, plan.cells.len());
    let buckets = negotiated
        .images()
        .buckets()
        .iter()
        .map(|bucket| ReferenceBucketReport {
            bucket: bucket.bucket(),
            label: bucket.bucket().label(),
            kept: bucket.kept().len(),
            limit: bucket.cap().limit(),
            dropped: bucket.dropped().len(),
        })
        .collect();
    let layers = stack
        .sources()
        .iter()
        .map(|source| {
            let node_id = source.node_id();
            let kept = negotiated
                .images()
                .kept()
                .filter(|fragment| {
                    fragment.node_id() == node_id && fragment.layer() == source.layer
                })
                .count();
            let mut reasons: Vec<String> = negotiated
                .images()
                .dropped()
                .filter(|drop| {
                    drop.fragment.node_id() == node_id && drop.fragment.layer() == source.layer
                })
                .map(|_| "reference budget".to_owned())
                .collect();
            reasons.extend(
                negotiated
                    .downgrades()
                    .iter()
                    .filter(|drop| {
                        drop.fragment.node_id() == node_id && drop.fragment.layer() == source.layer
                    })
                    .map(|drop| drop.reason.label().to_owned()),
            );
            ReferenceLayerReport {
                node_id,
                layer: source.layer,
                kept,
                dropped: reasons.len(),
                reasons,
            }
        })
        .collect();
    Ok(ImageReferenceReport { buckets, layers, cost, locked_seed })
}
