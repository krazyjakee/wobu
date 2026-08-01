//! Preparing and running one image generation from the Inspector.
//!
//! The command resolves and negotiates before it queues anything, so bad input,
//! a missing provider, a missing key or an unreadable reference fails without
//! starting a paid job. The task owns only an immutable request and a project
//! path; it never holds the open-project mutex across a provider call.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tauri::{AppHandle, Emitter, State};
use wobu_core::{
    Asset, AssetKind, FragmentTarget, Generation, GenerationVariation, Id, InfluenceSnapshot, Node,
    Preset, PresetGeneration, SnapshotFragment, SnapshotLayer, VariationValue, default_preset,
    new_id, preset,
};
use wobu_imagine::{
    AspectRatio, Capabilities, ComfyBackend, Error as ImageError, GeminiBackend, ImageBackend,
    ImageRequest, ImageUsage, ProgressSink, Reference, Resolution, comfy, gemini, negotiate,
};
use wobu_influence::{
    Budget, Fragment, FragmentBody, RefBucket, Shot, Sliders, World, compile, fragments,
    fragments_for_view, resolve,
};
use wobu_jobs::{Billed, Failure, JobContext, JobKind, Outcome, Preview, Progress, Task};
use wobu_store::Project;

use crate::error::{Code, CommandResult, WobuError};
use crate::keys::Keys;
use crate::state::{AppState, Jobs};

pub const GENERATION_RECORDED: &str = "generation:recorded";
const PRICE_SOURCE: &str = "https://ai.google.dev/gemini-api/docs/pricing";
const PRICE_CHECKED_AT: &str = "2026-08-01";
const SPEND_DIR: &str = ".wobu/spend";
const LOCK_ATTEMPTS: usize = 200;

