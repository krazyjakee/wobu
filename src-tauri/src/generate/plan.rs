//! What a request means before anything is sent.
//!
//! Steps 2 and 3 of the chain in [`super`]. Normalizing the controls, deciding
//! the seed, resolving the influence stack and expanding a variant grid all
//! happen here, so the four surfaces that ask "what would this generate?" agree
//! by construction rather than by four matching implementations.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wobu_core::{
    FragmentTarget, GenerationVariation, Id, InfluenceSnapshot, Node, Preset, PresetGeneration,
    SnapshotFragment, SnapshotLayer, VariationValue, default_preset, new_id, preset,
};
use wobu_imagine::{AspectRatio, Capabilities};
use wobu_influence::{
    Fragment, FragmentBody, Shot, Sliders, World, fragments, fragments_for_view, resolve,
};

use crate::error::{Code, CommandResult, WobuError};

pub(super) const MAX_GRID_CELLS: usize = 16;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "axis", rename_all = "snake_case")]
pub enum VariantGrid {
    Seed {
        values: Vec<u64>,
    },
    FragmentWeight {
        #[serde(rename = "nodeId")]
        node_id: Id,
        values: Vec<f32>,
    },
    Preset {
        values: Vec<String>,
    },
    Aspect {
        values: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SeedSource {
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
    pub(super) node_id: Id,
    pub(super) value: f32,
    #[serde(default)]
    pub(super) muted: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateShot {
    pub(super) label: Option<String>,
    pub(super) weight: Option<f32>,
    pub(super) prompt: Option<String>,
}

#[derive(Clone)]
pub(super) struct GenerationPlanRequest {
    pub(super) subject_id: Id,
    pub(super) preset_id: Option<String>,
    pub(super) sliders: Vec<GenerateSlider>,
    pub(super) shot: GenerateShot,
    pub(super) aspect: Option<String>,
    pub(super) seed: u64,
    pub(super) seed_source: SeedSource,
    pub(super) locked_seed: Option<u64>,
    pub(super) grid: Option<VariantGrid>,
}

/// The controls that are true of the whole batch, after normalization.
///
/// Deliberately *not* the aspect or the slider values: a variant grid varies
/// one of those per cell, so the authoritative copy of each lives on the
/// [`VariantCell`]. Recording a second batch-level copy here is how a report
/// and a receipt end up disagreeing about the same generation.
#[derive(Debug, Clone)]
pub(super) struct NormalizedControls {
    pub(super) shot_label: String,
    pub(super) shot_weight: f32,
    pub(super) user_prompt: String,
    pub(super) muted_nodes: HashSet<Id>,
}

impl NormalizedControls {
    /// `params.controls` for one cell of a batch.
    ///
    /// The slider values are the *cell's*, not the request's: a fragment-weight
    /// grid varies exactly one of them per cell, and a receipt that recorded the
    /// request's would describe a generation that never happened.
    pub(super) fn receipt_controls(&self, cell_sliders: &[(Id, f32)]) -> Value {
        json!({
            "sliders": cell_sliders.iter().map(|(node_id, value)| json!({
                "nodeId": node_id,
                "value": value,
                "muted": self.muted_nodes.contains(node_id),
            })).collect::<Vec<_>>(),
            "shot": {
                "label": self.shot_label,
                "weight": self.shot_weight,
                "prompt": self.user_prompt,
            },
        })
    }
}

/// One request, normalized and expanded into the exact images it means.
///
/// Preview, batch generation and composition all read this and nothing else
/// about what to send; the cells carry everything that differs per image.
#[derive(Debug, Clone)]
pub(super) struct PreparedGenerationPlan {
    pub(super) subject_id: Id,
    pub(super) subject_name: String,
    pub(super) controls: NormalizedControls,
    pub(super) cells: Vec<VariantCell>,
    pub(super) locked_seed: Option<u64>,
}

/// Whether the seed is about to produce images or only an estimate.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SeedIntent {
    Execute,
    /// The reference report. Every other input is the one execution would use,
    /// but a *random* seed here would make two identical previews of the same
    /// controls disagree, so the unseeded case is pinned to zero instead. Only
    /// the seed differs, and nothing the report returns depends on it: the
    /// budget and the price are decided by fragments, aspect and resolution.
    Estimate,
}

/// The one place that decides which seed a request runs on, and what the
/// receipt is allowed to claim about where it came from.
pub(super) fn resolve_seed(
    requested: Option<u64>,
    locked: Option<u64>,
    intent: SeedIntent,
) -> (u64, SeedSource) {
    match (requested, locked) {
        (Some(seed), _) => (seed, SeedSource::Rerolled),
        (None, Some(seed)) => (seed, SeedSource::Locked),
        (None, None) => match intent {
            SeedIntent::Execute => (u128::from(new_id()) as u64, SeedSource::Random),
            SeedIntent::Estimate => (0, SeedSource::Random),
        },
    }
}

/// The entity's shared identity seed, or `None` — including for an id that is
/// not in this world any more, which planning reports by name rather than by
/// this lookup failing quietly.
pub(super) fn locked_seed_of(nodes: &[Node], subject_id: Id) -> Option<u64> {
    nodes.iter().find(|node| node.id == subject_id).and_then(|node| node.locked_seed)
}

pub(super) fn normalize_prompt(prompt: Option<&str>) -> String {
    prompt.map(str::trim).unwrap_or_default().to_owned()
}

pub(super) fn normalize_aspect(
    requested: Option<&str>,
    fallback: &str,
) -> CommandResult<AspectRatio> {
    let requested = requested.map(str::trim).filter(|value| !value.is_empty()).unwrap_or(fallback);
    AspectRatio::parse(requested).ok_or_else(|| {
        WobuError::new(Code::Invalid, "That is not a supported aspect ratio.")
            .with_detail(requested.to_owned())
    })
}

pub(super) fn normalize_weight(weight: Option<f32>) -> f32 {
    weight.unwrap_or(1.0).clamp(0.0, 1.0)
}

fn normalize_sliders(sliders: &[GenerateSlider]) -> (Vec<(Id, f32)>, HashSet<Id>) {
    let values = sliders
        .iter()
        .map(|slider| (slider.node_id, if slider.muted { 0.0 } else { slider.value }))
        .collect();
    let muted = sliders.iter().filter(|slider| slider.muted).map(|slider| slider.node_id).collect();
    (values, muted)
}

pub(super) fn prepare_generation_plan(
    nodes: &[Node],
    request: GenerationPlanRequest,
    caps: &Capabilities,
) -> CommandResult<PreparedGenerationPlan> {
    let subject = nodes.iter().find(|node| node.id == request.subject_id).ok_or_else(|| {
        WobuError::new(Code::NoSuchNode, "That entity is not in this project any more.")
    })?;
    let chosen = request
        .preset_id
        .as_deref()
        .and_then(preset)
        .filter(|candidate| candidate.applies_to(subject.kind))
        .copied()
        .unwrap_or_else(|| *default_preset(subject.kind));
    let shot_label = request
        .shot
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or(chosen.label)
        .to_owned();
    let shot_weight = normalize_weight(request.shot.weight);
    let user_prompt = normalize_prompt(request.shot.prompt.as_deref());
    let requested_aspect = normalize_aspect(request.aspect.as_deref(), chosen.aspect)?;
    let (slider_values, muted_nodes) = normalize_sliders(&request.sliders);
    let world = World::new(nodes.iter());
    let stack =
        resolve(&world, request.subject_id, Some(Shot { label: &shot_label, weight: shot_weight }))
            .ok_or_else(|| {
                WobuError::new(Code::NoSuchNode, "That entity is not in this project any more.")
            })?;
    let available_nodes = stack.sources().iter().filter_map(|source| source.node_id()).collect();
    let cells = variant_cells(
        subject,
        chosen,
        requested_aspect,
        request.seed,
        request.seed_source,
        &slider_values,
        &available_nodes,
        request.grid.as_ref(),
        caps,
    )?;
    Ok(PreparedGenerationPlan {
        subject_id: request.subject_id,
        subject_name: subject.name.clone(),
        controls: NormalizedControls { shot_label, shot_weight, user_prompt, muted_nodes },
        cells,
        locked_seed: request.locked_seed,
    })
}

pub(super) fn resolve_generation_stack<'a>(
    nodes: &'a [Node],
    plan: &'a PreparedGenerationPlan,
) -> CommandResult<wobu_influence::ResolvedStack<'a>> {
    let world = World::new(nodes.iter());
    resolve(
        &world,
        plan.subject_id,
        Some(Shot { label: &plan.controls.shot_label, weight: plan.controls.shot_weight }),
    )
    .ok_or_else(|| WobuError::new(Code::NoSuchNode, "That entity is not in this project any more."))
}

