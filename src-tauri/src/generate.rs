//! Preparing and running one image generation from the Inspector.
//!
//! The command resolves and negotiates before it queues anything, so bad input,
//! a missing provider, a missing key or an unreadable reference fails without
//! starting a paid job. The task owns only an immutable request and a project
//! path; it never holds the open-project mutex across a provider call.
//!
//! # One planning path
//!
//! Four surfaces ask "what would this generate?" — the Inspector's live
//! reference report, a batch, a variant grid, and a scene composition — and
//! they answer with one chain rather than four:
//!
//! 1. [`selected_image_provider`] and [`ProviderChoice::model`] decide the
//!    provider and model. [`image_backend`] builds the adapter, or
//!    [`unprobed_image_backend`] for the one caller that cannot await.
//! 2. [`resolve_seed`] decides the seed and what the receipt may claim about
//!    where it came from.
//! 3. [`prepare_generation_plan`] normalizes the controls and expands the
//!    request into [`VariantCell`]s — one per image, carrying its own preset,
//!    aspect, seed and slider values.
//! 4. [`plan_batch`], [`plan_scene`] and [`reference_report_for_plan`] all read
//!    that plan. The first two produce a [`PlannedBatch`]; the report negotiates
//!    the same first cell the batch would send first, so the numbers on screen
//!    are the numbers that would be spent.
//! 5. [`PlannedBatch::into_task`] is the only route to the queue, so spend
//!    reservation and the billing flag are stated once.
//!
//! Replay deliberately joins at step 5 and nowhere earlier: it re-sends a
//! recorded request rather than compiling a new one. Mesh reconstruction
//! (`commands::mesh`) shares step 1 only — it consumes finished generations, so
//! it has no influence stack, no preset and no aspect to negotiate.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::{self, Write as _};
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
    Asset, AssetKind, AssetRole, FragmentTarget, Generation, GenerationVariation, Id,
    InfluenceSnapshot, Node, Preset, PresetGeneration, SceneComposition, SnapshotFragment,
    SnapshotLayer, VariationValue, default_preset, kind_def, new_id, preset,
};
use wobu_imagine::{
    AspectRatio, Capabilities, Error as ImageError, GeminiBackend, ImageBackend, ImageRequest,
    ImageUsage, LoraWeight, ProgressSink, Reference, ReferenceMechanism, Resolution, comfy, gemini,
    negotiate, negotiate_scene,
};
use wobu_influence::{
    Budget, Fragment, FragmentBody, RefBucket, ResolvedScene, SceneScope, Shot, Sliders, World,
    compile, fragments, fragments_for_view, resolve, resolve_scene, scene_fragments,
};
use wobu_jobs::{Billed, Failure, JobContext, JobKind, Outcome, Preview, Progress, Task};
use wobu_store::Project;

use crate::commands::providers::ProviderChoice;
use crate::error::{Code, CommandResult, WobuError};
use crate::keys::Keys;
use crate::machine::MachineSettings;
use crate::state::{AppState, Jobs};

pub const GENERATION_RECORDED: &str = "generation:recorded";
const PRICE_SOURCE: &str = "https://ai.google.dev/gemini-api/docs/pricing";
const PRICE_CHECKED_AT: &str = "2026-08-01";
const SPEND_DIR: &str = ".wobu/spend";
const SPEND_AGGREGATE: &str = "aggregate.json";
const SPEND_AGGREGATE_VERSION: u32 = 1;
const LOCK_ATTEMPTS: usize = 200;
const LORA_PROTOCOL: u32 = 1;
/// Composition has no preset of its own to take a default aspect from, and the
/// `environment_matte` framing its receipt records is a wide establishing shot.
const SCENE_ASPECT: &str = "16:9";

/// Batch-local immutable reference data. The loader is deliberately
/// single-threaded: it runs on Tauri's blocking pool, bounds filesystem
/// concurrency at one, and retains at most one buffer for every asset id the
/// batch actually keeps.
struct ReferenceLoader<R = fn(&Path) -> io::Result<Vec<u8>>> {
    read: R,
    loaded: HashMap<Id, Arc<[u8]>>,
}

impl ReferenceLoader {
    fn new() -> Self {
        Self { read: read_reference, loaded: HashMap::new() }
    }
}

fn read_reference(path: &Path) -> io::Result<Vec<u8>> {
    std::fs::read(path)
}

impl<R> ReferenceLoader<R>
where
    R: FnMut(&Path) -> io::Result<Vec<u8>>,
{
    #[cfg(test)]
    fn with_reader(read: R) -> Self {
        Self { read, loaded: HashMap::new() }
    }

    fn load(&mut self, asset_id: Id, path: &Path) -> io::Result<Arc<[u8]>> {
        if let Some(bytes) = self.loaded.get(&asset_id) {
            return Ok(Arc::clone(bytes));
        }
        let bytes: Arc<[u8]> = (self.read)(path)?.into();
        self.loaded.insert(asset_id, Arc::clone(&bytes));
        Ok(bytes)
    }
}

/// What to call the images in an error, which is the only thing the two
/// reference-loading callers disagree about.
#[derive(Clone, Copy)]
enum ReferenceScope {
    Image,
    Scene,
}

impl ReferenceScope {
    fn missing(self, asset_id: Id) -> WobuError {
        let message = match self {
            ReferenceScope::Image => "A reference image is no longer in this project.",
            ReferenceScope::Scene => "A scene reference is no longer in this project.",
        };
        WobuError::new(Code::NoSuchAsset, message).with_detail(asset_id.to_string())
    }

    fn unreadable(self, error: io::Error) -> WobuError {
        let message = match self {
            ReferenceScope::Image => "A reference image could not be read.",
            ReferenceScope::Scene => "A scene reference image could not be read.",
        };
        WobuError::new(Code::Io, message).with_detail(error.to_string())
    }
}

/// The bytes for every reference negotiation kept, in the order the provider
/// will receive them.
///
/// One loader for batch generation and composition. That order is not
/// incidental: it is what `referenceAssetIds` records and what replay later
/// restores verbatim, so it cannot be allowed to differ between the two paths
/// by accident.
fn load_references(
    negotiated: &wobu_imagine::Negotiated<'_>,
    assets: &HashMap<Id, &Asset>,
    root: &Path,
    loader: &mut ReferenceLoader,
    scope: ReferenceScope,
) -> CommandResult<Vec<Reference>> {
    let mut references = Vec::new();
    for bucket in negotiated.images().buckets() {
        for fragment in bucket.kept() {
            let asset_id = fragment.asset_id().expect("kept image fragments have asset ids");
            let asset = assets.get(&asset_id).ok_or_else(|| scope.missing(asset_id))?;
            let bytes = loader
                .load(asset_id, &root.join(&asset.rel_path))
                .map_err(|error| scope.unreadable(error))?;
            if let Some(reference) =
                Reference::from_fragment(*fragment, bucket.bucket(), bytes, asset.mime.clone())
            {
                references.push(reference);
            }
        }
    }
    Ok(references)
}

async fn prepare_blocking<T: Send + 'static>(
    lost: &'static str,
    prepare: impl FnOnce() -> CommandResult<T> + Send + 'static,
) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(prepare)
        .await
        .map_err(|error| WobuError::new(Code::Internal, lost).with_detail(error.to_string()))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReceiptLora {
    node_id: Id,
    content_hash: String,
    provider_name: String,
    trigger_token: String,
    strength: f32,
}