#[derive(Debug, Clone, Copy)]
struct Price {
    per_image_usd_micros: u64,
    conservative_fallback: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostEstimate {
    currency: &'static str,
    per_image_usd_micros: u64,
    batch_usd_micros: u64,
    images: usize,
    varies_by_cell: bool,
    indicative: bool,
    conservative_fallback: bool,
    checked_at: &'static str,
    source_url: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendStatus {
    ceiling_usd_micros: Option<u64>,
    spent_usd_micros: u64,
    reserved_usd_micros: u64,
    remaining_usd_micros: Option<u64>,
    pending_reservations: usize,
    oldest_reservation_at: Option<String>,
    ledger_locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReservationFile {
    id: Id,
    remaining_usd_micros: u64,
    created_at: String,
}

const MAX_GRID_CELLS: usize = 16;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "axis", rename_all = "snake_case")]
pub enum VariantGrid {
    Seed { values: Vec<u64> },
    FragmentWeight {
        #[serde(rename = "nodeId")]
        node_id: Id,
        values: Vec<f32>,
    },
    Preset { values: Vec<String> },
    Aspect { values: Vec<String> },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum SeedSource {
    Locked,
    LockedDerived,
    Rerolled,
    RerolledDerived,
    Random,
    RandomDerived,
    Grid,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateSlider {
    node_id: Id,
    value: f32,
    #[serde(default)]
    muted: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateShot {
    label: Option<String>,
    weight: Option<f32>,
    prompt: Option<String>,
}

/// Start one image and return the queue id immediately.
#[tauri::command]
pub fn generate_start(
    app: AppHandle,
    state: State<'_, AppState>,
    jobs: State<'_, Jobs>,
    keys: State<'_, Keys>,
    subject_id: Id,
    preset: Option<String>,
    sliders: Option<Vec<GenerateSlider>>,
    shot: Option<GenerateShot>,
    aspect: Option<String>,
    model: Option<String>,
    seed: Option<u64>,
    grid: Option<VariantGrid>,
) -> CommandResult<String> {
    let (root, project_id, nodes, assets, provider, selected_model) = state.with(|project| {
        if project.is_read_only() {
            return Err(WobuError::new(
                Code::ReadOnly,
                "This project is read-only, so a generated image could not be saved.",
            ));
        }
        let selected = project
            .meta()
            .providers
            .get("image")
            .and_then(Value::as_object)
            .ok_or_else(no_image_provider)?;
        let provider = selected
            .get("provider")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(no_image_provider)?
            .to_owned();
        let selected_model = selected
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        Ok((
            project.root().to_path_buf(),
            project.id(),
            project.world_nodes()?.to_vec(),
            project.list_assets()?,
            provider,
            selected_model,
        ))
    })?;

    let backend: Arc<dyn ImageBackend> = match provider.as_str() {
        comfy::ID => Arc::new(ComfyBackend::new(comfy::DEFAULT_URL).map_err(|error| {
            WobuError::new(Code::ProviderUnavailable, error.to_string())
        })?),
        gemini::ID => {
            let secret = keys.secret(gemini::ID).ok_or_else(|| {
                WobuError::new(
                    Code::ProviderNoKey,
                    "Gemini is selected for images, but there is no key on this machine. Add one in Settings.",
                )
            })?;
            Arc::new(GeminiBackend::new(secret.expose()).map_err(|error| {
                WobuError::new(Code::ProviderUnavailable, error.to_string())
            })?)
        }
        other => {
            return Err(WobuError::new(
                Code::Invalid,
                format!("This build has no image adapter for {other}."),
            ));
        }
    };
    let model = model
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or(selected_model)
        .unwrap_or_else(|| backend.default_model().to_owned());
    let locked_seed = nodes
        .iter()
        .find(|node| node.id == subject_id)
        .and_then(|node| node.locked_seed);
    let (seed, seed_source) = match (seed, locked_seed) {
        (Some(seed), _) => (seed, SeedSource::Rerolled),
        (None, Some(seed)) => (seed, SeedSource::Locked),
        (None, None) => (u128::from(new_id()) as u64, SeedSource::Random),
    };

    let mut plan = prepare(Prepare {
        root,
        project_id,
        nodes,
        assets,
        subject_id,
        preset_id: preset,
        sliders: sliders.unwrap_or_default(),
        shot: shot.unwrap_or_default(),
        aspect,
        model,
        seed,
        seed_source,
        locked_seed,
        grid,
        backend,
        provider,
        app,
    })?;
    plan.reserve_spend()?;
    let id = jobs.queue().submit(plan);
    Ok(id.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceBucketReport {
    bucket: RefBucket,
    label: &'static str,
    kept: usize,
    limit: Option<usize>,
    dropped: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceLayerReport {
    node_id: Option<Id>,
    layer: wobu_core::Layer,
    kept: usize,
    dropped: usize,
    reasons: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageReferenceReport {
    buckets: Vec<ReferenceBucketReport>,
    layers: Vec<ReferenceLayerReport>,
    cost: Option<CostEstimate>,
    spend: SpendStatus,
    locked_seed: Option<u64>,
}

/// Reconstructed, never trusted from a mutable counter.
#[tauri::command]
pub fn spend_status(state: State<'_, AppState>) -> CommandResult<SpendStatus> {
    let root = state.with(|project| Ok(project.root().to_path_buf()))?;
    spend_status_for_report(&root)
}

/// Change the shared hard ceiling. `null` disables paid generation rather than
/// turning the guard off; local ComfyUI remains unaffected.
#[tauri::command]
pub fn spend_ceiling_set(
    state: State<'_, AppState>,
    ceiling_usd_micros: Option<u64>,
) -> CommandResult<SpendStatus> {
    let root = state.with(|project| {
        if project.is_read_only() {
            return Err(WobuError::new(
                Code::ReadOnly,
                "This project is read-only, so its spend ceiling cannot be changed.",
            ));
        }
        let root = project.root().to_path_buf();
        let _guard = SpendLock::acquire(&root)?;
        project.set_spend_ceiling(ceiling_usd_micros)?;
        Ok(root)
    })?;
    spend_status_for(&root)
}

/// Archive reservations after a crash. This is deliberately explicit and
/// refuses while this process has paid work queued or running. Another machine
/// cannot be interrogated reliably, so the UI requires the user to confirm all
/// other Wobu instances using the project have stopped paid work.
#[tauri::command]
pub fn spend_recovery_reset(
    state: State<'_, AppState>,
    jobs: State<'_, Jobs>,
    confirm_no_paid_jobs: bool,
) -> CommandResult<SpendStatus> {
    if !confirm_no_paid_jobs {
        return Err(WobuError::new(
            Code::Invalid,
            "Confirm that no paid generations are running before recovering spend reservations.",
        ));
    }
    if jobs
        .snapshot()
        .jobs
        .iter()
        .any(|job| job.kind == JobKind::Generate && !job.state.is_terminal())
    {
        return Err(WobuError::new(
            Code::Invalid,
            "A generation is still queued or running in this Wobu window.",
        ));
    }
    let root = state.with(|project| {
        if project.is_read_only() {
            return Err(WobuError::new(
                Code::ReadOnly,
                "This project is read-only, so spend recovery cannot be changed.",
            ));
        }
        Ok(project.root().to_path_buf())
    })?;
    let ledger = root.join(SPEND_DIR);
    if ledger.exists() {
        let archive = root
            .join(".wobu")
            .join(format!("spend-recovery-{}-{}", Utc::now().format("%Y%m%dT%H%M%SZ"), new_id()));
        std::fs::rename(&ledger, &archive).map_err(|error| {
            spend_io("The pending spend ledger could not be archived.", &ledger, error)
        })?;
    }
    spend_status_for(&root)
}

/// Provider-aware reference quotas and every image that negotiation withholds.
#[tauri::command]
pub fn image_reference_report(
    state: State<'_, AppState>,
    subject_id: Id,
    preset: Option<String>,
    sliders: Option<Vec<GenerateSlider>>,
    shot: Option<GenerateShot>,
    aspect: Option<String>,
    model: Option<String>,
    seed: Option<u64>,
    grid: Option<VariantGrid>,
) -> CommandResult<ImageReferenceReport> {
    let (root, nodes, provider, selected_model) = state.with(|project| {
        let selected = project
            .meta()
            .providers
            .get("image")
            .and_then(Value::as_object)
            .ok_or_else(no_image_provider)?;
        let provider = selected
            .get("provider")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(no_image_provider)?
            .to_owned();
        let selected_model = selected
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        Ok((project.root().to_path_buf(), project.world_nodes()?.to_vec(), provider, selected_model))
    })?;
    let backend: Box<dyn ImageBackend> = match provider.as_str() {
        comfy::ID => Box::new(ComfyBackend::new(comfy::DEFAULT_URL).map_err(|error| {
            WobuError::new(Code::ProviderUnavailable, error.to_string())
        })?),
        gemini::ID => Box::new(GeminiBackend::new("capability-preview").map_err(|error| {
            WobuError::new(Code::ProviderUnavailable, error.to_string())
        })?),
        other => {
            return Err(WobuError::new(
                Code::Invalid,
                format!("This build has no image adapter for {other}."),
            ));
        }
    };
    let model = model
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or(selected_model)
        .unwrap_or_else(|| backend.default_model().to_owned());
    let subject = nodes.iter().find(|node| node.id == subject_id).ok_or_else(|| {
        WobuError::new(Code::NoSuchNode, "That entity is not in this project any more.")
    })?;
    let locked_seed = subject.locked_seed;
    let chosen = preset
        .as_deref()
        .and_then(wobu_core::preset)
        .filter(|candidate| candidate.applies_to(subject.kind))
        .unwrap_or_else(|| default_preset(subject.kind));
    let controls = shot.unwrap_or_default();
    let shot_label = controls.label.as_deref().unwrap_or(chosen.label);
    let world = World::new(nodes.iter());
    let stack = resolve(
        &world,
        subject_id,
        Some(Shot {
            label: shot_label,
            weight: controls.weight.unwrap_or(1.0).clamp(0.0, 1.0),
        }),
    )
    .ok_or_else(|| WobuError::new(Code::NoSuchNode, "That entity is not in this project any more."))?;
    let slider_values: Vec<(Id, f32)> = sliders
        .unwrap_or_default()
        .into_iter()
        .map(|slider| (slider.node_id, if slider.muted { 0.0 } else { slider.value }))
        .collect();
    let sliders = Sliders::from_pairs(slider_values.iter().copied());
    let mut extracted = fragments(&stack, chosen, &sliders);
    append_user_prompt(
        &stack,
        &mut extracted,
        controls.prompt.as_deref().map(str::trim).unwrap_or(""),
    );
    let requested_aspect = aspect.as_deref().unwrap_or(chosen.aspect);
    let aspect = AspectRatio::parse(requested_aspect).ok_or_else(|| {
        WobuError::new(Code::Invalid, "That is not a supported aspect ratio.")
            .with_detail(requested_aspect.to_owned())
    })?;
    let negotiated = negotiate(&extracted, aspect, &backend.capabilities(&model));
    let estimate_seed = seed.or(locked_seed).unwrap_or(0);
    let cells = variant_cells(
        subject,
        *chosen,
        aspect,
        estimate_seed,
        SeedSource::Random,
        locked_seed,
        &slider_values,
        &stack.sources().iter().filter_map(|source| source.node_id()).collect(),
        grid.as_ref(),
        &backend.capabilities(&model),
    )?;
    let prices: Vec<Price> = cells
        .iter()
        .filter_map(|cell| {
            let negotiated = negotiate(&extracted, cell.aspect, &backend.capabilities(&model));
            image_price(&provider, &model, negotiated.resolution())
        })
        .collect();
    let cost = cost_estimate_prices(prices, cells.len());
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
                .filter(|fragment| fragment.node_id() == node_id && fragment.layer() == source.layer)
                .count();
            let mut reasons: Vec<String> = negotiated
                .images()
                .dropped()
                .filter(|drop| drop.fragment.node_id() == node_id && drop.fragment.layer() == source.layer)
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
    Ok(ImageReferenceReport {
        buckets,
        layers,
        cost,
        spend: spend_status_for_report(&root)?,
        locked_seed,
    })
}

#[derive(Debug, Clone)]
struct VariantCell {
    preset: Preset,
    item: PresetGeneration,
    aspect: AspectRatio,
    slider_values: Vec<(Id, f32)>,
    seed_source: SeedSource,
    variation: Option<GenerationVariation>,
}

#[allow(clippy::too_many_arguments)]
fn variant_cells(
    subject: &Node,
    chosen: Preset,
    base_aspect: AspectRatio,
    base_seed: u64,
    base_seed_source: SeedSource,
    _locked_seed: Option<u64>,
    slider_values: &[(Id, f32)],
    available_nodes: &HashSet<Id>,
    grid: Option<&VariantGrid>,
    caps: &Capabilities,
) -> CommandResult<Vec<VariantCell>> {
    let Some(grid) = grid else {
        return Ok(chosen
            .generations(base_seed)
            .into_iter()
            .map(|item| VariantCell {
                preset: chosen,
                seed_source: if item.seed == base_seed {
                    base_seed_source
                } else {
                    derived_seed_source(base_seed_source)
                },
                item,
                aspect: base_aspect,
                slider_values: slider_values.to_vec(),
                variation: None,
            })
            .collect());
    };

    if !chosen.views.is_empty() {
        return Err(WobuError::new(
            Code::Invalid,
            "Variant grids are not available for named-view presets such as Turnaround.",
        ));
    }
    let total = grid_len(grid)?;
    let grid_id = new_id();
    let total_u16 = total as u16;
    let common = |index: usize, preset: Preset, seed: u64, aspect: AspectRatio, values| {
        VariantCell {
            preset,
            item: PresetGeneration { index: index as u8, seed, view: None },
            aspect,
            slider_values: values,
            seed_source: if matches!(grid, VariantGrid::Seed { .. }) {
                SeedSource::Grid
            } else {
                base_seed_source
            },
            variation: None,
        }
    };

    let cells = match grid {
        VariantGrid::Seed { values } => {
            require_distinct(values.iter().copied(), "seed")?;
            values
                .iter()
                .copied()
                .enumerate()
                .map(|(index, seed)| {
                    let mut cell = common(
                        index,
                        chosen,
                        seed,
                        base_aspect,
                        slider_values.to_vec(),
                    );
                    cell.variation = Some(GenerationVariation {
                        grid_id,
                        index: index as u16,
                        total: total_u16,
                        value: VariationValue::Seed { seed },
                    });
                    cell
                })
                .collect()
        }
        VariantGrid::FragmentWeight { node_id, values } => {
            if !available_nodes.contains(node_id) {
                return Err(WobuError::new(
                    Code::Invalid,
                    "The fragment-weight grid names a layer outside this influence stack.",
                ));
            }
            if values.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
                return Err(WobuError::new(
                    Code::Invalid,
                    "Variant fragment weights must be between 0 and 1.",
                ));
            }
            let distinct: Vec<u32> = values.iter().map(|value| value.to_bits()).collect();
            require_distinct(distinct.into_iter(), "fragment weight")?;
            values
                .iter()
                .copied()
                .enumerate()
                .map(|(index, weight)| {
                    let mut varied = slider_values.to_vec();
                    if let Some((_, value)) = varied.iter_mut().find(|(id, _)| id == node_id) {
                        *value = weight;
                    } else {
                        varied.push((*node_id, weight));
                    }
                    let mut cell = common(index, chosen, base_seed, base_aspect, varied);
                    cell.variation = Some(GenerationVariation {
                        grid_id,
                        index: index as u16,
                        total: total_u16,
                        value: VariationValue::FragmentWeight { node_id: *node_id, weight },
                    });
                    cell
                })
                .collect()
        }
        VariantGrid::Preset { values } => {
            let ids: Vec<String> = values.iter().map(|value| value.trim().to_owned()).collect();
            require_distinct(ids.iter().cloned(), "preset")?;
            let mut cells = Vec::with_capacity(total);
            for (index, id) in ids.into_iter().enumerate() {
                let candidate = preset(&id)
                    .filter(|candidate| candidate.applies_to(subject.kind))
                    .ok_or_else(|| {
                        WobuError::new(
                            Code::Invalid,
                            format!("{id} is not an output preset for this entity."),
                        )
                    })?;
                if !candidate.views.is_empty() {
                    return Err(WobuError::new(
                        Code::Invalid,
                        "Named-view presets cannot be cells in a variant grid.",
                    ));
                }
                let mut cell = common(
                    index,
                    *candidate,
                    base_seed,
                    base_aspect,
                    slider_values.to_vec(),
                );
                cell.variation = Some(GenerationVariation {
                    grid_id,
                    index: index as u16,
                    total: total_u16,
                    value: VariationValue::Preset { preset: id },
                });
                cells.push(cell);
            }
            cells
        }
        VariantGrid::Aspect { values } => {
            let mut aspects = Vec::with_capacity(total);
            for value in values {
                let parsed = AspectRatio::parse(value.trim()).ok_or_else(|| {
                    WobuError::new(Code::Invalid, format!("{value} is not a supported aspect ratio."))
                })?;
                if !caps.aspect_ratios.is_empty() && !caps.aspect_ratios.contains(&parsed) {
                    return Err(WobuError::new(
                        Code::Invalid,
                        format!("The selected image model does not support {value}."),
                    ));
                }
                if aspects.contains(&parsed) {
                    return Err(WobuError::new(
                        Code::Invalid,
                        "Every aspect cell must have a different value.",
                    ));
                }
                aspects.push(parsed);
            }
            aspects
                .into_iter()
                .enumerate()
                .map(|(index, aspect)| {
                    let mut cell = common(
                        index,
                        chosen,
                        base_seed,
                        aspect,
                        slider_values.to_vec(),
                    );
                    cell.variation = Some(GenerationVariation {
                        grid_id,
                        index: index as u16,
                        total: total_u16,
                        value: VariationValue::Aspect { aspect: aspect.to_string() },
                    });
                    cell
                })
                .collect()
        }
    };
    Ok(cells)
}

fn grid_len(grid: &VariantGrid) -> CommandResult<usize> {
    let len = match grid {
        VariantGrid::Seed { values } => values.len(),
        VariantGrid::FragmentWeight { values, .. } => values.len(),
        VariantGrid::Preset { values } | VariantGrid::Aspect { values } => values.len(),
    };
    if !(2..=MAX_GRID_CELLS).contains(&len) {
        return Err(WobuError::new(
            Code::Invalid,
            format!("A variant grid needs 2 to {MAX_GRID_CELLS} cells."),
        ));
    }
    Ok(len)
}

fn require_distinct<T: Eq + std::hash::Hash>(
    values: impl IntoIterator<Item = T>,
    label: &str,
) -> CommandResult<()> {
    let mut seen = HashSet::new();
    if values.into_iter().any(|value| !seen.insert(value)) {
        return Err(WobuError::new(
            Code::Invalid,
            format!("Every {label} grid cell must have a different value."),
        ));
    }
    Ok(())
}

fn derived_seed_source(source: SeedSource) -> SeedSource {
    match source {
        SeedSource::Locked | SeedSource::LockedDerived => SeedSource::LockedDerived,
        SeedSource::Rerolled | SeedSource::RerolledDerived => SeedSource::RerolledDerived,
        SeedSource::Random | SeedSource::RandomDerived => SeedSource::RandomDerived,
        SeedSource::Grid => SeedSource::Grid,
    }
}

struct Prepare {
    root: PathBuf,
    project_id: Id,
    nodes: Vec<Node>,
    assets: Vec<Asset>,
    subject_id: Id,
    preset_id: Option<String>,
    sliders: Vec<GenerateSlider>,
    shot: GenerateShot,
    aspect: Option<String>,
    model: String,
    seed: u64,
    seed_source: SeedSource,
    locked_seed: Option<u64>,
    grid: Option<VariantGrid>,
    backend: Arc<dyn ImageBackend>,
    provider: String,
    app: AppHandle,
}

fn prepare(input: Prepare) -> CommandResult<GenerateTask> {
    let subject = input
        .nodes
        .iter()
        .find(|node| node.id == input.subject_id)
        .ok_or_else(|| {
            WobuError::new(Code::NoSuchNode, "That entity is not in this project any more.")
        })?;
    let chosen = input
        .preset_id
        .as_deref()
        .and_then(preset)
        .filter(|candidate| candidate.applies_to(subject.kind))
        .unwrap_or_else(|| default_preset(subject.kind));
    let shot_label = input
        .shot
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or(chosen.label);
    let shot = Shot {
        label: shot_label,
        weight: input.shot.weight.unwrap_or(1.0).clamp(0.0, 1.0),
    };
    let slider_values: Vec<(Id, f32)> = input
        .sliders
        .iter()
        .map(|slider| {
            (slider.node_id, if slider.muted { 0.0 } else { slider.value })
        })
        .collect();
    let muted_nodes: HashSet<Id> = input
        .sliders
        .iter()
        .filter(|slider| slider.muted)
        .map(|slider| slider.node_id)
        .collect();
    let world = World::new(input.nodes.iter());
    let stack = resolve(&world, input.subject_id, Some(shot)).ok_or_else(|| {
        WobuError::new(Code::NoSuchNode, "That entity is not in this project any more.")
    })?;
    let requested_aspect = input.aspect.as_deref().unwrap_or(chosen.aspect);
    let requested_aspect = AspectRatio::parse(requested_aspect).ok_or_else(|| {
        WobuError::new(Code::Invalid, "That is not a supported aspect ratio.")
            .with_detail(requested_aspect.to_owned())
    })?;
    let caps = input.backend.capabilities(&input.model);
    let assets: HashMap<Id, &Asset> = input.assets.iter().map(|asset| (asset.id, asset)).collect();
    let user_prompt = input.shot.prompt.as_deref().map(str::trim).unwrap_or("");
    let available_nodes: HashSet<Id> =
        stack.sources().iter().filter_map(|source| source.node_id()).collect();
    let cells = variant_cells(
        subject,
        *chosen,
        requested_aspect,
        input.seed,
        input.seed_source,
        input.locked_seed,
        &slider_values,
        &available_nodes,
        input.grid.as_ref(),
        &caps,
    )?;
    let batch_size = cells.len();
    let mut plans = Vec::with_capacity(batch_size);
    for (batch_index, cell) in cells.into_iter().enumerate() {
        let sliders = Sliders::from_pairs(cell.slider_values.iter().copied());
        let mut extracted = match cell.item.view {
            Some(view) => fragments_for_view(&stack, &cell.preset, &sliders, view),
            None => fragments(&stack, &cell.preset, &sliders),
        };
        append_user_prompt(&stack, &mut extracted, user_prompt);
        let negotiated = negotiate(&extracted, cell.aspect, &caps);
        let compiled = compile(negotiated.fragments(), Budget::unlimited());
        let mut references = Vec::new();
        for bucket in negotiated.images().buckets() {
            for fragment in bucket.kept() {
                let asset_id = fragment.asset_id().expect("kept image fragments have asset ids");
                let asset = assets.get(&asset_id).ok_or_else(|| {
                    WobuError::new(
                        Code::NoSuchAsset,
                        "A reference image is no longer in this project.",
                    )
                    .with_detail(asset_id.to_string())
                })?;
                let bytes = std::fs::read(input.root.join(&asset.rel_path)).map_err(|error| {
                    WobuError::new(Code::Io, "A reference image could not be read.")
                        .with_detail(error.to_string())
                })?;
                if let Some(reference) =
                    Reference::from_fragment(*fragment, bucket.bucket(), bytes, asset.mime.clone())
                {
                    references.push(reference);
                }
            }
        }
        let dropped: Vec<FragmentKey> = compiled
            .dropped()
            .iter()
            .map(|drop| FragmentKey::of(drop.fragment))
            .chain(negotiated.images().dropped().map(|drop| FragmentKey::of(drop.fragment)))
            .chain(negotiated.downgrades().iter().map(|drop| FragmentKey::of(drop.fragment)))
            .collect();
        let resolution = negotiated.resolution();
        let mut params = Map::new();
        params.insert("batchIndex".into(), json!(batch_index));
        params.insert("batchSize".into(), json!(batch_size));
        params.insert("requestedAspect".into(), json!(cell.aspect.to_string()));
        params.insert("aspect".into(), json!(negotiated.aspect().to_string()));
        params.insert("width".into(), json!(resolution.width));
        params.insert("height".into(), json!(resolution.height));
        params.insert("seedSource".into(), json!(cell.seed_source));
        if let Some(locked_seed) = input.locked_seed {
            params.insert("lockedSeed".into(), json!(locked_seed));
            params.insert("usedLockedSeed".into(), json!(cell.item.seed == locked_seed));
        }
        if let Some(variation) = &cell.variation {
            params.insert("variation".into(), serde_json::to_value(variation).map_err(|error| {
                WobuError::new(Code::Internal, "The variant grid metadata could not be encoded.")
                    .with_detail(error.to_string())
            })?);
        }
        let price = image_price(&input.provider, &input.model, resolution);
        let cost_usd_micros = price.map_or(0, |price| price.per_image_usd_micros);
        params.insert("estimatedCostUsdMicros".into(), json!(cost_usd_micros));
        if price.is_some() {
            params.insert("pricingCheckedAt".into(), json!(PRICE_CHECKED_AT));
            params.insert("pricingSource".into(), json!(PRICE_SOURCE));
            params.insert("pricingIndicative".into(), json!(true));
            params.insert(
                "pricingConservativeFallback".into(),
                json!(price.is_some_and(|price| price.conservative_fallback)),
            );
        }
        let request = ImageRequest::new(
            input.model.clone(),
            compiled.prompt(),
            cell.item.seed,
            &negotiated,
        )
        .with_negative(compiled.negative())
        .with_references(references);
        plans.push(PlannedImage {
            request,
            cost_usd_micros,
            generation: Generation {
                id: new_id(),
                node_id: input.subject_id,
                created_at: Utc::now(),
                preset: cell.preset.id.to_owned(),
                view_type: cell.item.view.map(|view| view.view_type.to_owned()),
                user_prompt: user_prompt.to_owned(),
                compiled_prompt: compiled.prompt().to_owned(),
                negative_prompt: compiled.negative().to_owned(),
                backend: input.provider.clone(),
                model: input.model.clone(),
                seed: cell.item.seed,
                params,
                output_asset_ids: Vec::new(),
                influence_snapshot: snapshot(
                    &stack,
                    &extracted,
                    &cell.slider_values,
                    &muted_nodes,
                    &dropped,
                ),
            },
        });
    }

    Ok(GenerateTask {
        label: format!("Generate {} ×{}", subject.name, plans.len()),
        subject_id: input.subject_id,
        project_id: input.project_id,
        root: input.root,
        backend: input.backend,
        plans,
        next: 0,
        completed: Vec::new(),
        app: input.app,
        requires_billing: caps.requires_billing,
        reservation: None,
    })
}

fn append_user_prompt<'a>(
    stack: &wobu_influence::ResolvedStack<'a>,
    fragments: &mut Vec<Fragment<'a>>,
    prompt: &'a str,
) {
    if prompt.is_empty() {
        return;
    }
    if let Some(source) = stack.sources().iter().find(|source| source.layer == wobu_core::Layer::Shot)
    {
        fragments.push(Fragment::new(
            source,
            "user_prompt",
            FragmentBody::Text(prompt),
            source.weight,
            FragmentTarget::Prompt,
        ));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FragmentKey {
    node: Option<Id>,
    section: &'static str,
    text: Option<String>,
    asset: Option<Id>,
    target: FragmentTarget,
}

impl FragmentKey {
    fn of(fragment: Fragment<'_>) -> Self {
        Self {
            node: fragment.node_id(),
            section: fragment.section(),
            text: fragment.text().map(str::to_owned),
            asset: fragment.asset_id(),
            target: fragment.target(),
        }
    }
}

fn snapshot(
    stack: &wobu_influence::ResolvedStack<'_>,
    fragments: &[Fragment<'_>],
    sliders: &[(Id, f32)],
    muted_nodes: &HashSet<Id>,
    dropped: &[FragmentKey],
) -> InfluenceSnapshot {
    let values: HashMap<Id, f32> = sliders.iter().copied().collect();
    let mut remaining_drops = dropped.to_vec();
    InfluenceSnapshot {
        layers: stack
            .sources()
            .iter()
            .map(|source| {
                let node_id = source.node_id();
                let slider = node_id.and_then(|id| values.get(&id).copied()).unwrap_or(1.0);
                SnapshotLayer {
                    layer: source.layer,
                    node_id,
                    node_name: source.name().to_owned(),
                    weight: source.weight * slider,
                    muted: node_id.is_some_and(|id| muted_nodes.contains(&id))
                        || (node_id.is_none() && source.weight <= 0.0),
                    fragments: fragments
                        .iter()
                        .copied()
                        .filter(|fragment| {
                            fragment.layer() == source.layer && fragment.node_id() == node_id
                        })
                        .map(|fragment| {
                            let key = FragmentKey::of(fragment);
                            let reported = remaining_drops
                                .iter()
                                .position(|candidate| candidate == &key)
                                .map(|index| {
                                    remaining_drops.remove(index);
                                })
                                .is_some();
                            SnapshotFragment {
                                section: fragment.section().to_owned(),
                                text: fragment.text().map(str::to_owned),
                                asset_id: fragment.asset_id(),
                                weight: fragment.weight(),
                                target: fragment.target(),
                                dropped: reported || !fragment.contributes(),
                            }
                        })
                        .collect(),
                }
            })
            .collect(),
    }
}

struct GenerateTask {
    label: String,
    subject_id: Id,
    project_id: Id,
    root: PathBuf,
    backend: Arc<dyn ImageBackend>,
    plans: Vec<PlannedImage>,
    next: usize,
    completed: Vec<GenerateReady>,
    app: AppHandle,
    requires_billing: bool,
    reservation: Option<SpendReservation>,
}

struct PlannedImage {
    request: ImageRequest,
    cost_usd_micros: u64,
    generation: Generation,
}

impl GenerateTask {
    fn reserve_spend(&mut self) -> CommandResult<()> {
        let total = self.plans.iter().try_fold(0_u64, |total, plan| {
            total.checked_add(plan.cost_usd_micros).ok_or_else(|| {
                WobuError::new(Code::Invalid, "The estimated batch cost is too large.")
            })
        })?;
        if total > 0 {
            self.reservation = Some(SpendReservation::create(&self.root, total)?);
        }
        Ok(())
    }

    fn commit_spend(&mut self, cost_usd_micros: u64) -> CommandResult<()> {
        if cost_usd_micros == 0 {
            return Ok(());
        }
        let reservation = self.reservation.as_mut().ok_or_else(|| {
            WobuError::new(
                Code::Internal,
                "Paid generation started without a spend reservation.",
            )
        })?;
        if let Err(error) = reservation.commit(cost_usd_micros) {
            // A receipt was already persisted. Retaining the remaining
            // reservation fails closed until the project can be inspected.
            reservation.release_on_drop = false;
            return Err(error);
        }
        Ok(())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateReady {
    subject_id: Id,
    generation: Generation,
    asset: Asset,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateBatchReady {
    subject_id: Id,
    generations: Vec<Generation>,
    assets: Vec<Asset>,
}

#[async_trait]
impl Task for GenerateTask {
    fn kind(&self) -> JobKind {
        JobKind::Generate
    }

    fn subject_id(&self) -> Option<String> {
        Some(self.subject_id.to_string())
    }

    fn label(&self) -> String {
        self.label.clone()
    }

    async fn run(&mut self, ctx: &JobContext) -> Outcome {
        while self.next < self.plans.len() {
            let batch_index = self.next;
            let batch_total = self.plans.len();
            let cost_usd_micros = self.plans[batch_index].cost_usd_micros;
            let base_generation = self.plans[batch_index].generation.clone();
            let mut progress = JobProgress { ctx: ctx.clone(), batch_index, batch_total };
            let outcome = self
                .backend
                .generate(&self.plans[batch_index].request, &mut progress, ctx.cancel())
                .await;
            let image = match outcome.result {
                Ok(image) => image,
                Err(error) if outcome.usage.is_billed() => {
                    let mut generation = base_generation.clone();
                    generation.created_at = Utc::now();
                    generation.params.insert("outcome".into(), json!("failed"));
                    generation.params.insert("errorCode".into(), json!(error.code()));
                    let root = self.root.clone();
                    let project_id = self.project_id;
                    let recorded = tauri::async_runtime::spawn_blocking(move || {
                        let mut project = Project::open(&root)?;
                        if project.id() != project_id {
                            return Err(WobuError::new(
                                Code::Invalid,
                                "The project at this location changed while the image was generating.",
                            ));
                        }
                        project.record_generation(generation).map_err(WobuError::from)
                    })
                    .await;
                    match recorded {
                        Ok(Ok(generation)) => {
                            if let Err(save_error) = self.commit_spend(cost_usd_micros) {
                                return Outcome::failed(command_failure(save_error));
                            }
                            let _ = self.app.emit(
                                GENERATION_RECORDED,
                                json!({
                                    "subjectId": self.subject_id,
                                    "generation": generation,
                                    "asset": null,
                                }),
                            );
                        }
                        Ok(Err(save_error)) => {
                            if let Some(reservation) = self.reservation.as_mut() {
                                reservation.release_on_drop = false;
                            }
                            return Outcome::failed(command_failure(save_error));
                        }
                        Err(join_error) => {
                            if let Some(reservation) = self.reservation.as_mut() {
                                reservation.release_on_drop = false;
                            }
                            return Outcome::failed(
                                Failure::new(
                                    "internal",
                                    "The billed generation receipt could not be saved.",
                                )
                                .with_detail(join_error.to_string())
                                .billed(Billed::Charged),
                            );
                        }
                    }
                    if matches!(error, ImageError::Cancelled) {
                        return Outcome::Cancelled;
                    }
                    return Outcome::failed(image_failure(
                        &error,
                        outcome.usage,
                        self.requires_billing,
                    ));
                }
                Err(ImageError::Cancelled) => return Outcome::Cancelled,
                Err(error) => {
                    if billing_may_be_unknown(&error, outcome.usage, self.requires_billing) {
                        if let Some(reservation) = self.reservation.as_mut() {
                            reservation.release_on_drop = false;
                        }
                    }
                    return Outcome::failed(image_failure(
                        &error,
                        outcome.usage,
                        self.requires_billing,
                    ));
                }
            };

            // Persist a completed (and possibly paid) image even if cancellation
            // raced the final response. Cancellation stops before the next image.
            let root = self.root.clone();
            let project_id = self.project_id;
            let subject_id = self.subject_id;
            let bytes = image.bytes;
            let mut generation = base_generation;
            generation.created_at = Utc::now();
            generation.seed = image.seed.unwrap_or(generation.seed);
            if let Some(locked_seed) = generation.params.get("lockedSeed").and_then(Value::as_u64) {
                generation
                    .params
                    .insert("usedLockedSeed".into(), json!(generation.seed == locked_seed));
            }
            generation.params.insert("outcome".into(), json!("done"));
            let saved =
                tauri::async_runtime::spawn_blocking(move || -> CommandResult<GenerateReady> {
                    let mut project = Project::open(&root)?;
                    if project.id() != project_id {
                        return Err(WobuError::new(
                            Code::Invalid,
                            "The project at this location changed while the image was generating.",
                        ));
                    }
                    let imported = project.import_asset(&bytes, AssetKind::Generated)?;
                    generation.output_asset_ids = vec![imported.asset.id];
                    let generation = project.record_generation(generation)?;
                    Ok(GenerateReady { subject_id, generation, asset: imported.asset })
                })
                .await;
            match saved {
                Ok(Ok(ready)) => {
                    if let Err(error) = self.commit_spend(cost_usd_micros) {
                        return Outcome::failed(command_failure(error));
                    }
                    // Per-image, not merely at batch completion: a later failure
                    // or cancellation must not hide work already persisted.
                    let _ = self.app.emit(GENERATION_RECORDED, &ready);
                    self.completed.push(ready);
                    self.next += 1;
                }
                Ok(Err(error)) => {
                    if let Some(reservation) = self.reservation.as_mut() {
                        reservation.release_on_drop = false;
                    }
                    return Outcome::failed(command_failure(error));
                }
                Err(error) => {
                    if let Some(reservation) = self.reservation.as_mut() {
                        reservation.release_on_drop = false;
                    }
                    return Outcome::failed(
                        Failure::new("internal", "The generated image could not be saved.")
                            .with_detail(error.to_string())
                            .billed(if outcome.usage.is_billed() {
                                Billed::Charged
                            } else {
                                Billed::Unknown
                            }),
                    );
                }
            }
            if ctx.is_cancelled() {
                return Outcome::Cancelled;
            }
        }
        Outcome::done_with(GenerateBatchReady {
            subject_id: self.subject_id,
            generations: self.completed.iter().map(|ready| ready.generation.clone()).collect(),
            assets: self.completed.iter().map(|ready| ready.asset.clone()).collect(),
        })
    }
}

struct JobProgress {
    ctx: JobContext,
    batch_index: usize,
    batch_total: usize,
}

impl ProgressSink for JobProgress {
    fn step(&mut self, done: u32, total: u32, note: Option<&str>) {
        let within = if total == 0 { 100 } else { done.min(total) * 100 / total };
        let overall = self.batch_index as u32 * 100 + within;
        let prefix = format!("Image {}/{}", self.batch_index + 1, self.batch_total);
        let note = note.map_or(prefix.clone(), |note| format!("{prefix} · {note}"));
        self.ctx.progress(
            Progress::new(overall, self.batch_total as u32 * 100).with_note(note),
        );
    }

    fn preview(&mut self, image: &str, step: Option<u32>) {
        let preview = step
            // Backends restart their step counter for each image. Reserve a
            // wide monotonic range per batch entry so image two's first preview
            // cannot be rejected as older than image one's last.
            .map(|step| {
                Preview::new(image).at_step(
                    (self.batch_index as u32).saturating_mul(1_000_000).saturating_add(step),
                )
            })
            .unwrap_or_else(|| Preview::new(image));
        self.ctx.preview(preview);
    }
}

fn image_failure(error: &ImageError, usage: ImageUsage, requires_billing: bool) -> Failure {
    let billed = if usage.is_billed() {
        Billed::Charged
    } else {
        match error {
            ImageError::Refused { .. }
            | ImageError::NoImage
            | ImageError::NotAnImage { .. }
            | ImageError::NoMesh
            | ImageError::NotAMesh { .. }
                if requires_billing => Billed::Unknown,
            _ => Billed::Nothing,
        }
    };
    let mut failure = Failure::new(error.code(), error.to_string())
        .retryable(error.is_retryable())
        .billed(billed);
    if let ImageError::RateLimited { retry_after: Some(wait), .. } = error {
        failure = failure.after(*wait);
    }
    failure
}

fn billing_may_be_unknown(
    error: &ImageError,
    usage: ImageUsage,
    requires_billing: bool,
) -> bool {
    !usage.is_billed()
        && requires_billing
        && matches!(
            error,
            ImageError::Refused { .. }
                | ImageError::NoImage
                | ImageError::NotAnImage { .. }
                | ImageError::NoMesh
                | ImageError::NotAMesh { .. }
        )
}

fn command_failure(error: WobuError) -> Failure {
    let mut failure = Failure::new(error.code.as_str(), error.message).retryable(error.retryable);
    if let Some(detail) = error.detail {
        failure = failure.with_detail(detail);
    }
    failure.billed(Billed::Unknown)
}

fn image_price(provider: &str, model: &str, resolution: Resolution) -> Option<Price> {
    if provider != gemini::ID {
        return None;
    }
    let longest = resolution.width.max(resolution.height);
    let (per_image_usd_micros, conservative_fallback) = match model {
        "gemini-3.1-flash-image" if longest <= 512 => (45_000, false),
        "gemini-3.1-flash-image" if longest <= 1_024 => (67_000, false),
        "gemini-3.1-flash-image" if longest <= 2_048 => (101_000, false),
        "gemini-3.1-flash-image" => (151_000, false),
        "gemini-3.1-flash-lite-image" => (33_600, false),
        "gemini-3-pro-image" if longest <= 2_048 => (134_000, false),
        "gemini-3-pro-image" => (240_000, false),
        "gemini-2.5-flash-image" => (39_000, false),
        // A newly selected paid Gemini model must not silently bypass the
        // ceiling before its price is added. Use the highest current known
        // synchronous image price and say that the estimate is conservative.
        _ => (240_000, true),
    };
    Some(Price { per_image_usd_micros, conservative_fallback })
}

fn cost_estimate(
    provider: &str,
    model: &str,
    resolution: Resolution,
    images: usize,
) -> Option<CostEstimate> {
    let price = image_price(provider, model, resolution)?;
    Some(CostEstimate {
        currency: "USD",
        per_image_usd_micros: price.per_image_usd_micros,
        batch_usd_micros: price.per_image_usd_micros.saturating_mul(images as u64),
        images,
        varies_by_cell: false,
        indicative: true,
        conservative_fallback: price.conservative_fallback,
        checked_at: PRICE_CHECKED_AT,
        source_url: PRICE_SOURCE,
    })
}

fn cost_estimate_prices(prices: Vec<Price>, images: usize) -> Option<CostEstimate> {
    let first = *prices.first()?;
    let batch_usd_micros = prices
        .iter()
        .fold(0_u64, |total, price| total.saturating_add(price.per_image_usd_micros));
    let varies_by_cell = prices
        .iter()
        .any(|price| price.per_image_usd_micros != first.per_image_usd_micros);
    Some(CostEstimate {
        currency: "USD",
        per_image_usd_micros: first.per_image_usd_micros,
        batch_usd_micros,
        images,
        varies_by_cell,
        indicative: true,
        conservative_fallback: prices.iter().any(|price| price.conservative_fallback),
        checked_at: PRICE_CHECKED_AT,
        source_url: PRICE_SOURCE,
    })
}

fn receipt_cost(generation: &Generation) -> u64 {
    if generation.backend != gemini::ID {
        return 0;
    }
    if let Some(cost) = generation.params.get("estimatedCostUsdMicros").and_then(Value::as_u64) {
        return cost;
    }
    let width = generation
        .params
        .get("width")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(1_024);
    let height = generation
        .params
        .get("height")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(1_024);
    image_price(&generation.backend, &generation.model, Resolution::new(width, height))
        .map_or(240_000, |price| price.per_image_usd_micros)
}

struct SpendLock {
    path: PathBuf,
}

impl SpendLock {
    fn acquire(root: &Path) -> CommandResult<SpendLock> {
        let dir = root.join(SPEND_DIR);
        std::fs::create_dir_all(dir.join("reservations"))
            .map_err(|error| spend_io("The spend ledger could not be prepared.", &dir, error))?;
        let path = dir.join("lock");
        for _ in 0..LOCK_ATTEMPTS {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(file, "{}", Utc::now().to_rfc3339()).map_err(|error| {
                        spend_io("The spend ledger lock could not be written.", &path, error)
                    })?;
                    return Ok(SpendLock { path });
                }
                // Never steal by age. A legitimate critical section can be
                // arbitrarily slow on a network share; age is not ownership.
                // Crash recovery is an explicit, user-confirmed archive path.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => {
                    return Err(spend_io(
                        "The spend ledger could not be locked.",
                        &path,
                        error,
                    ));
                }
            }
        }
        Err(WobuError::new(
            Code::Io,
            "The shared spend ledger is busy. Try Generate again.",
        ))
    }
}

impl Drop for SpendLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug)]
struct SpendReservation {
    root: PathBuf,
    path: PathBuf,
    file: ReservationFile,
    release_on_drop: bool,
}

impl SpendReservation {
    fn create(root: &Path, amount_usd_micros: u64) -> CommandResult<SpendReservation> {
        let _guard = SpendLock::acquire(root)?;
        let status = read_spend_status_locked(root)?;
        let ceiling = status.ceiling_usd_micros.ok_or_else(|| {
            WobuError::new(
                Code::SpendCeilingExceeded,
                "Paid generation is disabled for this project. Set a spend ceiling first.",
            )
        })?;
        let committed = status
            .spent_usd_micros
            .checked_add(status.reserved_usd_micros)
            .and_then(|used| used.checked_add(amount_usd_micros))
            .ok_or_else(|| WobuError::new(Code::Invalid, "The project spend total is too large."))?;
        if committed > ceiling {
            return Err(WobuError::new(
                Code::SpendCeilingExceeded,
                "This batch would cross the project's shared spend ceiling.",
            )
            .with_detail(format!(
                "spent={} reserved={} batch={} ceiling={} USD micros",
                status.spent_usd_micros,
                status.reserved_usd_micros,
                amount_usd_micros,
                ceiling,
            )));
        }
        let id = new_id();
        let path = root.join(SPEND_DIR).join("reservations").join(format!("{id}.json"));
        let file = ReservationFile {
            id,
            remaining_usd_micros: amount_usd_micros,
            created_at: Utc::now().to_rfc3339(),
        };
        write_reservation_new(&path, &file)?;
        Ok(SpendReservation {
            root: root.to_path_buf(),
            path,
            file,
            release_on_drop: true,
        })
    }

    fn commit(&mut self, amount_usd_micros: u64) -> CommandResult<()> {
        if amount_usd_micros > self.file.remaining_usd_micros {
            return Err(WobuError::new(
                Code::Internal,
                "A generation cost exceeded its spend reservation.",
            ));
        }
        let _guard = SpendLock::acquire(&self.root)?;
        let remaining = self.file.remaining_usd_micros - amount_usd_micros;
        if remaining > 0 {
            // Reservations are write-once. Publishing the replacement before
            // removing the old file means a crash can only over-reserve, never
            // open a window where concurrent work can overspend.
            let id = new_id();
            let path = self
                .root
                .join(SPEND_DIR)
                .join("reservations")
                .join(format!("{id}.json"));
            let replacement = ReservationFile {
                id,
                remaining_usd_micros: remaining,
                created_at: self.file.created_at.clone(),
            };
            write_reservation_new(&path, &replacement)?;
            std::fs::remove_file(&self.path).map_err(|error| {
                spend_io("The previous spend reservation could not be retired.", &self.path, error)
            })?;
            self.path = path;
            self.file = replacement;
        } else {
            std::fs::remove_file(&self.path).map_err(|error| {
                spend_io("The completed spend reservation could not be retired.", &self.path, error)
            })?;
            self.file.remaining_usd_micros = 0;
        }
        Ok(())
    }
}

impl Drop for SpendReservation {
    fn drop(&mut self) {
        if !self.release_on_drop {
            return;
        }
        if let Ok(_guard) = SpendLock::acquire(&self.root) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn spend_status_for(root: &Path) -> CommandResult<SpendStatus> {
    let _guard = SpendLock::acquire(root)?;
    read_spend_status_locked(root)
}

fn spend_status_for_report(root: &Path) -> CommandResult<SpendStatus> {
    match SpendLock::acquire(root) {
        Ok(_guard) => read_spend_status_locked(root),
        Err(_error) if root.join(SPEND_DIR).join("lock").exists() => {
            // Display-only fallback. Admission never uses this snapshot: it
            // still requires the exclusive lock. This lets the Inspector
            // explain and recover a crash-orphaned lock instead of replacing
            // the whole cost report with an opaque busy error.
            let mut status = read_spend_status_locked(root)?;
            status.ledger_locked = true;
            Ok(status)
        }
        Err(error) => Err(error),
    }
}

fn read_spend_status_locked(root: &Path) -> CommandResult<SpendStatus> {
    let (ceiling_usd_micros, receipts) = Project::spend_ledger(root)?;
    let spent_usd_micros = receipts.into_iter().try_fold(
        0_u64,
        |total, generation| {
            total.checked_add(receipt_cost(&generation)).ok_or_else(|| {
                WobuError::new(Code::Invalid, "The project spend total is too large.")
            })
        },
    )?;
    let reservations = root.join(SPEND_DIR).join("reservations");
    let mut reserved_usd_micros = 0_u64;
    let mut pending_reservations = 0_usize;
    let mut oldest_reservation_at: Option<String> = None;
    for entry in std::fs::read_dir(&reservations)
        .map_err(|error| spend_io("The spend reservations could not be read.", &reservations, error))?
    {
        let entry = entry.map_err(|error| {
            spend_io("A spend reservation could not be read.", &reservations, error)
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| spend_io("A spend reservation could not be read.", &path, error))?;
        let reservation: ReservationFile = serde_json::from_slice(&bytes).map_err(|error| {
            WobuError::new(
                Code::Malformed,
                "A spend reservation is malformed; paid generation is stopped safely.",
            )
            .with_detail(format!("{}: {error}", path.display()))
        })?;
        reserved_usd_micros = reserved_usd_micros
            .checked_add(reservation.remaining_usd_micros)
            .ok_or_else(|| WobuError::new(Code::Invalid, "The reserved spend total is too large."))?;
        pending_reservations += 1;
        if oldest_reservation_at
            .as_ref()
            .is_none_or(|oldest| reservation.created_at.as_str() < oldest.as_str())
        {
            oldest_reservation_at = Some(reservation.created_at);
        }
    }
    let used = spent_usd_micros.saturating_add(reserved_usd_micros);
    Ok(SpendStatus {
        ceiling_usd_micros,
        spent_usd_micros,
        reserved_usd_micros,
        remaining_usd_micros: ceiling_usd_micros.map(|ceiling| ceiling.saturating_sub(used)),
        pending_reservations,
        oldest_reservation_at,
        ledger_locked: false,
    })
}

fn write_reservation_new(path: &Path, reservation: &ReservationFile) -> CommandResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| spend_io("A spend reservation could not be created.", path, error))?;
    serde_json::to_writer_pretty(&mut file, reservation).map_err(|error| {
        WobuError::new(Code::Internal, "A spend reservation could not be encoded.")
            .with_detail(error.to_string())
    })?;
    file.flush()
        .map_err(|error| spend_io("A spend reservation could not be written.", path, error))?;
    file.sync_all()
        .map_err(|error| spend_io("A spend reservation could not be secured.", path, error))
}

fn spend_io(message: &str, path: &Path, error: std::io::Error) -> WobuError {
    WobuError::new(Code::Io, message).with_detail(format!("{}: {error}", path.display()))
}

fn no_image_provider() -> WobuError {
    WobuError::new(
        Code::Invalid,
        "Choose an image provider in Settings before generating.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestProject {
        parent: PathBuf,
        root: PathBuf,
        node_id: Id,
    }

    impl TestProject {
        fn new(ceiling_usd_micros: u64) -> TestProject {
            let parent = std::env::temp_dir().join(format!("wobu-spend-test-{}", new_id()));
            std::fs::create_dir(&parent).unwrap();
            let mut project = Project::create(&parent, "Ledger").unwrap();
            project.set_spend_ceiling(Some(ceiling_usd_micros)).unwrap();
            let node_id = project
                .create_node(wobu_core::NodeKind::Character, "Kael", None)
                .unwrap()
                .id;
            let root = project.root().to_path_buf();
            drop(project);
            TestProject { parent, root, node_id }
        }
    }

    impl Drop for TestProject {
        fn drop(&mut self) {
            // The parent is a unique path minted by this test, never a caller
            // supplied directory or a workspace root.
            let _ = std::fs::remove_dir_all(&self.parent);
        }
    }

    fn receipt(
        node_id: Id,
        backend: &str,
        model: &str,
        width: u32,
        height: u32,
    ) -> Generation {
        Generation {
            id: new_id(),
            node_id,
            created_at: Utc::now(),
            preset: "portrait".into(),
            view_type: None,
            user_prompt: String::new(),
            compiled_prompt: "portrait".into(),
            negative_prompt: String::new(),
            backend: backend.into(),
            model: model.into(),
            seed: 1,
            params: Map::from_iter([
                ("width".into(), json!(width)),
                ("height".into(), json!(height)),
            ]),
            output_asset_ids: Vec::new(),
            influence_snapshot: InfluenceSnapshot { layers: Vec::new() },
        }
    }

    #[test]
    fn google_standard_output_prices_are_exact_usd_micros() {
        let cases = [
            ("gemini-3.1-flash-lite-image", 1_024, 33_600),
            ("gemini-3.1-flash-image", 512, 45_000),
            ("gemini-3.1-flash-image", 1_024, 67_000),
            ("gemini-3.1-flash-image", 2_048, 101_000),
            ("gemini-3.1-flash-image", 4_096, 151_000),
            ("gemini-3-pro-image", 1_024, 134_000),
            ("gemini-3-pro-image", 4_096, 240_000),
            ("gemini-2.5-flash-image", 1_024, 39_000),
        ];
        for (model, side, expected) in cases {
            assert_eq!(
                image_price(gemini::ID, model, Resolution::new(side, side))
                    .unwrap()
                    .per_image_usd_micros,
                expected,
                "{model} at {side}px"
            );
        }
    }

    #[test]
    fn local_is_free_and_unknown_paid_models_fail_high() {
        assert!(image_price(comfy::ID, "anything", Resolution::new(4_096, 4_096)).is_none());
        let unknown = image_price(gemini::ID, "gemini-future-image", Resolution::new(1_024, 1_024))
            .unwrap();
        assert_eq!(unknown.per_image_usd_micros, 240_000);
        assert!(unknown.conservative_fallback);
    }

    #[test]
    fn batch_estimate_and_old_receipts_use_recorded_model_and_size() {
        let estimate = cost_estimate(
            gemini::ID,
            "gemini-3.1-flash-image",
            Resolution::new(2_048, 2_048),
            8,
        )
        .unwrap();
        assert_eq!(estimate.batch_usd_micros, 808_000);

        let old = receipt(new_id(), gemini::ID, "gemini-3-pro-image", 4_096, 4_096);
        assert_eq!(receipt_cost(&old), 240_000);
        let local = receipt(new_id(), comfy::ID, "local", 4_096, 4_096);
        assert_eq!(receipt_cost(&local), 0);
        let mut explicit = old;
        explicit.params.insert("estimatedCostUsdMicros".into(), json!(123_456));
        assert_eq!(receipt_cost(&explicit), 123_456);
    }

    #[test]
    fn variant_cells_change_one_axis_and_report_the_real_output_count() {
        let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
        let chosen = *preset("character_sheet").unwrap();
        let caps = GeminiBackend::new("test-key")
            .unwrap()
            .capabilities("gemini-3.1-flash-image");
        let available = HashSet::from([subject.id]);
        let weight_grid = VariantGrid::FragmentWeight {
            node_id: subject.id,
            values: vec![0.4, 0.7, 1.0],
        };
        let weights = variant_cells(
            &subject,
            chosen,
            AspectRatio::parse("3:4").unwrap(),
            42,
            SeedSource::Locked,
            Some(42),
            &[],
            &available,
            Some(&weight_grid),
            &caps,
        )
        .unwrap();
        assert_eq!(weights.len(), 3);
        assert!(weights.iter().all(|cell| cell.item.seed == 42));
        assert_eq!(
            weights
                .iter()
                .map(|cell| cell.slider_values[0].1)
                .collect::<Vec<_>>(),
            [0.4, 0.7, 1.0]
        );
        assert!(weights.iter().all(|cell| {
            matches!(
                cell.variation.as_ref().map(|variation| &variation.value),
                Some(VariationValue::FragmentWeight { .. })
            )
        }));

        let seed_grid = VariantGrid::Seed { values: vec![11, 22, 33, 44, 55] };
        let seeds = variant_cells(
            &subject,
            chosen,
            AspectRatio::parse("3:4").unwrap(),
            42,
            SeedSource::Locked,
            Some(42),
            &[],
            &available,
            Some(&seed_grid),
            &caps,
        )
        .unwrap();
        assert_eq!(seeds.iter().map(|cell| cell.item.seed).collect::<Vec<_>>(), [11, 22, 33, 44, 55]);

        let estimate = cost_estimate_prices(
            seeds
                .iter()
                .map(|_| Price { per_image_usd_micros: 67_000, conservative_fallback: false })
                .collect(),
            seeds.len(),
        )
        .unwrap();
        assert_eq!(estimate.images, 5);
        assert_eq!(estimate.batch_usd_micros, 335_000);
    }

    #[test]
    fn named_view_presets_are_refused_as_variant_grids() {
        let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
        let caps = GeminiBackend::new("test-key")
            .unwrap()
            .capabilities("gemini-3.1-flash-image");
        let grid = VariantGrid::Seed { values: vec![1, 2] };
        let error = variant_cells(
            &subject,
            *preset("turnaround").unwrap(),
            AspectRatio::parse("1:1").unwrap(),
            1,
            SeedSource::Locked,
            Some(1),
            &[],
            &HashSet::from([subject.id]),
            Some(&grid),
            &caps,
        )
        .unwrap_err();
        assert_eq!(error.code, Code::Invalid);
    }

    #[test]
    fn competing_reservations_cannot_consume_the_same_remaining_ceiling() {
        let project = TestProject::new(100_000);
        let first = SpendReservation::create(&project.root, 60_000).unwrap();
        let second = SpendReservation::create(&project.root, 50_000).unwrap_err();
        assert_eq!(second.code, Code::SpendCeilingExceeded);

        let held = spend_status_for(&project.root).unwrap();
        assert_eq!(held.spent_usd_micros, 0);
        assert_eq!(held.reserved_usd_micros, 60_000);
        assert_eq!(held.remaining_usd_micros, Some(40_000));
        drop(first);
        assert_eq!(spend_status_for(&project.root).unwrap().reserved_usd_micros, 0);
    }

    #[test]
    fn committed_receipt_and_reduced_reservation_are_not_double_counted() {
        let project = TestProject::new(200_000);
        let mut reservation = SpendReservation::create(&project.root, 134_000).unwrap();
        let mut generation = receipt(
            project.node_id,
            gemini::ID,
            "gemini-3.1-flash-image",
            1_024,
            1_024,
        );
        generation.params.insert("estimatedCostUsdMicros".into(), json!(67_000));
        let mut store = Project::open(&project.root).unwrap();
        store.record_generation(generation).unwrap();
        drop(store);
        reservation.commit(67_000).unwrap();

        let status = spend_status_for(&project.root).unwrap();
        assert_eq!(status.spent_usd_micros, 67_000);
        assert_eq!(status.reserved_usd_micros, 67_000);
        assert_eq!(status.remaining_usd_micros, Some(66_000));
    }

    #[test]
    fn malformed_canonical_receipt_fails_closed() {
        let project = TestProject::new(200_000);
        let month = project.root.join("generations/2026-08");
        std::fs::create_dir_all(&month).unwrap();
        std::fs::write(month.join(format!("{}.json", new_id())), b"{not-json").unwrap();

        let error = spend_status_for(&project.root).unwrap_err();
        assert_eq!(error.code, Code::Malformed);
        assert!(SpendReservation::create(&project.root, 1).is_err());
    }
}