pub(super) fn fragments_for_cell<'a>(
    stack: &wobu_influence::ResolvedStack<'a>,
    cell: &VariantCell,
    user_prompt: &'a str,
) -> Vec<Fragment<'a>> {
    let sliders = Sliders::from_pairs(cell.slider_values.iter().copied());
    let mut extracted = match cell.item.view {
        Some(view) => fragments_for_view(stack, &cell.preset, &sliders, view),
        None => fragments(stack, &cell.preset, &sliders),
    };
    append_user_prompt(stack, &mut extracted, user_prompt);
    extracted
}

#[derive(Debug, Clone)]
pub(super) struct VariantCell {
    pub(super) preset: Preset,
    pub(super) item: PresetGeneration,
    pub(super) aspect: AspectRatio,
    pub(super) slider_values: Vec<(Id, f32)>,
    pub(super) seed_source: SeedSource,
    pub(super) variation: Option<GenerationVariation>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn variant_cells(
    subject: &Node,
    chosen: Preset,
    base_aspect: AspectRatio,
    base_seed: u64,
    base_seed_source: SeedSource,
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
    let common =
        |index: usize, preset: Preset, seed: u64, aspect: AspectRatio, values| VariantCell {
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
        };

    let cells = match grid {
        VariantGrid::Seed { values } => {
            require_distinct(values.iter().copied(), "seed")?;
            values
                .iter()
                .copied()
                .enumerate()
                .map(|(index, seed)| {
                    let mut cell = common(index, chosen, seed, base_aspect, slider_values.to_vec());
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
            require_distinct(distinct, "fragment weight")?;
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
                let mut cell =
                    common(index, *candidate, base_seed, base_aspect, slider_values.to_vec());
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
                let parsed = normalize_aspect(Some(value), chosen.aspect).map_err(|_| {
                    WobuError::new(
                        Code::Invalid,
                        format!("{value} is not a supported aspect ratio."),
                    )
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
                    let mut cell = common(index, chosen, base_seed, aspect, slider_values.to_vec());
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

pub(super) fn append_user_prompt<'a>(
    stack: &wobu_influence::ResolvedStack<'a>,
    fragments: &mut Vec<Fragment<'a>>,
    prompt: &'a str,
) {
    if prompt.is_empty() {
        return;
    }
    if let Some(source) =
        stack.sources().iter().find(|source| source.layer == wobu_core::Layer::Shot)
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
pub(super) struct FragmentKey {
    pub(super) node: Option<Id>,
    pub(super) section: &'static str,
    pub(super) text: Option<String>,
    pub(super) asset: Option<Id>,
    pub(super) target: FragmentTarget,
}

impl FragmentKey {
    pub(super) fn of(fragment: Fragment<'_>) -> Self {
        Self {
            node: fragment.node_id(),
            section: fragment.section(),
            text: fragment.text().map(str::to_owned),
            asset: fragment.asset_id(),
            target: fragment.target(),
        }
    }
}

pub(super) fn snapshot(
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
                                asset_role: fragment.asset_role(),
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