impl ReceiptLora {
    fn weight(&self) -> LoraWeight {
        LoraWeight {
            content_hash: self.content_hash.clone(),
            provider_name: self.provider_name.clone(),
            trigger_token: self.trigger_token.clone(),
            strength: self.strength,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoraDowngrade {
    node_id: Id,
    content_hash: String,
    state: &'static str,
    detail: String,
}

#[derive(Clone)]
struct ResolvedLoras {
    receipts: Vec<ReceiptLora>,
    weights: Vec<LoraWeight>,
    downgrades: Vec<LoraDowngrade>,
}

#[derive(Debug, Clone, Copy)]
struct Price {
    per_image_usd_micros: u64,
    conservative_fallback: bool,
}

struct ReceiptPreparation<'a> {
    batch_index: usize,
    batch_size: usize,
    requested_aspect: AspectRatio,
    actual_aspect: AspectRatio,
    resolution: Resolution,
    negative_prompt_supported: bool,
    seed_source: SeedSource,
    cost_usd_micros: u64,
    reference_asset_ids: &'a [Id],
    loras: &'a [ReceiptLora],
    lora_downgrades: &'a [LoraDowngrade],
    price: Option<Price>,
    /// The transient controls that produced this image, as History replays
    /// them. Single-subject and scene shapes differ — a composition has no
    /// per-layer sliders and no Shot label — so the shape is supplied by the
    /// normalized controls rather than reconstructed here.
    controls: Value,
}

impl ReceiptPreparation<'_> {
    fn params(&self) -> Map<String, Value> {
        let mut params = Map::new();
        params.insert("batchIndex".into(), json!(self.batch_index));
        params.insert("batchSize".into(), json!(self.batch_size));
        params.insert("requestedAspect".into(), json!(self.requested_aspect.to_string()));
        params.insert("aspect".into(), json!(self.actual_aspect.to_string()));
        params.insert("width".into(), json!(self.resolution.width));
        params.insert("height".into(), json!(self.resolution.height));
        params.insert("negativePromptSupported".into(), json!(self.negative_prompt_supported));
        params.insert("seedSource".into(), json!(self.seed_source));
        params.insert("estimatedCostUsdMicros".into(), json!(self.cost_usd_micros));
        params.insert("referenceAssetIds".into(), json!(self.reference_asset_ids));
        params.insert("loras".into(), json!(self.loras));
        params.insert("loraDowngrades".into(), json!(self.lora_downgrades));
        params.insert("controls".into(), self.controls.clone());
        apply_pricing_metadata(&mut params, self.price);
        params
    }
}

fn apply_pricing_metadata(params: &mut Map<String, Value>, price: Option<Price>) {
    if let Some(price) = price {
        params.insert("pricingCheckedAt".into(), json!(PRICE_CHECKED_AT));
        params.insert("pricingSource".into(), json!(PRICE_SOURCE));
        params.insert("pricingIndicative".into(), json!(true));
        params.insert("pricingConservativeFallback".into(), json!(price.conservative_fallback));
    } else {
        params.remove("pricingCheckedAt");
        params.remove("pricingSource");
        params.remove("pricingIndicative");
        params.remove("pricingConservativeFallback");
    }
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

/// Disposable display cache for the immutable receipt ledger.
///
/// Admission never trusts this file. It reconstructs the same values from the
/// canonical receipts while holding [`SpendLock`], then refreshes this cache as
/// a side effect. Losing or corrupting it therefore costs one reconstruction,
/// not either money or history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpendAggregate {
    version: u32,
    spent_usd_micros: u64,
    receipts: usize,
}

const MAX_GRID_CELLS: usize = 16;

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

fn selected_image_provider(project: &Project) -> CommandResult<ProviderChoice> {
    ProviderChoice::of(project, "image").ok_or_else(no_image_provider)
}

#[derive(Clone)]
struct GenerationPlanRequest {
    subject_id: Id,
    preset_id: Option<String>,
    sliders: Vec<GenerateSlider>,
    shot: GenerateShot,
    aspect: Option<String>,
    seed: u64,
    seed_source: SeedSource,
    locked_seed: Option<u64>,
    grid: Option<VariantGrid>,
    /// Emit one image instead of the preset's whole batch.
    ///
    /// A preset declares `images` because a set of four is what makes a
    /// variation batch worth looking at — but the same preset is also the only
    /// way to say "this shot, these influences", and a user who wants one
    /// picture of it should not have to pay for four to get it. So this trims
    /// the batch rather than adding a preset per count: the framing, the
    /// priorities and the aspect are the preset's, and only how many of them
    /// are sent changes.
    single: bool,
}

/// The controls that are true of the whole batch, after normalization.
///
/// Deliberately *not* the aspect or the slider values: a variant grid varies
/// one of those per cell, so the authoritative copy of each lives on the
/// [`VariantCell`]. Recording a second batch-level copy here is how a report
/// and a receipt end up disagreeing about the same generation.
#[derive(Debug, Clone)]
struct NormalizedControls {
    shot_label: String,
    shot_weight: f32,
    user_prompt: String,
    muted_nodes: HashSet<Id>,
}

impl NormalizedControls {
    /// `params.controls` for one cell of a batch.
    ///
    /// The slider values are the *cell's*, not the request's: a fragment-weight
    /// grid varies exactly one of them per cell, and a receipt that recorded the
    /// request's would describe a generation that never happened.
    fn receipt_controls(&self, cell_sliders: &[(Id, f32)]) -> Value {
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
struct PreparedGenerationPlan {
    subject_id: Id,
    subject_name: String,
    controls: NormalizedControls,
    cells: Vec<VariantCell>,
    locked_seed: Option<u64>,
}

/// Whether the seed is about to produce images or only an estimate.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SeedIntent {
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
fn resolve_seed(
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
fn locked_seed_of(nodes: &[Node], subject_id: Id) -> Option<u64> {
    nodes.iter().find(|node| node.id == subject_id).and_then(|node| node.locked_seed)
}

fn normalize_prompt(prompt: Option<&str>) -> String {
    prompt.map(str::trim).unwrap_or_default().to_owned()
}

fn normalize_aspect(requested: Option<&str>, fallback: &str) -> CommandResult<AspectRatio> {
    let requested = requested.map(str::trim).filter(|value| !value.is_empty()).unwrap_or(fallback);
    AspectRatio::parse(requested).ok_or_else(|| {
        WobuError::new(Code::Invalid, "That is not a supported aspect ratio.")
            .with_detail(requested.to_owned())
    })
}

fn normalize_weight(weight: Option<f32>) -> f32 {
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

fn prepare_generation_plan(
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
        request.single,
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

fn resolve_generation_stack<'a>(
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

fn fragments_for_cell<'a>(
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

/// What an image adapter is being built for.
///
/// Carries the credential because the two are the same decision: a call that
/// will really be made needs this machine's key and has to say so by name when
/// there isn't one, and a preview needs no key at all because `Capabilities`
/// are a pure function of the model id.
#[derive(Clone, Copy)]
enum BackendPurpose<'a> {
    Generate(&'a Keys),
    Replay(&'a Keys),
    /// Reading capabilities only. The adapter is built with a placeholder
    /// credential and must never be used to send anything.
    Preview,
}

impl<'a> BackendPurpose<'a> {
    /// The keychain to read, and what to say when it holds nothing. `None` for
    /// a preview, which needs no credential.
    fn credential(self) -> Option<(&'a Keys, &'static str)> {
        match self {
            BackendPurpose::Generate(keys) => Some((
                keys,
                "Gemini is selected for images, but there is no key on this machine. Add one in Settings.",
            )),
            BackendPurpose::Replay(keys) => Some((
                keys,
                "This replay used Gemini, but there is no Gemini key on this machine. Add one in Settings.",
            )),
            BackendPurpose::Preview => None,
        }
    }

    fn unsupported(self, provider: &str) -> WobuError {
        let message = match self {
            BackendPurpose::Replay(_) => {
                format!("This build has no image adapter for the recorded provider {provider}.")
            }
            _ => format!("This build has no image adapter for {provider}."),
        };
        WobuError::new(Code::Invalid, message)
    }
}

fn provider_unavailable(error: ImageError) -> WobuError {
    WobuError::new(Code::ProviderUnavailable, error.to_string())
}

/// Every image adapter that needs no local endpoint.
///
/// Split from the ComfyUI arm because ComfyUI is the one adapter the callers
/// legitimately disagree about — see [`image_backend`] and
/// [`unprobed_image_backend`] — while the hosted arms are identical everywhere
/// and were previously spelled out three times.
fn hosted_image_backend(
    provider: &str,
    purpose: BackendPurpose<'_>,
) -> CommandResult<Arc<dyn ImageBackend>> {
    if provider != gemini::ID {
        return Err(purpose.unsupported(provider));
    }
    let key = match purpose.credential() {
        Some((keys, missing)) => keys
            .secret(gemini::ID)
            .ok_or_else(|| WobuError::new(Code::ProviderNoKey, missing))?
            .expose()
            .to_owned(),
        None => "capability-preview".to_owned(),
    };
    Ok(Arc::new(GeminiBackend::new(key).map_err(provider_unavailable)?))
}

/// The adapter for a caller that can await, which is every caller but one.
///
/// ComfyUI is *probed*: `capabilities` for a local server are what that server
/// actually has — its models and the resolution its VRAM allows — so an
/// unprobed adapter would answer with assumptions.
async fn image_backend(
    provider: &str,
    machine: &MachineSettings,
    purpose: BackendPurpose<'_>,
) -> CommandResult<Arc<dyn ImageBackend>> {
    if provider == comfy::ID {
        return Ok(Arc::new(machine.connect_comfy_image().await.map_err(provider_unavailable)?));
    }
    hosted_image_backend(provider, purpose)
}

/// The same adapter for the one caller that cannot await: the synchronous
/// reference report.
///
/// The difference is deliberate and is a difference in what the answer means. A
/// probe is a network round trip, and the report is recomputed on every slider
/// nudge; more importantly, a local server that is switched off would turn an
/// advisory panel into an error. So the report reads ComfyUI's *declared*
/// capabilities and is allowed to be optimistic about a machine that is not
/// listening — where execution, which is about to spend real time, is not.
fn unprobed_image_backend(
    provider: &str,
    machine: &MachineSettings,
    purpose: BackendPurpose<'_>,
) -> CommandResult<Arc<dyn ImageBackend>> {
    if provider == comfy::ID {
        return Ok(Arc::new(machine.comfy_image().map_err(provider_unavailable)?));
    }
    hosted_image_backend(provider, purpose)
}

/// Everything planning reads from the open project.
struct PlanningInputs {
    root: PathBuf,
    project_id: Id,
    nodes: Vec<Node>,
    assets: Vec<Asset>,
    selection: ProviderChoice,
}

/// Take one copy of the project state a plan is built from, then let go.
///
/// The lock is held for exactly this read and is released before the adapter is
/// built, which is where the network is. Both queueing commands read the same
/// five things and differ only in what a read-only project should be told they
/// were trying to save.
fn planning_inputs(state: &AppState, read_only: &'static str) -> CommandResult<PlanningInputs> {
    state.with(|project| {
        if project.is_read_only() {
            return Err(WobuError::new(Code::ReadOnly, read_only));
        }
        // Order matters and is the order the caller would want to be told
        // about: writable, then configured, then readable.
        let selection = selected_image_provider(project)?;
        Ok(PlanningInputs {
            root: project.root().to_path_buf(),
            project_id: project.id(),
            nodes: project.world_nodes()?.to_vec(),
            assets: project.list_assets()?,
            selection,
        })
    })
}

/// Start one image and return the queue id immediately.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri exposes these as named bridge arguments.
pub async fn generate_start(
    app: AppHandle,
    state: State<'_, AppState>,
    jobs: State<'_, Jobs>,
    keys: State<'_, Keys>,
    machine: State<'_, MachineSettings>,
    subject_id: Id,
    preset: Option<String>,
    sliders: Option<Vec<GenerateSlider>>,
    shot: Option<GenerateShot>,
    aspect: Option<String>,
    model: Option<String>,
    seed: Option<u64>,
    grid: Option<VariantGrid>,
    single: Option<bool>,
    views: Option<Vec<String>>,
) -> CommandResult<String> {
    let read_only = "This project is read-only, so a generated image could not be saved.";
    let PlanningInputs { root, project_id, nodes, assets, selection } =
        planning_inputs(&state, read_only)?;

    let backend =
        image_backend(&selection.provider, &machine, BackendPurpose::Generate(&keys)).await?;
    let model = selection.model(model, backend.default_model());
    let locked_seed = locked_seed_of(&nodes, subject_id);
    let (seed, seed_source) = resolve_seed(seed, locked_seed, SeedIntent::Execute);

    let mut plan = prepare_blocking("Generation preparation stopped unexpectedly.", move || {
        prepare(Prepare {
            root,
            project_id,
            nodes,
            assets,
            request: GenerationPlanRequest {
                subject_id,
                preset_id: preset,
                sliders: sliders.unwrap_or_default(),
                shot: shot.unwrap_or_default(),
                aspect,
                seed,
                seed_source,
                locked_seed,
                grid,
                single: single.unwrap_or(false),
            },
            model,
            backend,
            provider: selection.provider,
            app,
        })
    })
    .await?;
    keep_named_views(&mut plan, views.as_deref())?;
    plan.reserve_spend()?;
    let id = jobs.queue().submit(plan);
    Ok(id.to_string())
}

/// Re-roll part of a named-view preset instead of the whole sheet.
///
/// The Turnaround preset emits eight images on one locked seed, which is what
/// makes them views of the same object — and is also why "this back view came
/// out wrong" cannot be answered by generating eight more. #110's review step
/// needs one image, tagged with the view it replaces, on a seed of its own.
///
/// Applied by filtering the prepared plan rather than by teaching the planner a
/// subset: the cells are already built and already carry their `view_type`, and
/// a second path through `variant_cells` would be a second place for the eight
/// names to be spelled. Spend is reserved *after* this, so a one-view reroll
/// reserves one image.
fn keep_named_views(plan: &mut GenerateTask, views: Option<&[String]>) -> CommandResult<()> {
    let Some(views) = views.filter(|views| !views.is_empty()) else { return Ok(()) };
    let wanted: HashSet<&str> = views.iter().map(|view| view.trim()).collect();
    plan.plans.retain(|planned| {
        planned.generation.view_type.as_deref().is_some_and(|view| wanted.contains(view))
    });
    if plan.plans.is_empty() {
        return Err(WobuError::new(
            Code::Invalid,
            "That preset does not emit any of the named views that were asked for.",
        ));
    }
    // The receipt says which of *this* batch each image is, and after filtering
    // this batch is the reroll rather than the sheet it came from.
    let size = plan.plans.len();
    for (index, planned) in plan.plans.iter_mut().enumerate() {
        planned.generation.params.insert("batchIndex".into(), json!(index));
        planned.generation.params.insert("batchSize".into(), json!(size));
    }
    let stem = plan.label.rsplit_once(" ×").map_or(plan.label.as_str(), |(head, _)| head);
    plan.label = format!("{stem} ×{size}");
    Ok(())
}

/// Queue one image containing two to four ordered world entities.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri exposes these as named bridge arguments.
pub async fn scene_generate_start(
    app: AppHandle,
    state: State<'_, AppState>,
    jobs: State<'_, Jobs>,
    keys: State<'_, Keys>,
    machine: State<'_, MachineSettings>,
    subject_ids: Vec<Id>,
    prompt: Option<String>,
    aspect: Option<String>,
    model: Option<String>,
    seed: Option<u64>,
) -> CommandResult<String> {
    if !(2..=4).contains(&subject_ids.len()) {
        return Err(WobuError::new(Code::Invalid, "A scene needs two to four entities."));
    }
    let distinct: HashSet<Id> = subject_ids.iter().copied().collect();
    if distinct.len() != subject_ids.len() {
        return Err(WobuError::new(Code::Invalid, "A scene cannot contain the same entity twice."));
    }

    let read_only = "This project is read-only, so a generated scene could not be saved.";
    let PlanningInputs { root, project_id, nodes, assets, selection } =
        planning_inputs(&state, read_only)?;

    for subject_id in &subject_ids {
        let subject = nodes.iter().find(|node| node.id == *subject_id).ok_or_else(|| {
            WobuError::new(Code::NoSuchNode, "A scene entity is not in this project any more.")
                .with_detail(subject_id.to_string())
        })?;
        if kind_def(subject.kind).singleton {
            return Err(WobuError::new(
                Code::Invalid,
                "Style Guides and World Bibles are scene context, not scene entities.",
            ));
        }
    }

    let backend =
        image_backend(&selection.provider, &machine, BackendPurpose::Generate(&keys)).await?;
    let model = selection.model(model, backend.default_model());
    // A composition has no locked seed of its own: its participants may each
    // have one and they may disagree, so there is nothing to inherit.
    let (seed, seed_source) = resolve_seed(seed, None, SeedIntent::Execute);
    let mut plan = prepare_blocking("Scene preparation stopped unexpectedly.", move || {
        prepare_scene(ScenePrepare {
            root,
            project_id,
            nodes,
            assets,
            subject_ids,
            prompt: prompt.unwrap_or_default(),
            aspect,
            model,
            seed,
            seed_source,
            backend,
            provider: selection.provider,
            app,
        })
    })
    .await?;
    plan.reserve_spend()?;
    let id = jobs.queue().submit(plan);
    Ok(id.to_string())
}

/// Queue the exact provider request captured by an immutable generation.
///
/// This path deliberately never resolves today's world, selects today's
/// preset, or negotiates against today's capabilities. Those are compilation
/// steps and replay means compilation already happened. Current state is used
/// only to locate the old receipt and its referenced immutable asset bytes.
#[tauri::command]
pub async fn generation_replay(
    app: AppHandle,
    state: State<'_, AppState>,
    jobs: State<'_, Jobs>,
    keys: State<'_, Keys>,
    machine: State<'_, MachineSettings>,
    generation_id: Id,
) -> CommandResult<String> {
    let (root, project_id, generation, assets) = state.with(|project| {
        if project.is_read_only() {
            return Err(WobuError::new(
                Code::ReadOnly,
                "This project is read-only, so a replayed image could not be saved.",
            ));
        }
        let generation = project.get_generation(generation_id)?.ok_or_else(|| {
            WobuError::new(Code::Invalid, "That generation is not in this project any more.")
        })?;
        Ok((project.root().to_path_buf(), project.id(), generation, project.list_assets()?))
    })?;
    let subject_name = generation
        .influence_snapshot
        .layers
        .iter()
        .find(|layer| layer.node_id == Some(generation.node_id))
        .map(|layer| layer.node_name.clone())
        .unwrap_or_else(|| format!("generation {}", generation.id));

    let backend =
        image_backend(&generation.backend, &machine, BackendPurpose::Replay(&keys)).await?;
    let capabilities = backend.capabilities(&generation.model);
    let requires_billing = capabilities.requires_billing;
    let replay_root = root.clone();
    let replay_backend = Arc::clone(&backend);
    let plan = prepare_blocking("Replay preparation stopped unexpectedly.", move || {
        replay_plan(&replay_root, &assets, generation, &capabilities, replay_backend.as_ref())
    })
    .await?;
    if requires_billing && plan.cost_usd_micros == 0 {
        return Err(WobuError::new(
            Code::Invalid,
            "This paid historical model has no current safe price, so replay cannot reserve the spend ceiling.",
        ));
    }
    let subject_id = plan.generation.node_id;
    let mut task = PlannedBatch {
        label: format!("Replay {subject_name}"),
        subject_id,
        plans: vec![plan],
        requires_billing,
        archival_replay: true,
    }
    .into_task(root, project_id, backend, app);
    task.reserve_spend()?;
    let id = jobs.queue().submit(task);
    Ok(id.to_string())
}

/// Replay's one planned image.
///
/// Deliberately not a [`ReceiptPreparation`]: replay copies the original
/// receipt and patches the handful of fields that are about *this* attempt,
/// because preparing a fresh one would re-derive values from today's
/// capabilities and today's registry. It joins the shared path at
/// [`PlannedBatch::into_task`], which is where spend reservation lives.
fn replay_plan(
    root: &Path,
    assets: &[Asset],
    original: Generation,
    capabilities: &Capabilities,
    backend: &dyn ImageBackend,
) -> CommandResult<PlannedImage> {
    let aspect_text = replay_param_str(&original, "aspect")?;
    let aspect = AspectRatio::parse(aspect_text).ok_or_else(|| {
        replay_metadata_error(&original, format!("recorded aspect {aspect_text:?} is invalid"))
    })?;
    let width = replay_param_u32(&original, "width")?;
    let height = replay_param_u32(&original, "height")?;
    let resolution = Resolution::new(width, height);
    let by_id: HashMap<Id, &Asset> = assets.iter().map(|asset| (asset.id, asset)).collect();
    let mut references = Vec::new();
    let mut reference_loader = ReferenceLoader::new();

    for fragment in original
        .influence_snapshot
        .layers
        .iter()
        .flat_map(|layer| layer.fragments.iter())
        .filter(|fragment| !fragment.dropped)
    {
        let Some(asset_id) = fragment.asset_id else { continue };
        let Some(mechanism) = ReferenceMechanism::for_target(fragment.target) else {
            continue;
        };
        let asset = by_id.get(&asset_id).ok_or_else(|| {
            WobuError::new(
                Code::NoSuchAsset,
                "A reference captured by this generation snapshot is missing, so it cannot be replayed verbatim.",
            )
            .with_detail(asset_id.to_string())
        })?;
        let role =
            fragment.asset_role.or_else(|| snapshot_role(&fragment.section, fragment.target));
        let role = role.ok_or_else(|| {
            replay_metadata_error(
                &original,
                format!("reference {asset_id} has no reconstructable role"),
            )
        })?;
        let requested_bucket = RefBucket::for_role(role).ok_or_else(|| {
            replay_metadata_error(
                &original,
                format!("reference {asset_id} has a non-conditioning role"),
            )
        })?;
        let bucket = capabilities.image_refs.meter(requested_bucket).0;
        let bytes =
            reference_loader.load(asset_id, &root.join(&asset.rel_path)).map_err(|error| {
                WobuError::new(
                    Code::Io,
                    "A reference captured by this generation snapshot could not be read.",
                )
                .with_detail(error.to_string())
            })?;
        references.push(Reference {
            asset_id,
            role,
            bucket,
            mechanism,
            weight: fragment.weight,
            bytes,
            mime: asset.mime.clone(),
        });
    }
    restore_reference_order(&original, &mut references)?;
    let recorded_loras: Vec<ReceiptLora> = match original.params.get("loras") {
        None => Vec::new(),
        Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
            replay_metadata_error(&original, format!("recorded LoRA metadata is invalid: {error}"))
        })?,
    };
    let mut lora_hashes = HashSet::new();
    let mut loras = Vec::with_capacity(recorded_loras.len());
    for recorded in recorded_loras {
        if !lora_hashes.insert(recorded.content_hash.clone()) {
            return Err(replay_metadata_error(
                &original,
                "recorded LoRA list contains a duplicate content hash".into(),
            ));
        }
        validate_replay_lora(root, &original, &recorded, backend)?;
        loras.push(recorded.weight());
    }

    let request = ImageRequest {
        model: original.model.clone(),
        prompt: original.compiled_prompt.clone(),
        negative: original.negative_prompt.clone(),
        aspect,
        resolution,
        seed: original.seed,
        references,
        loras,
    };
    let current_price = image_price(&original.backend, &original.model, resolution);
    let current_cost = current_price.map_or(0, |price| price.per_image_usd_micros);
    let mut generation = original.clone();
    generation.id = new_id();
    generation.created_at = Utc::now();
    generation.output_asset_ids.clear();
    generation.params.remove("outcome");
    generation.params.remove("errorCode");
    if let Some(original_cost) =
        generation.params.get("estimatedCostUsdMicros").and_then(Value::as_u64)
    {
        generation
            .params
            .insert("replayOriginalEstimatedCostUsdMicros".into(), json!(original_cost));
    }
    generation.params.insert("replayOf".into(), json!(original.id));
    generation.params.insert("replayPriceBasis".into(), json!("current"));
    generation.params.insert("batchIndex".into(), json!(0));
    generation.params.insert("batchSize".into(), json!(1));
    generation.params.insert("seedSource".into(), json!("replay"));
    generation.params.insert("estimatedCostUsdMicros".into(), json!(current_cost));
    apply_pricing_metadata(&mut generation.params, current_price);

    Ok(PlannedImage { request, cost_usd_micros: current_cost, generation })
}

fn validate_replay_lora(
    root: &Path,
    generation: &Generation,
    lora: &ReceiptLora,
    backend: &dyn ImageBackend,
) -> CommandResult<()> {
    if !lora.strength.is_finite()
        || !(0.0..=2.0).contains(&lora.strength)
        || !safe_trigger_token(&lora.trigger_token)
        || !safe_lora_name(&lora.provider_name)
    {
        return Err(replay_metadata_error(generation, "recorded LoRA fields are invalid".into()));
    }
    if !backend.supports_lora(&generation.model, &lora.provider_name) {
        return Err(WobuError::new(
            Code::ProviderUnavailable,
            "The provider can no longer apply every LoRA captured by this generation, so replay cannot be verbatim.",
        )
        .with_detail(lora.provider_name.clone()));
    }
    let rel_path = wobu_core::asset::lora_path(&lora.content_hash).ok_or_else(|| {
        replay_metadata_error(generation, "recorded LoRA content hash is invalid".into())
    })?;
    let path = root.join(rel_path);
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
        WobuError::new(
            Code::Io,
            "A project-owned LoRA captured by this generation is missing, so replay cannot be verbatim.",
        )
        .with_detail(error.to_string())
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(replay_metadata_error(
            generation,
            "recorded LoRA path is not a regular project-owned file".into(),
        ));
    }
    let bytes = std::fs::read(&path).map_err(|error| {
        WobuError::new(Code::Io, "A recorded LoRA could not be read for replay.")
            .with_detail(error.to_string())
    })?;
    if wobu_store::atomic::hash_bytes(&bytes) != lora.content_hash
        || wobu_store::lora::validate(&bytes).is_err()
    {
        return Err(replay_metadata_error(
            generation,
            "recorded LoRA failed its content or safetensors integrity check".into(),
        ));
    }
    Ok(())
}

fn resolve_loras(
    root: &Path,
    nodes: &[Node],
    ordered_node_ids: impl IntoIterator<Item = Id>,
    model: &str,
    backend: &dyn ImageBackend,
) -> ResolvedLoras {
    let by_id: HashMap<Id, &Node> = nodes.iter().map(|node| (node.id, node)).collect();
    let mut visited_nodes = HashSet::new();
    let mut applied_hashes = HashSet::new();
    let mut receipts = Vec::new();
    let mut weights = Vec::new();
    let mut downgrades = Vec::new();
    for node_id in ordered_node_ids {
        if !visited_nodes.insert(node_id) {
            continue;
        }
        let Some(pin) = by_id.get(&node_id).and_then(|node| node.lora.as_ref()) else {
            continue;
        };
        let reject = |state: &'static str, detail: String| LoraDowngrade {
            node_id,
            content_hash: pin.hash.clone(),
            state,
            detail,
        };
        if pin.protocol != LORA_PROTOCOL {
            downgrades.push(reject(
                "protocol_mismatch",
                format!("The pin uses trainer protocol {}, not {LORA_PROTOCOL}.", pin.protocol),
            ));
            continue;
        }
        if pin.base_model != model {
            downgrades.push(reject(
                "model_mismatch",
                format!("The LoRA was trained for {}, not {model}.", pin.base_model),
            ));
            continue;
        }
        if !pin.strength.is_finite() || !(0.0..=2.0).contains(&pin.strength) {
            downgrades.push(reject("weight_corrupt", "The LoRA strength is invalid.".into()));
            continue;
        }
        if !safe_trigger_token(&pin.trigger_token) || !safe_lora_name(&pin.provider_name) {
            downgrades.push(reject(
                "pin_invalid",
                "The LoRA pin contains an unsafe trigger token or provider filename.".into(),
            ));
            continue;
        }
        let Some(expected_path) = wobu_core::asset::lora_path(&pin.hash) else {
            downgrades.push(reject("pin_invalid", "The LoRA content hash is invalid.".into()));
            continue;
        };
        if pin.rel_path != expected_path {
            downgrades.push(reject(
                "pin_invalid",
                "The LoRA path does not match its content hash.".into(),
            ));
            continue;
        }
        let path = root.join(expected_path);
        let valid_bytes = std::fs::symlink_metadata(&path)
            .ok()
            .filter(|metadata| metadata.file_type().is_file() && !metadata.file_type().is_symlink())
            .filter(|metadata| metadata.len() == pin.bytes)
            .and_then(|_| std::fs::read(&path).ok())
            .filter(|bytes| wobu_store::atomic::hash_bytes(bytes) == pin.hash)
            .filter(|bytes| wobu_store::lora::validate(bytes).is_ok());
        if valid_bytes.is_none() {
            downgrades.push(reject(
                "weight_missing_or_corrupt",
                "The project-owned LoRA is missing or failed its integrity check.".into(),
            ));
            continue;
        }
        if !backend.supports_lora(model, &pin.provider_name) {
            downgrades.push(reject(
                "provider_unsupported",
                "The probed provider cannot load this LoRA for the selected model.".into(),
            ));
            continue;
        }
        if !applied_hashes.insert(pin.hash.clone()) {
            downgrades.push(reject(
                "deduplicated",
                "An earlier influence source already applies the same content-addressed LoRA."
                    .into(),
            ));
            continue;
        }
        let receipt = ReceiptLora {
            node_id,
            content_hash: pin.hash.clone(),
            provider_name: pin.provider_name.clone(),
            trigger_token: pin.trigger_token.clone(),
            strength: pin.strength,
        };
        weights.push(receipt.weight());
        receipts.push(receipt);
    }
    ResolvedLoras { receipts, weights, downgrades }
}

fn prompt_with_lora_triggers(prompt: &str, loras: &[LoraWeight]) -> String {
    let triggers = missing_lora_triggers(prompt, loras);
    if triggers.is_empty() {
        prompt.to_owned()
    } else if prompt.trim().is_empty() {
        triggers.join(", ")
    } else {
        format!("{}, {}", prompt.trim_end(), triggers.join(", "))
    }
}

fn scene_prompt_with_lora_triggers(prompt: &str, loras: &[LoraWeight]) -> String {
    let triggers = missing_lora_triggers(prompt, loras);
    if triggers.is_empty() {
        return prompt.to_owned();
    }
    let trigger_clause = triggers.join(", ");
    match prompt.rsplit_once("; ") {
        Some((before_identity, identity)) => {
            format!("{before_identity}; {trigger_clause}; {identity}")
        }
        None if prompt.trim().is_empty() => trigger_clause,
        None => format!("{trigger_clause}; {prompt}"),
    }
}

fn missing_lora_triggers<'a>(prompt: &str, loras: &'a [LoraWeight]) -> Vec<&'a str> {
    let existing: HashSet<&str> = prompt
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || character == '_' || character == '-')
        })
        .filter(|part| !part.is_empty())
        .collect();
    let mut seen = HashSet::new();
    loras
        .iter()
        .map(|lora| lora.trigger_token.as_str())
        .filter(|token| !existing.contains(token) && seen.insert(*token))
        .collect()
}

