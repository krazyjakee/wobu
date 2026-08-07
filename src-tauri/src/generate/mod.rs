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
//! 2. [`plan::resolve_seed`] decides the seed and what the receipt may claim about
//!    where it came from.
//! 3. [`plan::prepare_generation_plan`] normalizes the controls and expands the
//!    request into [`plan::VariantCell`]s — one per image, carrying its own preset,
//!    aspect, seed and slider values.
//! 4. [`batch::plan_batch`], [`scene::plan_scene`] and [`preview::reference_report_for_plan`] all read
//!    that plan. The first two produce a [`task::PlannedBatch`]; the report negotiates
//!    the same first cell the batch would send first, so the numbers on screen
//!    are the numbers that would be spent.
//! 5. [`task::PlannedBatch::into_task`] is the only route to the queue, so spend
//!    reservation and the billing flag are stated once.
//!
//! Replay deliberately joins at step 5 and nowhere earlier: it re-sends a
//! recorded request rather than compiling a new one. Mesh reconstruction
//! (`commands::mesh`) shares step 1 only — it consumes finished generations, so
//! it has no influence stack, no preset and no aspect to negotiate.

// One module per stage of the chain above. Each is `pub` because
// `tauri::generate_handler!` resolves a command through its real module path.
pub mod batch;
pub mod loras;
pub mod plan;
pub mod preview;
pub mod replay;
pub mod scene;
pub mod spend;
pub mod task;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value, json};
use tauri::{AppHandle, State};
use wobu_core::{Asset, Id, Node};
use wobu_imagine::{
    AspectRatio, Error as ImageError, GeminiBackend, ImageBackend, Reference, Resolution, comfy,
    gemini,
};
use wobu_store::Project;

use self::batch::{Prepare, prepare};
use self::loras::{LoraDowngrade, ReceiptLora};
use self::plan::{
    GenerateShot, GenerateSlider, GenerationPlanRequest, SeedIntent, SeedSource, VariantGrid,
    locked_seed_of, resolve_seed,
};
use self::spend::{Price, apply_pricing_metadata};
use self::task::GenerateTask;
use crate::commands::providers::ProviderChoice;
use crate::error::{Code, CommandResult, WobuError};
use crate::keys::Keys;
use crate::machine::MachineSettings;
use crate::state::{AppState, Jobs};

pub const GENERATION_RECORDED: &str = "generation:recorded";

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

fn selected_image_provider(project: &Project) -> CommandResult<ProviderChoice> {
    ProviderChoice::of(project, "image").ok_or_else(no_image_provider)
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

fn no_image_provider() -> WobuError {
    WobuError::new(Code::Invalid, "Choose an image provider in Settings before generating.")
}