fn safe_trigger_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 64
        && token.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn safe_lora_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && name.ends_with(".safetensors")
        && !name.starts_with('/')
        && !name.contains('\\')
        && name.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

fn replay_param_str<'a>(generation: &'a Generation, key: &str) -> CommandResult<&'a str> {
    generation
        .params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| replay_metadata_error(generation, format!("missing {key}")))
}

fn replay_param_u32(generation: &Generation, key: &str) -> CommandResult<u32> {
    generation
        .params
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| replay_metadata_error(generation, format!("missing or invalid {key}")))
}

fn replay_metadata_error(generation: &Generation, detail: String) -> WobuError {
    WobuError::new(
        Code::Invalid,
        "This older generation does not contain enough immutable request metadata to replay verbatim.",
    )
    .with_detail(format!("generation {}: {detail}", generation.id))
}

fn snapshot_role(section: &str, target: FragmentTarget) -> Option<AssetRole> {
    match section {
        "silhouette" => Some(AssetRole::Silhouette),
        "palette" => Some(AssetRole::Palette),
        "material" => Some(AssetRole::Material),
        "mood" => Some(AssetRole::Mood),
        "pose" => Some(AssetRole::Pose),
        "costume" => Some(AssetRole::Costume),
        "full_ref" => Some(AssetRole::FullRef),
        _ => match target {
            FragmentTarget::StyleRef => Some(AssetRole::FullRef),
            FragmentTarget::StructureRef => Some(AssetRole::Silhouette),
            FragmentTarget::Palette => Some(AssetRole::Palette),
            FragmentTarget::Prompt | FragmentTarget::Negative | FragmentTarget::MoodboardOnly => {
                None
            }
        },
    }
}

fn restore_reference_order(
    generation: &Generation,
    references: &mut Vec<Reference>,
) -> CommandResult<()> {
    let Some(order) = generation.params.get("referenceAssetIds").and_then(Value::as_array) else {
        // Before `referenceAssetIds` was recorded, prepare still emitted a
        // deterministic order: all object-bucket references, then characters,
        // then style refs, preserving extraction order inside each bucket.
        // Recreate that stable grouping rather than sending snapshot layer
        // order and calling it verbatim.
        references.sort_by_key(|reference| match reference.bucket {
            RefBucket::Objects => 0,
            RefBucket::Characters => 1,
            RefBucket::StyleRefs => 2,
        });
        return Ok(());
    };
    if order.len() != references.len() {
        return Err(replay_metadata_error(
            generation,
            "recorded reference order does not match the snapshot".to_string(),
        ));
    }
    let mut ordered = Vec::with_capacity(references.len());
    for value in order {
        let id = value.as_str().and_then(|value| Id::from_string(value).ok()).ok_or_else(|| {
            replay_metadata_error(generation, "recorded reference order is invalid".to_string())
        })?;
        let index =
            references.iter().position(|reference| reference.asset_id == id).ok_or_else(|| {
                replay_metadata_error(
                    generation,
                    format!("recorded reference {id} is not in the snapshot"),
                )
            })?;
        ordered.push(references.remove(index));
    }
    if !references.is_empty() {
        return Err(replay_metadata_error(
            generation,
            "snapshot has references missing from the recorded order".to_string(),
        ));
    }
    *references = ordered;
    Ok(())
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
    locked_seed: Option<u64>,
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
    provider: String,
    model: String,
    aspect_ratios: Vec<AspectRatio>,
    flexible_aspect: bool,
    previews: Vec<ImageAspectPreview>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAspectPreview {
    requested_aspect: AspectRatio,
    actual_aspect: AspectRatio,
    width: u32,
    height: u32,
    substituted: bool,
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

fn aspect_capability_view(
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
        let archive = root.join(".wobu").join(format!(
            "spend-recovery-{}-{}",
            Utc::now().format("%Y%m%dT%H%M%SZ"),
            new_id()
        ));
        std::fs::rename(&ledger, &archive).map_err(|error| {
            spend_io("The pending spend ledger could not be archived.", &ledger, error)
        })?;
    }
    spend_status_for(&root)
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
    single: Option<bool>,
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
            single: single.unwrap_or(false),
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
fn reference_report_for_plan(
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
    slider_values: &[(Id, f32)],
    available_nodes: &HashSet<Id>,
    grid: Option<&VariantGrid>,
    single: bool,
    caps: &Capabilities,
) -> CommandResult<Vec<VariantCell>> {
    /*
     * Both of these say how many images the batch is, so asking for both is a
     * question with two answers rather than a preference to reconcile. Refused
     * here, at the one place the count is decided, so the bridge and the MCP
     * surface cannot each grow their own idea of which wins.
     */
    if single && grid.is_some() {
        return Err(WobuError::new(
            Code::Invalid,
            "A single image and a variant grid would both decide how many pictures this batch is. \
             Ask for one or the other.",
        ));
    }
    if single && !chosen.views.is_empty() {
        return Err(WobuError::new(
            Code::Invalid,
            "Named-view presets such as Turnaround emit one image per view, so a single image \
             would be a sheet with no views. Ask for the view you want instead.",
        ));
    }

    let Some(grid) = grid else {
        let mut items = chosen.generations(base_seed);
        // The first, not a random one: it is the cell that carries the seed the
        // caller asked for, so re-rolling a single image and locking the seed
        // that produced it describe the same picture.
        if single {
            items.truncate(1);
        }
        return Ok(items
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

struct ScenePrepare {
    root: PathBuf,
    project_id: Id,
    nodes: Vec<Node>,
    assets: Vec<Asset>,
    /// The full ordered participant set is carried to every composition step.
    /// Participant LoRA collection belongs here too: #69 can collect compatible
    /// pins for every id and dedupe by hash without changing this request seam.
    subject_ids: Vec<Id>,
    prompt: String,
    aspect: Option<String>,
    model: String,
    seed: u64,
    seed_source: SeedSource,
    backend: Arc<dyn ImageBackend>,
    provider: String,
    app: AppHandle,
}

/// Composition's half of the shared planning seam. Same shape and same rules as
/// [`BatchPlan`]: borrowed project state in, immutable [`PlannedBatch`] out, no
/// `AppHandle` and no queue.
struct ScenePlan<'a> {
    root: &'a Path,
    nodes: &'a [Node],
    assets: &'a [Asset],
    subject_ids: &'a [Id],
    prompt: &'a str,
    aspect: Option<&'a str>,
    model: &'a str,
    provider: &'a str,
    seed: u64,
    seed_source: SeedSource,
    backend: &'a dyn ImageBackend,
}

#[derive(Debug, Clone, PartialEq)]
struct NormalizedSceneControls {
    prompt: String,
    aspect: AspectRatio,
    shot_weight: f32,
}

fn normalize_scene_controls(
    prompt: &str,
    aspect: Option<&str>,
) -> CommandResult<NormalizedSceneControls> {
    Ok(NormalizedSceneControls {
        prompt: normalize_prompt(Some(prompt)),
        aspect: normalize_aspect(aspect, SCENE_ASPECT)?,
        shot_weight: normalize_weight(None),
    })
}

impl NormalizedSceneControls {
    /// `params.controls` for a composition.
    ///
    /// Deliberately a different shape from the single-subject one: a scene has
    /// no per-layer sliders and no Shot label to re-apply, and History reads the
    /// `scene` key to know which of the two it is looking at.
    fn receipt_controls(&self) -> Value {
        json!({
            "scene": {
                "prompt": self.prompt,
                "aspect": self.aspect.to_string(),
            },
        })
    }
}

fn prepare_scene(input: ScenePrepare) -> CommandResult<GenerateTask> {
    let planned = plan_scene(ScenePlan {
        root: &input.root,
        nodes: &input.nodes,
        assets: &input.assets,
        subject_ids: &input.subject_ids,
        prompt: &input.prompt,
        aspect: input.aspect.as_deref(),
        model: &input.model,
        provider: &input.provider,
        seed: input.seed,
        seed_source: input.seed_source,
        backend: input.backend.as_ref(),
    })?;
    Ok(planned.into_task(input.root, input.project_id, input.backend, input.app))
}

fn plan_scene(input: ScenePlan<'_>) -> CommandResult<PlannedBatch> {
    let controls = normalize_scene_controls(input.prompt, input.aspect)?;
    let world = World::new(input.nodes.iter());
    let names: Vec<String> = input
        .subject_ids
        .iter()
        .map(|id| {
            world
                .get(*id)
                .map(|node| node.name.clone())
                .ok_or_else(|| WobuError::new(Code::NoSuchNode, "A scene entity disappeared."))
        })
        .collect::<Result<_, _>>()?;
    let scene_label = format!("Scene · {}", names.join(" + "));
    let scene = resolve_scene(
        &world,
        input.subject_ids,
        Shot { label: &scene_label, weight: controls.shot_weight },
    )
    .map_err(|error| {
        WobuError::new(Code::Invalid, "The scene influence stacks could not be composed.")
            .with_detail(format!("{error:?}"))
    })?;
    let mut extracted = scene_fragments(&world, &scene);
    append_scene_prompt(&scene, &mut extracted, &controls.prompt);
    let requested_aspect = controls.aspect;
    let caps = input.backend.capabilities(input.model);
    let negotiated = negotiate_scene(&extracted, requested_aspect, &caps, input.subject_ids);
    ensure_scene_reference_fairness(input.subject_ids, &extracted, &negotiated)?;
    let (prompt, negative) = compile_scene_prompt(&scene, negotiated.fragments(), &names);
    let loras = resolve_loras(
        input.root,
        input.nodes,
        scene.stack().sources().iter().filter_map(|source| source.node_id()),
        input.model,
        input.backend,
    );
    let prompt = scene_prompt_with_lora_triggers(&prompt, &loras.weights);
    if prompt.trim().is_empty() {
        return Err(WobuError::new(
            Code::Invalid,
            "Describe at least one scene entity before generating.",
        ));
    }

    let assets: HashMap<Id, &Asset> = input.assets.iter().map(|asset| (asset.id, asset)).collect();
    let mut reference_loader = ReferenceLoader::new();
    let references = load_references(
        &negotiated,
        &assets,
        input.root,
        &mut reference_loader,
        ReferenceScope::Scene,
    )?;
    let dropped: Vec<FragmentKey> = negotiated
        .images()
        .dropped()
        .map(|drop| FragmentKey::of(drop.fragment))
        .chain(negotiated.downgrades().iter().map(|drop| FragmentKey::of(drop.fragment)))
        .collect();
    let resolution = negotiated.resolution();
    let price = image_price(input.provider, input.model, resolution);
    let cost_usd_micros = price.map_or(0, |price| price.per_image_usd_micros);
    let reference_asset_ids =
        references.iter().map(|reference| reference.asset_id).collect::<Vec<_>>();
    let mut params = ReceiptPreparation {
        batch_index: 0,
        batch_size: 1,
        requested_aspect,
        actual_aspect: negotiated.aspect(),
        resolution,
        negative_prompt_supported: caps.negative_prompt,
        seed_source: input.seed_source,
        cost_usd_micros,
        reference_asset_ids: &reference_asset_ids,
        loras: &loras.receipts,
        lora_downgrades: &loras.downgrades,
        price,
        controls: controls.receipt_controls(),
    }
    .params();
    params.insert(
        "sceneComposition".into(),
        serde_json::to_value(SceneComposition {
            version: 1,
            subject_ids: input.subject_ids.to_vec(),
            subject_names: names.clone(),
        })
        .map_err(|error| {
            WobuError::new(Code::Internal, "The scene receipt could not be encoded.")
                .with_detail(error.to_string())
        })?,
    );
    let request = ImageRequest::new(input.model.to_owned(), &prompt, input.seed, &negotiated)
        .with_negative(&negative)
        .with_references(references)
        .with_loras(loras.weights);
    let primary = input.subject_ids[0];
    let generation = Generation {
        id: new_id(),
        node_id: primary,
        created_at: Utc::now(),
        // Scene composition deliberately uses the registry's wide establishing
        // preset for framing/aspect. `params.sceneComposition` is the mode and
        // participant record; inventing a preset id no registry knows would
        // make history and replay disagree about which framing was chosen.
        preset: "environment_matte".into(),
        view_type: None,
        user_prompt: controls.prompt.clone(),
        compiled_prompt: prompt,
        negative_prompt: negative,
        backend: input.provider.to_owned(),
        model: input.model.to_owned(),
        seed: input.seed,
        params,
        output_asset_ids: Vec::new(),
        influence_snapshot: snapshot(scene.stack(), &extracted, &[], &HashSet::new(), &dropped),
    };
    Ok(PlannedBatch {
        label: format!("Compose scene · {}", names.join(" + ")),
        subject_id: primary,
        plans: vec![PlannedImage { request, cost_usd_micros, generation }],
        requires_billing: caps.requires_billing,
        archival_replay: false,
    })
}

fn append_scene_prompt<'a>(
    scene: &ResolvedScene<'a>,
    fragments: &mut Vec<Fragment<'a>>,
    prompt: &'a str,
) {
    if prompt.is_empty() {
        return;
    }
    if let Some(source) =
        scene.stack().sources().iter().find(|source| source.layer == wobu_core::Layer::Shot)
    {
        let fragment = Fragment::new(
            source,
            "user_prompt",
            FragmentBody::Text(prompt),
            source.weight,
            FragmentTarget::Prompt,
        );
        let identity = fragments
            .iter()
            .position(|fragment| fragment.section() == "scene_identity")
            .unwrap_or(fragments.len());
        fragments.insert(identity, fragment);
    }
}

fn compile_scene_prompt(
    scene: &ResolvedScene<'_>,
    fragments: &[Fragment<'_>],
    names: &[String],
) -> (String, String) {
    let mut shared = Vec::new();
    let mut by_subject: HashMap<Id, Vec<&str>> =
        scene.subjects().iter().copied().map(|id| (id, Vec::new())).collect();
    let mut shot = Vec::new();
    let mut negatives = Vec::new();
    for fragment in fragments.iter().copied().filter(|fragment| fragment.contributes()) {
        let Some(text) = fragment.text() else { continue };
        match fragment.target() {
            FragmentTarget::Negative => push_unique(&mut negatives, text),
            FragmentTarget::Prompt => match fragment.node_id() {
                None => push_unique(&mut shot, text),
                Some(id) => match scene.scope_for_node(id) {
                    Some(SceneScope::Subject(subject)) => {
                        if let Some(values) = by_subject.get_mut(&subject) {
                            push_unique(values, text);
                        }
                    }
                    Some(SceneScope::Shared) | None => push_unique(&mut shared, text),
                },
            },
            FragmentTarget::StyleRef
            | FragmentTarget::StructureRef
            | FragmentTarget::Palette
            | FragmentTarget::MoodboardOnly => {}
        }
    }

    let mut clauses = Vec::new();
    if !shared.is_empty() {
        clauses.push(format!("Shared world and style: {}", shared.join(", ")));
    }
    for (subject, name) in scene.subjects().iter().zip(names) {
        if let Some(values) = by_subject.get(subject)
            && !values.is_empty()
        {
            clauses.push(format!("{name}: {}", values.join(", ")));
        }
    }
    clauses.extend(shot.into_iter().map(str::to_owned));
    (clauses.join("; "), negatives.join(", "))
}

fn push_unique<'a>(values: &mut Vec<&'a str>, value: &'a str) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn ensure_scene_reference_fairness(
    subjects: &[Id],
    offered: &[Fragment<'_>],
    negotiated: &wobu_imagine::Negotiated<'_>,
) -> CommandResult<()> {
    let kept: HashSet<(Id, AssetRole)> = negotiated
        .images()
        .kept()
        .filter_map(|fragment| Some((fragment.asset_id()?, fragment.asset_role()?)))
        .collect();
    for subject in subjects {
        let direct: Vec<(Id, AssetRole)> = offered
            .iter()
            .copied()
            .filter(|fragment| {
                fragment.node_id() == Some(*subject)
                    && fragment.contributes()
                    && fragment.is_sendable()
            })
            .filter_map(|fragment| Some((fragment.asset_id()?, fragment.asset_role()?)))
            .collect();
        if !direct.is_empty() && !direct.iter().any(|reference| kept.contains(reference)) {
            return Err(WobuError::new(
                Code::Invalid,
                "The selected image model cannot keep one identity reference for every scene entity.",
            )
            .with_detail(format!("no reference slot remained for {subject}")));
        }
    }
    Ok(())
}

struct Prepare {
    root: PathBuf,
    project_id: Id,
    nodes: Vec<Node>,
    assets: Vec<Asset>,
    request: GenerationPlanRequest,
    model: String,
    backend: Arc<dyn ImageBackend>,
    provider: String,
    app: AppHandle,
}

/// The batch planning seam.
///
/// Everything downstream of [`prepare_generation_plan`] — negotiation, prompt
/// compilation, reference loading, pricing and receipt preparation — happens
/// here, from borrowed project state and a borrowed adapter. Nothing in it can
/// reach the running app, which is what lets the same call the Inspector's
/// preview makes be exercised directly by a test.
struct BatchPlan<'a> {
    root: &'a Path,
    nodes: &'a [Node],
    assets: &'a [Asset],
    request: GenerationPlanRequest,
    model: &'a str,
    provider: &'a str,
    backend: &'a dyn ImageBackend,
}

fn prepare(input: Prepare) -> CommandResult<GenerateTask> {
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

fn plan_batch(input: BatchPlan<'_>) -> CommandResult<PlannedBatch> {
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
        let request =
            ImageRequest::new(input.model.to_owned(), &compiled_prompt, cell.item.seed, &negotiated)
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

fn append_user_prompt<'a>(
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
    /// Replay can legitimately outlive the node its immutable receipt names.
    archival_replay: bool,
}

struct PlannedImage {
    request: ImageRequest,
    cost_usd_micros: u64,
    generation: Generation,
}

/// A finished plan: every provider request and every receipt this job will
/// write, decided before anything is queued and unable to change afterwards.
///
/// This is the single output of planning. Batch generation ([`plan_batch`]),
/// scene composition ([`plan_scene`]) and replay ([`replay_plan`]) all produce
/// one, and [`PlannedBatch::into_task`] is the only way any of them reaches the
/// queue — so spend reservation, billing and the archival-replay exemption are
/// stated once rather than three times.
struct PlannedBatch {
    label: String,
    subject_id: Id,
    plans: Vec<PlannedImage>,
    requires_billing: bool,
    /// Replay can legitimately outlive the node its immutable receipt names.
    archival_replay: bool,
}

impl PlannedBatch {
    fn into_task(
        self,
        root: PathBuf,
        project_id: Id,
        backend: Arc<dyn ImageBackend>,
        app: AppHandle,
    ) -> GenerateTask {
        GenerateTask {
            label: self.label,
            subject_id: self.subject_id,
            project_id,
            root,
            backend,
            plans: self.plans,
            next: 0,
            completed: Vec::new(),
            app,
            requires_billing: self.requires_billing,
            reservation: None,
            archival_replay: self.archival_replay,
        }
    }
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
            WobuError::new(Code::Internal, "Paid generation started without a spend reservation.")
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
            let archival_replay = self.archival_replay;
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
                        if archival_replay {
                            project.record_replay_generation(generation).map_err(WobuError::from)
                        } else {
                            project.record_generation(generation).map_err(WobuError::from)
                        }
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
                    if billing_may_be_unknown(&error, outcome.usage, self.requires_billing)
                        && let Some(reservation) = self.reservation.as_mut()
                    {
                        reservation.release_on_drop = false;
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
                    let generation = if archival_replay {
                        project.record_replay_generation(generation)?
                    } else {
                        project.record_generation(generation)?
                    };
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
        let within = done.min(total).saturating_mul(100).checked_div(total).unwrap_or(100);
        let overall = self.batch_index as u32 * 100 + within;
        let prefix = format!("Image {}/{}", self.batch_index + 1, self.batch_total);
        let note = note.map_or(prefix.clone(), |note| format!("{prefix} · {note}"));
        self.ctx.progress(Progress::new(overall, self.batch_total as u32 * 100).with_note(note));
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
                if requires_billing =>
            {
                Billed::Unknown
            }
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

fn billing_may_be_unknown(error: &ImageError, usage: ImageUsage, requires_billing: bool) -> bool {
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

#[cfg(test)]
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
    let batch_usd_micros =
        prices.iter().fold(0_u64, |total, price| total.saturating_add(price.per_image_usd_micros));
    let varies_by_cell =
        prices.iter().any(|price| price.per_image_usd_micros != first.per_image_usd_micros);
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
                    return Err(spend_io("The spend ledger could not be locked.", &path, error));
                }
            }
        }
        Err(WobuError::new(Code::Io, "The shared spend ledger is busy. Try Generate again."))
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
        let status = reconstruct_spend_status_locked(root)?;
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
            .ok_or_else(|| {
                WobuError::new(Code::Invalid, "The project spend total is too large.")
            })?;
        if committed > ceiling {
            return Err(WobuError::new(
                Code::SpendCeilingExceeded,
                "This batch would cross the project's shared spend ceiling.",
            )
            .with_detail(format!(
                "spent={} reserved={} batch={} ceiling={} USD micros",
                status.spent_usd_micros, status.reserved_usd_micros, amount_usd_micros, ceiling,
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
        Ok(SpendReservation { root: root.to_path_buf(), path, file, release_on_drop: true })
    }

    fn commit(&mut self, amount_usd_micros: u64) -> CommandResult<()> {
        if amount_usd_micros > self.file.remaining_usd_micros {
            return Err(WobuError::new(
                Code::Internal,
                "A generation cost exceeded its spend reservation.",
            ));
        }
        let _guard = SpendLock::acquire(&self.root)?;
        // The receipt was persisted before this call. Refresh from canonical
        // bytes rather than adding `amount_usd_micros` to a mutable counter: a
        // second process may have reconstructed the aggregate in the narrow
        // interval between that receipt landing and this lock being acquired.
        // Re-reading makes that interleaving exact instead of double-counting.
        reconstruct_spend_aggregate_with(&self.root, || {
            Project::spend_ledger(&self.root).map_err(WobuError::from)
        })?;
        let remaining = self.file.remaining_usd_micros - amount_usd_micros;
        if remaining > 0 {
            // Reservations are write-once. Publishing the replacement before
            // removing the old file means a crash can only over-reserve, never
            // open a window where concurrent work can overspend.
            let id = new_id();
            let path = self.root.join(SPEND_DIR).join("reservations").join(format!("{id}.json"));
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
    reconstruct_spend_status_locked(root)
}

fn spend_status_for_report(root: &Path) -> CommandResult<SpendStatus> {
    match SpendLock::acquire(root) {
        Ok(_guard) => read_cached_spend_status_locked(root),
        Err(_error) if root.join(SPEND_DIR).join("lock").exists() => {
            // Display-only fallback. Admission never uses this snapshot: it
            // still requires the exclusive lock. This lets the Inspector
            // explain and recover a crash-orphaned lock instead of replacing
            // the whole cost report with an opaque busy error.
            let mut status = read_cached_spend_status_locked(root)?;
            status.ledger_locked = true;
            Ok(status)
        }
        Err(error) => Err(error),
    }
}

/// Admission's view: canonical receipts are opened and validated every time.
/// The aggregate is refreshed only after that succeeds, so no mutable cache can
/// authorise spend or hide a malformed receipt.
fn reconstruct_spend_status_locked(root: &Path) -> CommandResult<SpendStatus> {
    let (ceiling_usd_micros, aggregate) = reconstruct_spend_aggregate_with(root, || {
        Project::spend_ledger(root).map_err(WobuError::from)
    })?;
    status_with_reservations(root, ceiling_usd_micros, aggregate.spent_usd_micros)
}

/// Display's view: one small aggregate plus the changing reservation set.
/// Cache loss is recoverable and pays for one strict reconstruction; unchanged
/// five-second polls never walk a month shard or open a receipt.
fn read_cached_spend_status_locked(root: &Path) -> CommandResult<SpendStatus> {
    read_cached_spend_status_locked_with(root, || {
        Project::spend_ledger(root).map_err(WobuError::from)
    })
}

fn read_cached_spend_status_locked_with(
    root: &Path,
    reconstruct: impl FnOnce() -> CommandResult<(Option<u64>, Vec<Generation>)>,
) -> CommandResult<SpendStatus> {
    let (ceiling_usd_micros, aggregate) = match read_spend_aggregate(root) {
        Some(aggregate) => (Project::spend_ceiling(root)?, aggregate),
        None => reconstruct_spend_aggregate_with(root, reconstruct)?,
    };
    status_with_reservations(root, ceiling_usd_micros, aggregate.spent_usd_micros)
}

fn reconstruct_spend_aggregate_with(
    root: &Path,
    read_ledger: impl FnOnce() -> CommandResult<(Option<u64>, Vec<Generation>)>,
) -> CommandResult<(Option<u64>, SpendAggregate)> {
    let (ceiling_usd_micros, receipts) = read_ledger()?;
    let spent_usd_micros = receipts.iter().try_fold(0_u64, |total, generation| {
        total
            .checked_add(receipt_cost(generation))
            .ok_or_else(|| WobuError::new(Code::Invalid, "The project spend total is too large."))
    })?;
    let aggregate = SpendAggregate {
        version: SPEND_AGGREGATE_VERSION,
        spent_usd_micros,
        receipts: receipts.len(),
    };
    // Disposable optimisation only. A read-only or temporarily unavailable
    // cache must never turn a successfully reconstructed canonical ledger into
    // a failed admission; the next call can reconstruct it again.
    let _ = write_spend_aggregate(root, &aggregate);
    Ok((ceiling_usd_micros, aggregate))
}

fn read_spend_aggregate(root: &Path) -> Option<SpendAggregate> {
    let path = root.join(SPEND_DIR).join(SPEND_AGGREGATE);
    let aggregate: SpendAggregate = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (aggregate.version == SPEND_AGGREGATE_VERSION).then_some(aggregate)
}

fn write_spend_aggregate(root: &Path, aggregate: &SpendAggregate) -> CommandResult<()> {
    let path = root.join(SPEND_DIR).join(SPEND_AGGREGATE);
    let mut file =
        OpenOptions::new().write(true).create(true).truncate(true).open(&path).map_err(
            |error| spend_io("The spend display cache could not be written.", &path, error),
        )?;
    serde_json::to_writer(&mut file, aggregate).map_err(|error| {
        WobuError::new(Code::Internal, "The spend display cache could not be encoded.")
            .with_detail(error.to_string())
    })?;
    file.flush()
        .map_err(|error| spend_io("The spend display cache could not be written.", &path, error))?;
    file.sync_all()
        .map_err(|error| spend_io("The spend display cache could not be secured.", &path, error))
}

fn status_with_reservations(
    root: &Path,
    ceiling_usd_micros: Option<u64>,
    spent_usd_micros: u64,
) -> CommandResult<SpendStatus> {
    let reservations = root.join(SPEND_DIR).join("reservations");
    let mut reserved_usd_micros = 0_u64;
    let mut pending_reservations = 0_usize;
    let mut oldest_reservation_at: Option<String> = None;
    for entry in std::fs::read_dir(&reservations).map_err(|error| {
        spend_io("The spend reservations could not be read.", &reservations, error)
    })? {
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
        reserved_usd_micros =
            reserved_usd_micros.checked_add(reservation.remaining_usd_micros).ok_or_else(|| {
                WobuError::new(Code::Invalid, "The reserved spend total is too large.")
            })?;
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
    WobuError::new(Code::Invalid, "Choose an image provider in Settings before generating.")
}

#[cfg(test)]
mod tests {
    use wobu_core::{AssetRef, Description, SectionValue};

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
            let node_id =
                project.create_node(wobu_core::NodeKind::Character, "Kael", None).unwrap().id;
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

    /* ── the planning fixture ─────────────────────────────────────────────── */

    /// The model every planning test below prices against. Pinned rather than
    /// defaulted: a default that moved would silently rewrite every expected
    /// cost in this file.
    const MODEL: &str = "gemini-3.1-flash-image";

    /// A world with files in it: a house style, two characters, and one
    /// reference image each on disk.
    ///
    /// Planning reads reference bytes, so the images have to exist; everything
    /// else is in memory, because the point of the seam under test is that
    /// planning takes nodes and assets rather than an open project.
    struct PlanWorld {
        parent: PathBuf,
        root: PathBuf,
        nodes: Vec<Node>,
        assets: Vec<Asset>,
        kael: Id,
        rell: Id,
        kael_costume: Id,
    }

    impl PlanWorld {
        fn new() -> PlanWorld {
            let parent = std::env::temp_dir().join(format!("wobu-plan-test-{}", new_id()));
            let root = parent.join("world");
            std::fs::create_dir_all(root.join("assets/img")).unwrap();

            let mut assets = Vec::new();
            let mut attach = |owner: &mut Node, name: &str, role: AssetRole| {
                let id = new_id();
                let rel_path = format!("assets/img/{name}.png");
                std::fs::write(root.join(&rel_path), name.as_bytes()).unwrap();
                owner.asset_links.push(AssetRef::new(id, role));
                assets.push(Asset {
                    id,
                    hash: format!("hash-{name}"),
                    kind: AssetKind::Reference,
                    rel_path,
                    thumb_path: None,
                    mime: "image/png".into(),
                    width: 1_024,
                    height: 1_024,
                    bytes: name.len() as u64,
                    created_at: Utc::now(),
                });
                id
            };

            let mut style = Node::new(wobu_core::NodeKind::StyleGuide, "Ashfall House Style")
                .expect("fixture names are sluggable");
            describe(&mut style, [("medium", prose("Oil on board"))]);

            let mut kael = Node::new(wobu_core::NodeKind::Character, "Kael Vantris")
                .expect("fixture names are sluggable");
            describe(
                &mut kael,
                [
                    ("silhouette", prose("Tall, narrow, hooded")),
                    ("costume", prose("Ash-grey longcoat")),
                    ("never", list(&["modern firearms"])),
                ],
            );
            let kael_costume = attach(&mut kael, "kael-costume", AssetRole::Costume);

            let mut rell = Node::new(wobu_core::NodeKind::Character, "Rell Sarn")
                .expect("fixture names are sluggable");
            describe(&mut rell, [("silhouette", prose("Short, broad, plated"))]);
            attach(&mut rell, "rell-costume", AssetRole::Costume);

            let (kael_id, rell_id) = (kael.id, rell.id);
            PlanWorld {
                parent,
                root,
                nodes: vec![style, kael, rell],
                assets,
                kael: kael_id,
                rell: rell_id,
                kael_costume,
            }
        }

        fn backend() -> GeminiBackend {
            GeminiBackend::new("test-key").expect("the placeholder key is well formed")
        }

        fn request(&self, preset_id: &str, seed: u64) -> GenerationPlanRequest {
            GenerationPlanRequest {
                subject_id: self.kael,
                preset_id: Some(preset_id.to_owned()),
                sliders: Vec::new(),
                shot: GenerateShot::default(),
                aspect: None,
                seed,
                seed_source: SeedSource::Random,
                locked_seed: None,
                grid: None,
                single: false,
            }
        }

        fn plan(&self, request: GenerationPlanRequest) -> CommandResult<PlannedBatch> {
            let backend = PlanWorld::backend();
            plan_batch(BatchPlan {
                root: &self.root,
                nodes: &self.nodes,
                assets: &self.assets,
                request,
                model: MODEL,
                provider: gemini::ID,
                backend: &backend,
            })
        }

        fn compose(&self, prompt: &str, aspect: Option<&str>) -> CommandResult<PlannedBatch> {
            let backend = PlanWorld::backend();
            plan_scene(ScenePlan {
                root: &self.root,
                nodes: &self.nodes,
                assets: &self.assets,
                subject_ids: &[self.kael, self.rell],
                prompt,
                aspect,
                model: MODEL,
                provider: gemini::ID,
                seed: 7,
                seed_source: SeedSource::Random,
                backend: &backend,
            })
        }

        fn preview(&self, request: GenerationPlanRequest) -> CommandResult<ImageReferenceReport> {
            let backend = PlanWorld::backend();
            reference_report_for_plan(&self.nodes, gemini::ID, MODEL, &backend, request)
        }
    }

    impl Drop for PlanWorld {
        fn drop(&mut self) {
            // The parent is a unique path minted by this test, never a caller
            // supplied directory or a workspace root.
            let _ = std::fs::remove_dir_all(&self.parent);
        }
    }

    fn describe(node: &mut Node, sections: impl IntoIterator<Item = (&'static str, SectionValue)>) {
        node.description = Some(Description::from_sections(
            sections.into_iter().map(|(key, value)| (key.to_owned(), value)),
        ));
    }

    fn prose(value: &str) -> SectionValue {
        SectionValue::Text(value.to_owned())
    }

    fn list(items: &[&str]) -> SectionValue {
        SectionValue::List(items.iter().map(|item| (*item).to_owned()).collect())
    }

    fn keys(params: &Map<String, Value>) -> Vec<&str> {
        params.keys().map(String::as_str).collect()
    }

    fn receipt(node_id: Id, backend: &str, model: &str, width: u32, height: u32) -> Generation {
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
                ("aspect".into(), json!("1:1")),
                ("width".into(), json!(width)),
                ("height".into(), json!(height)),
            ]),
            output_asset_ids: Vec::new(),
            influence_snapshot: InfluenceSnapshot { layers: Vec::new() },
        }
    }

    #[test]
    fn sixteen_cell_batch_reads_each_reference_once_and_shares_its_buffer() {
        let reads = std::cell::Cell::new(0);
        let mut loader = ReferenceLoader::with_reader(|path| {
            reads.set(reads.get() + 1);
            Ok(path.to_string_lossy().as_bytes().to_vec())
        });
        let costume = new_id();
        let palette = new_id();

        let cells: Vec<_> = (0..MAX_GRID_CELLS)
            .map(|_| {
                [
                    loader.load(costume, Path::new("costume.png")).unwrap(),
                    loader.load(palette, Path::new("palette.png")).unwrap(),
                ]
            })
            .collect();

        assert_eq!(reads.get(), 2, "one filesystem read per unique asset");
        for cell in &cells[1..] {
            assert!(Arc::ptr_eq(&cells[0][0], &cell[0]));
            assert!(Arc::ptr_eq(&cells[0][1], &cell[1]));
        }
        assert!(!Arc::ptr_eq(&cells[0][0], &cells[0][1]));
        assert_eq!(Arc::strong_count(&cells[0][0]), MAX_GRID_CELLS + 1);
        assert_eq!(Arc::strong_count(&cells[0][1]), MAX_GRID_CELLS + 1);
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
        let unknown =
            image_price(gemini::ID, "gemini-future-image", Resolution::new(1_024, 1_024)).unwrap();
        assert_eq!(unknown.per_image_usd_micros, 240_000);
        assert!(unknown.conservative_fallback);
    }

    #[test]
    fn aspect_preview_exposes_ordered_choices_and_the_negotiated_substitution() {
        let mut caps =
            GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
        caps.max_resolution = Resolution::new(1_024, 1_024);
        caps.aspect_ratios =
            ["1:1", "2:3"].into_iter().map(|value| AspectRatio::parse(value).unwrap()).collect();

        let preview = aspect_capability_view(gemini::ID.into(), "restricted".into(), caps);

        assert_eq!(
            preview.aspect_ratios,
            [AspectRatio::parse("1:1").unwrap(), AspectRatio::parse("2:3").unwrap()]
        );
        let portrait = preview
            .previews
            .iter()
            .find(|candidate| candidate.requested_aspect == AspectRatio::parse("3:4").unwrap())
            .unwrap();
        assert!(portrait.substituted);
        assert_eq!(portrait.actual_aspect, AspectRatio::parse("2:3").unwrap());
        assert_eq!((portrait.width, portrait.height), (682, 1_023));
    }

    #[test]
    fn flexible_aspect_preview_uses_the_curated_validated_vocabulary() {
        let mut caps =
            GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
        caps.max_resolution = Resolution::new(2_048, 2_048);
        caps.aspect_ratios.clear();

        let preview = aspect_capability_view(comfy::ID.into(), "local".into(), caps);

        assert!(preview.flexible_aspect);
        assert_eq!(preview.aspect_ratios, AspectRatio::ALL);
        assert!(preview.previews.iter().all(|candidate| !candidate.substituted));
        let square = &preview.previews[0];
        assert_eq!(square.actual_aspect, AspectRatio::parse("1:1").unwrap());
        assert_eq!((square.width, square.height), (2_048, 2_048));
    }

    #[test]
    fn lora_triggers_are_deduplicated_and_scene_identity_stays_last() {
        let loras = vec![
            LoraWeight {
                content_hash: "a".repeat(64),
                provider_name: "first.safetensors".into(),
                trigger_token: "wobu_kael".into(),
                strength: 0.8,
            },
            LoraWeight {
                content_hash: "b".repeat(64),
                provider_name: "second.safetensors".into(),
                trigger_token: "wobu_kael".into(),
                strength: 0.7,
            },
        ];
        assert_eq!(prompt_with_lora_triggers("portrait", &loras), "portrait, wobu_kael");
        assert_eq!(
            prompt_with_lora_triggers("portrait of wobu_kael", &loras),
            "portrait of wobu_kael",
        );
        assert_eq!(
            scene_prompt_with_lora_triggers(
                "Shared world; wide framing; preserve every named identity",
                &loras,
            ),
            "Shared world; wide framing; wobu_kael; preserve every named identity",
        );
    }

    #[test]
    fn batch_estimate_and_old_receipts_use_recorded_model_and_size() {
        let estimate =
            cost_estimate(gemini::ID, "gemini-3.1-flash-image", Resolution::new(2_048, 2_048), 8)
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
    fn replay_plan_uses_recorded_request_and_current_price_without_compiling() {
        let mut original = receipt(new_id(), gemini::ID, "gemini-3.1-flash-image", 1_024, 1_024);
        original.compiled_prompt = "the immutable positive".into();
        original.negative_prompt = "the immutable negative".into();
        original.seed = 77;
        original.params.insert("estimatedCostUsdMicros".into(), json!(12_345));
        let original_id = original.id;
        let original_snapshot = original.influence_snapshot.clone();

        let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
        let backend = GeminiBackend::new("test-key").unwrap();
        let plan = replay_plan(Path::new("."), &[], original, &caps, &backend).unwrap();
        assert_eq!(plan.request.prompt, "the immutable positive");
        assert_eq!(plan.request.negative, "the immutable negative");
        assert_eq!(plan.request.seed, 77);
        assert_eq!(plan.request.resolution, Resolution::new(1_024, 1_024));
        assert_eq!(plan.generation.influence_snapshot, original_snapshot);
        assert_eq!(plan.generation.params.get("replayOf"), Some(&json!(original_id)));
        assert_eq!(
            plan.generation.params.get("replayOriginalEstimatedCostUsdMicros"),
            Some(&json!(12_345))
        );
        assert_eq!(plan.cost_usd_micros, 67_000);
        assert_eq!(plan.generation.params.get("estimatedCostUsdMicros"), Some(&json!(67_000)));
    }

    #[test]
    fn replay_refuses_missing_snapshot_reference_instead_of_using_current_links() {
        let missing = new_id();
        let mut original = receipt(new_id(), comfy::ID, "local", 1_024, 1_024);
        original.influence_snapshot.layers.push(SnapshotLayer {
            layer: wobu_core::Layer::Subject,
            node_id: Some(original.node_id),
            node_name: "Kael".into(),
            weight: 1.0,
            muted: false,
            fragments: vec![SnapshotFragment {
                section: "pose".into(),
                text: None,
                asset_id: Some(missing),
                asset_role: Some(AssetRole::Pose),
                weight: 0.8,
                target: FragmentTarget::StructureRef,
                dropped: false,
            }],
        });
        let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
        let backend = GeminiBackend::new("test-key").unwrap();
        let Err(error) = replay_plan(Path::new("."), &[], original, &caps, &backend) else {
            panic!("a missing immutable reference must refuse replay")
        };
        assert_eq!(error.code, Code::NoSuchAsset);
        assert!(error.detail.is_some_and(|detail| detail.contains(&missing.to_string())));
    }

    #[test]
    fn variant_cells_change_one_axis_and_report_the_real_output_count() {
        let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
        let chosen = *preset("character_sheet").unwrap();
        let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
        let available = HashSet::from([subject.id]);
        let weight_grid =
            VariantGrid::FragmentWeight { node_id: subject.id, values: vec![0.4, 0.7, 1.0] };
        let weights = variant_cells(
            &subject,
            chosen,
            AspectRatio::parse("3:4").unwrap(),
            42,
            SeedSource::Locked,
            &[],
            &available,
            Some(&weight_grid),
            false,
            &caps,
        )
        .unwrap();
        assert_eq!(weights.len(), 3);
        assert!(weights.iter().all(|cell| cell.item.seed == 42));
        assert_eq!(
            weights.iter().map(|cell| cell.slider_values[0].1).collect::<Vec<_>>(),
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
            &[],
            &available,
            Some(&seed_grid),
            false,
            &caps,
        )
        .unwrap();
        assert_eq!(
            seeds.iter().map(|cell| cell.item.seed).collect::<Vec<_>>(),
            [11, 22, 33, 44, 55]
        );

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
    fn preview_and_execution_prepare_the_same_normalized_plan() {
        let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
        let subject_id = subject.id;
        let nodes = vec![subject];
        let backend = GeminiBackend::new("test-key").unwrap();
        let model = "gemini-3.1-flash-image";
        let caps = backend.capabilities(model);
        let request = GenerationPlanRequest {
            subject_id,
            preset_id: Some("character_sheet".into()),
            sliders: vec![GenerateSlider { node_id: subject_id, value: 0.75, muted: false }],
            shot: GenerateShot {
                label: Some("  low angle  ".into()),
                weight: Some(1.5),
                prompt: Some("  wind catches the cloak  ".into()),
            },
            aspect: Some(" 3:4 ".into()),
            seed: 42,
            seed_source: SeedSource::Locked,
            locked_seed: Some(42),
            grid: None,
            single: false,
        };

        let preview =
            reference_report_for_plan(&nodes, gemini::ID, model, &backend, request.clone())
                .unwrap();
        let execution = prepare_generation_plan(&nodes, request, &caps).unwrap();
        assert_eq!(execution.controls.shot_label, "low angle");
        assert_eq!(execution.controls.shot_weight, 1.0);
        assert_eq!(execution.controls.user_prompt, "wind catches the cloak");
        // The per-cell copies are the authoritative ones, and for a request
        // with no grid every cell agrees with the request.
        assert_eq!(execution.cells[0].slider_values, [(subject_id, 0.75)]);
        assert_eq!(execution.cells[0].aspect, AspectRatio::parse("3:4").unwrap());

        let execution_stack = resolve_generation_stack(&nodes, &execution).unwrap();
        let execution_fragments = fragments_for_cell(
            &execution_stack,
            &execution.cells[0],
            &execution.controls.user_prompt,
        );
        let negotiated = negotiate(&execution_fragments, execution.cells[0].aspect, &caps);
        let execution_price = image_price(gemini::ID, model, negotiated.resolution()).unwrap();
        let preview_cost = preview.cost.unwrap();
        assert_eq!(preview_cost.images, execution.cells.len());
        assert_eq!(preview_cost.per_image_usd_micros, execution_price.per_image_usd_micros);
    }

    #[test]
    fn variant_grids_and_scene_composition_share_control_normalization() {
        let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
        let subject_id = subject.id;
        let nodes = vec![subject];
        let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
        let plan = prepare_generation_plan(
            &nodes,
            GenerationPlanRequest {
                subject_id,
                preset_id: Some("character_sheet".into()),
                sliders: Vec::new(),
                shot: GenerateShot {
                    label: None,
                    weight: None,
                    prompt: Some("  hold the horizon  ".into()),
                },
                aspect: Some(" 16:9 ".into()),
                seed: 9,
                seed_source: SeedSource::Random,
                locked_seed: None,
                grid: Some(VariantGrid::Aspect { values: vec![" 16:9 ".into(), " 1:1 ".into()] }),
                single: false,
            },
            &caps,
        )
        .unwrap();
        let scene = normalize_scene_controls("  hold the horizon  ", Some(" 16:9 ")).unwrap();

        assert_eq!(plan.controls.user_prompt, scene.prompt);
        assert_eq!(plan.controls.shot_weight, scene.shot_weight);
        assert_eq!(plan.cells[0].aspect, scene.aspect);
        assert_eq!(plan.cells[1].aspect, AspectRatio::parse("1:1").unwrap());
    }

    /* ── what the three callers plan ──────────────────────────────────────── */

    #[test]
    fn a_batch_plans_one_receipt_and_one_request_per_cell() {
        let world = PlanWorld::new();
        let mut request = world.request("character_sheet", 100);
        request.locked_seed = Some(100);
        request.seed_source = SeedSource::Locked;
        request.shot = GenerateShot {
            label: Some("  low angle  ".into()),
            weight: Some(0.5),
            prompt: Some("  wind catches the cloak  ".into()),
        };
        let planned = world.plan(request).unwrap();

        assert_eq!(planned.label, "Generate Kael Vantris ×4");
        assert_eq!(planned.subject_id, world.kael);
        assert!(planned.requires_billing);
        assert!(!planned.archival_replay);
        // `character_sheet` emits four images on the adjacent-seed family, and
        // only the first of them is the lock itself.
        assert_eq!(
            planned.plans.iter().map(|plan| plan.generation.seed).collect::<Vec<_>>(),
            [100, 101, 102, 103]
        );
        assert_eq!(
            planned.plans.iter().map(|plan| plan.request.seed).collect::<Vec<_>>(),
            [100, 101, 102, 103]
        );

        let first = &planned.plans[0].generation;
        assert_eq!(
            keys(&first.params),
            [
                "aspect",
                "batchIndex",
                "batchSize",
                "controls",
                "estimatedCostUsdMicros",
                "height",
                "lockedSeed",
                "loraDowngrades",
                "loras",
                "negativePromptSupported",
                "pricingCheckedAt",
                "pricingConservativeFallback",
                "pricingIndicative",
                "pricingSource",
                "referenceAssetIds",
                "requestedAspect",
                "seedSource",
                "usedLockedSeed",
                "width",
            ]
        );
        assert_eq!(first.preset, "character_sheet");
        assert_eq!(first.view_type, None);
        assert_eq!(first.user_prompt, "wind catches the cloak");
        assert_eq!(first.backend, gemini::ID);
        assert_eq!(first.model, MODEL);
        assert_eq!(first.params["batchIndex"], json!(0));
        assert_eq!(first.params["batchSize"], json!(4));
        assert_eq!(first.params["requestedAspect"], json!("3:4"));
        assert_eq!(first.params["aspect"], json!("3:4"));
        assert_eq!(first.params["seedSource"], json!("locked"));
        assert_eq!(first.params["lockedSeed"], json!(100));
        assert_eq!(first.params["usedLockedSeed"], json!(true));
        assert_eq!(first.params["referenceAssetIds"], json!([world.kael_costume]));
        assert_eq!(
            first.params["controls"],
            json!({
                "sliders": [],
                "shot": { "label": "low angle", "weight": 0.5, "prompt": "wind catches the cloak" },
            })
        );
        assert!(first.compiled_prompt.contains("Ash-grey longcoat"));
        assert!(first.compiled_prompt.contains("wind catches the cloak"));
        // Gemini has no negative prompt, so negotiation withholds the `never`
        // section rather than the receipt claiming one was sent.
        assert_eq!(first.params["negativePromptSupported"], json!(false));
        assert_eq!(first.negative_prompt, "");
        assert_eq!(planned.plans[0].request.references.len(), 1);
        assert_eq!(planned.plans[0].request.references[0].asset_id, world.kael_costume);

        // Only the first cell is the lock, and every cell is priced.
        assert_eq!(planned.plans[1].generation.params["usedLockedSeed"], json!(false));
        assert_eq!(planned.plans[1].generation.params["seedSource"], json!("locked_derived"));
        let cost = planned.plans[0].cost_usd_micros;
        assert!(cost > 0);
        assert!(planned.plans.iter().all(|plan| plan.cost_usd_micros == cost));
    }

    #[test]
    fn a_named_view_batch_plans_eight_tagged_views_on_one_seed() {
        let world = PlanWorld::new();
        let planned = world.plan(world.request("turnaround", 55)).unwrap();
        assert_eq!(
            planned
                .plans
                .iter()
                .map(|plan| (
                    plan.generation.view_type.clone().unwrap(),
                    plan.generation.seed,
                    plan.generation.params["batchSize"].clone()
                ))
                .collect::<Vec<_>>(),
            [
                ("front".to_owned(), 55, json!(8)),
                ("left".to_owned(), 55, json!(8)),
                ("right".to_owned(), 55, json!(8)),
                ("back".to_owned(), 55, json!(8)),
                ("top".to_owned(), 55, json!(8)),
                ("bottom".to_owned(), 55, json!(8)),
                ("left_front".to_owned(), 55, json!(8)),
                ("right_front".to_owned(), 55, json!(8)),
            ]
        );
    }

    #[test]
    fn a_variant_grid_varies_one_axis_and_records_it_per_cell() {
        let world = PlanWorld::new();
        let mut request = world.request("character_sheet", 12);
        request.grid = Some(VariantGrid::FragmentWeight {
            node_id: world.kael,
            values: vec![0.25, 0.75],
        });
        let planned = world.plan(request).unwrap();

        assert_eq!(planned.plans.len(), 2);
        for (index, weight) in [0.25_f32, 0.75].into_iter().enumerate() {
            let params = &planned.plans[index].generation.params;
            // One seed across the grid: the axis is the weight, nothing else.
            assert_eq!(planned.plans[index].generation.seed, 12);
            assert_eq!(params["controls"]["sliders"][0]["nodeId"], json!(world.kael));
            assert_eq!(params["controls"]["sliders"][0]["value"], json!(weight));
            assert_eq!(params["variation"]["index"], json!(index));
            assert_eq!(params["variation"]["total"], json!(2));
            assert_eq!(params["variation"]["axis"], json!("fragment_weight"));
            assert_eq!(params["variation"]["weight"], json!(weight));
        }
        assert_eq!(
            planned.plans[0].generation.params["variation"]["gridId"],
            planned.plans[1].generation.params["variation"]["gridId"]
        );
    }

    #[test]
    fn a_composition_plans_one_receipt_naming_every_participant() {
        let world = PlanWorld::new();
        let planned = world.compose("  they meet on the bridge  ", None).unwrap();

        assert_eq!(planned.label, "Compose scene · Kael Vantris + Rell Sarn");
        assert_eq!(planned.subject_id, world.kael);
        assert_eq!(planned.plans.len(), 1);
        let generation = &planned.plans[0].generation;
        assert_eq!(generation.preset, "environment_matte");
        assert_eq!(generation.node_id, world.kael);
        assert_eq!(generation.user_prompt, "they meet on the bridge");
        assert_eq!(generation.params["batchSize"], json!(1));
        assert_eq!(generation.params["requestedAspect"], json!("16:9"));
        assert_eq!(
            generation.params["controls"],
            json!({ "scene": { "prompt": "they meet on the bridge", "aspect": "16:9" } })
        );
        assert_eq!(generation.params["sceneComposition"]["version"], json!(1));
        assert_eq!(
            generation.params["sceneComposition"]["subjectIds"],
            json!([world.kael, world.rell])
        );
        assert_eq!(
            generation.params["sceneComposition"]["subjectNames"],
            json!(["Kael Vantris", "Rell Sarn"])
        );
        // Composition has no per-layer sliders and no `variation`, and it never
        // acquires a `lockedSeed`: the participants may disagree about theirs.
        assert!(!generation.params.contains_key("variation"));
        assert!(!generation.params.contains_key("lockedSeed"));
        assert!(generation.compiled_prompt.contains("Kael Vantris"));
        assert!(generation.compiled_prompt.contains("Rell Sarn"));
        assert_eq!(planned.plans[0].request.references.len(), 2);
    }

    #[test]
    fn the_preview_prices_and_budgets_exactly_what_the_batch_would_send() {
        let world = PlanWorld::new();
        let request = world.request("character_sheet", 3);
        let planned = world.plan(request.clone()).unwrap();
        let preview = world.preview(request).unwrap();

        let cost = preview.cost.unwrap();
        assert_eq!(cost.images, planned.plans.len());
        assert_eq!(cost.per_image_usd_micros, planned.plans[0].cost_usd_micros);
        assert_eq!(
            cost.batch_usd_micros,
            planned.plans.iter().map(|plan| plan.cost_usd_micros).sum::<u64>()
        );
        assert!(!cost.varies_by_cell);

        let kept: usize = preview.buckets.iter().map(|bucket| bucket.kept).sum();
        assert_eq!(kept, planned.plans[0].request.references.len());
        assert_eq!(preview.buckets.iter().map(|bucket| bucket.dropped).sum::<usize>(), 0);
        assert_eq!(preview.layers.iter().map(|layer| layer.kept).sum::<usize>(), kept);
    }

    #[test]
    fn the_preview_budgets_the_grids_first_cell_and_not_the_ungridded_request() {
        // The report used to negotiate the request's own fragment set, which
        // for a grid is a set no cell sends. Silencing the subject in cell one
        // is the case where that shows: the reference the report counted was
        // one the first image would not have carried.
        let world = PlanWorld::new();
        let mut request = world.request("character_sheet", 4);
        request.grid =
            Some(VariantGrid::FragmentWeight { node_id: world.kael, values: vec![0.0, 1.0] });
        let planned = world.plan(request.clone()).unwrap();
        let preview = world.preview(request).unwrap();

        assert_eq!(planned.plans[0].request.references.len(), 0);
        assert_eq!(planned.plans[1].request.references.len(), 1);
        assert_eq!(preview.buckets.iter().map(|bucket| bucket.kept).sum::<usize>(), 0);
    }

    #[test]
    fn the_preview_budgets_the_first_view_of_a_named_view_preset() {
        // A Turnaround's cells are per-view, so the report has to be a report
        // about a view: the viewless fragment set is one this preset never
        // sends. First rather than an average, because it is the cell execution
        // renders first and the only one a single report can be true about.
        let world = PlanWorld::new();
        let request = world.request("turnaround", 5);
        let planned = world.plan(request.clone()).unwrap();
        let preview = world.preview(request).unwrap();

        let kept: usize = preview.buckets.iter().map(|bucket| bucket.kept).sum();
        assert_eq!(kept, planned.plans[0].request.references.len());
        assert_eq!(preview.cost.unwrap().images, 8);
    }

    #[test]
    fn named_view_presets_are_refused_as_variant_grids() {
        let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
        let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
        let grid = VariantGrid::Seed { values: vec![1, 2] };
        let error = variant_cells(
            &subject,
            *preset("turnaround").unwrap(),
            AspectRatio::parse("1:1").unwrap(),
            1,
            SeedSource::Locked,
            &[],
            &HashSet::from([subject.id]),
            Some(&grid),
            false,
            &caps,
        )
        .unwrap_err();
        assert_eq!(error.code, Code::Invalid);
    }

    /// One image, on the seed that was asked for — and priced as one.
    ///
    /// The seed assertion is the point of the test rather than a detail of it:
    /// `generations` walks adjacent seeds so a ×4 batch varies, and trimming to
    /// any cell but the first would hand back a picture the caller cannot
    /// reproduce by locking the seed they chose.
    #[test]
    fn a_single_image_trims_the_preset_batch_to_one() {
        let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
        let chosen = *preset("character_sheet").unwrap();
        assert!(chosen.images > 1, "this test is only meaningful for a batch preset");
        let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
        let cells = variant_cells(
            &subject,
            chosen,
            AspectRatio::parse("3:4").unwrap(),
            42,
            SeedSource::Locked,
            &[],
            &HashSet::from([subject.id]),
            None,
            true,
            &caps,
        )
        .unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].item.seed, 42);
        assert_eq!(cells[0].item.index, 0);
        assert!(matches!(cells[0].seed_source, SeedSource::Locked));
        assert!(cells[0].variation.is_none());
    }

    /// The whole batch — label, receipts, and what gets billed — is one image.
    ///
    /// Planned rather than asserted on `variant_cells` alone: `batchSize` and
    /// the `×4` in the label are derived downstream of the cells, and a trim
    /// that left either of them describing the preset's count would put a
    /// receipt on disk that disagrees with the images beside it.
    #[test]
    fn a_single_image_plans_one_request_and_bills_for_one() {
        let world = PlanWorld::new();
        let mut request = world.request("character_sheet", 100);
        request.single = true;
        let planned = world.plan(request).unwrap();

        assert_eq!(planned.label, "Generate Kael Vantris ×1");
        assert_eq!(planned.plans.len(), 1);
        assert_eq!(planned.plans[0].generation.seed, 100);
        assert_eq!(planned.plans[0].generation.params["batchSize"], json!(1));
        assert_eq!(planned.plans[0].generation.params["batchIndex"], json!(0));

        let batch = world.plan(world.request("character_sheet", 100)).unwrap();
        assert_eq!(batch.plans.len(), 4, "the same preset unrestricted still emits its batch");
    }

    /// Both answer "how many images is this batch", so both is not a request.
    #[test]
    fn a_single_image_and_a_variant_grid_are_refused_together() {
        let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
        let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
        let grid = VariantGrid::Seed { values: vec![1, 2] };
        let error = variant_cells(
            &subject,
            *preset("character_sheet").unwrap(),
            AspectRatio::parse("1:1").unwrap(),
            1,
            SeedSource::Locked,
            &[],
            &HashSet::from([subject.id]),
            Some(&grid),
            true,
            &caps,
        )
        .unwrap_err();
        assert_eq!(error.code, Code::Invalid);
    }

    /// A turnaround trimmed to one image is a sheet with no views, which is
    /// what the `views` argument exists to ask for instead.
    #[test]
    fn named_view_presets_are_refused_as_single_images() {
        let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
        let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
        let error = variant_cells(
            &subject,
            *preset("turnaround").unwrap(),
            AspectRatio::parse("1:1").unwrap(),
            1,
            SeedSource::Locked,
            &[],
            &HashSet::from([subject.id]),
            None,
            true,
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
        let mut generation =
            receipt(project.node_id, gemini::ID, "gemini-3.1-flash-image", 1_024, 1_024);
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
    fn unchanged_poll_skips_four_thousand_artificially_slow_receipts() {
        let project = TestProject::new(500_000_000);
        std::fs::create_dir_all(project.root.join(SPEND_DIR).join("reservations")).unwrap();
        let receipts: Vec<_> = (0..4_000)
            .map(|_| {
                let mut generation =
                    receipt(project.node_id, gemini::ID, "gemini-3.1-flash-image", 1_024, 1_024);
                generation.params.insert("estimatedCostUsdMicros".into(), json!(67_000));
                generation
            })
            .collect();
        let opened = std::cell::Cell::new(0_usize);
        let rebuild_started = std::time::Instant::now();
        let rebuilt = read_cached_spend_status_locked_with(&project.root, || {
            for _ in &receipts {
                opened.set(opened.get() + 1);
                // Models the fixed per-file cost of opening a receipt over a
                // shared mount without making the test depend on one.
                std::thread::sleep(Duration::from_micros(25));
            }
            Ok((Some(500_000_000), receipts))
        })
        .unwrap();
        let rebuild_elapsed = rebuild_started.elapsed();
        assert_eq!(opened.get(), 4_000);
        assert_eq!(rebuilt.spent_usd_micros, 268_000_000);

        opened.set(0);
        let poll_started = std::time::Instant::now();
        let unchanged = read_cached_spend_status_locked_with(&project.root, || {
            opened.set(usize::MAX);
            panic!("an unchanged poll reopened the canonical receipt ledger")
        })
        .unwrap();
        let poll_elapsed = poll_started.elapsed();

        assert_eq!(unchanged.spent_usd_micros, rebuilt.spent_usd_micros);
        assert_eq!(opened.get(), 0, "the cached poll opened no receipt files");
        assert!(
            poll_elapsed < rebuild_elapsed,
            "cached poll {poll_elapsed:?} did not beat artificial receipt latency {rebuild_elapsed:?}",
        );
    }

    #[test]
    fn cache_loss_reconstructs_from_canonical_receipts() {
        let project = TestProject::new(200_000);
        let mut generation =
            receipt(project.node_id, gemini::ID, "gemini-3.1-flash-image", 1_024, 1_024);
        generation.params.insert("estimatedCostUsdMicros".into(), json!(67_000));
        let mut store = Project::open(&project.root).unwrap();
        store.record_generation(generation).unwrap();
        drop(store);

        let aggregate = project.root.join(SPEND_DIR).join(SPEND_AGGREGATE);
        let _ = std::fs::remove_file(&aggregate);
        let rebuilt = spend_status_for_report(&project.root).unwrap();
        assert_eq!(rebuilt.spent_usd_micros, 67_000);
        assert!(aggregate.is_file(), "the disposable aggregate was reconstructed");
    }

    #[test]
    fn malformed_canonical_receipt_fails_closed() {
        let project = TestProject::new(200_000);
        // Establish a plausible cache first. Admission must ignore it rather
        // than letting it hide a malformed receipt that arrives afterwards.
        assert_eq!(spend_status_for_report(&project.root).unwrap().spent_usd_micros, 0);
        let month = project.root.join("generations/2026-08");
        std::fs::create_dir_all(&month).unwrap();
        std::fs::write(month.join(format!("{}.json", new_id())), b"{not-json").unwrap();

        let error = spend_status_for(&project.root).unwrap_err();
        assert_eq!(error.code, Code::Malformed);
        assert!(SpendReservation::create(&project.root, 1).is_err());
    }
}
