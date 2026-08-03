//! Turning a reviewed Turnaround into a mesh.
//!
//! Everything either side of this file already existed when #110 was written.
//! `wobu_core::preset` emits eight tagged views, `generate.rs` renders them,
//! `wobu_imagine::MeshBackend` submits them and downloads the result, and
//! `wobu_store` content-addresses the GLB. What nothing did was *join* those:
//! no code path anywhere in the tree ever wrote `Generation.params.meshOutput`,
//! which is the one field `commands::mesh_concepts` reads to decide that a
//! generation produced a mesh. The viewer was therefore reachable only for a
//! project whose meshes had been put there by hand.
//!
//! Three commands close that gap:
//!
//! - [`turnaround_sheet`] surfaces what a node has actually rendered, per view
//!   and per complete batch, so the review step has something to review.
//! - [`mesh_options`] answers what the selected mesh backend will accept —
//!   view count, face-count range, generate modes, and whether the call is
//!   billed — before any money moves.
//! - [`mesh_start`] validates the chosen views, builds one [`MeshRequest`] and
//!   submits a [`MeshTask`] to the same queue images use.
//!
//! ## What this file deliberately does not do
//!
//! **It does not price the job.** `mesh.rs` in `wobu-imagine` explains why:
//! Hunyuan3D bills per job and the international `Query` response omits the
//! credit figure the mainland one returns, so there is no number to read back.
//! `generate.rs`'s spend reservation is built on a per-image price and would be
//! reserving zero here, which is worse than not reserving — a ceiling that
//! silently permits an unpriced paid call is a ceiling the user believes in and
//! does not have. Instead the caller must pass `accept_cost` when the backend
//! reports [`MeshCapabilities::requires_billing`], and the receipt records the
//! job count the provider admits to.
//!
//! **It does not count concurrent jobs.** `wobu-jobs` already defaults its
//! concurrency to three and names Hunyuan3D as the reason.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};
use wobu_core::{Asset, Generation, Id, InfluenceSnapshot, MeshOutput, new_id};
use wobu_imagine::{
    ComfyMeshBackend, DEFAULT_FACE_COUNT, Error as ImageError, FACE_COUNT, GenerateType,
    HunyuanBackend, MeshBackend, MeshCapabilities, MeshFormat, MeshRequest, MeshUsage, MeshView,
    ProgressSink, Turnaround, View, comfy, tencent,
};
use wobu_jobs::{Billed, Failure, JobContext, JobKind, Outcome, Progress, Task};
use wobu_store::Project;

use crate::error::{Code, CommandResult, WobuError};
use crate::generate::GENERATION_RECORDED;
use crate::keys::Keys;
use crate::machine::MachineSettings;
use crate::state::{AppState, Jobs};

use super::blocking;

/// The keychain entries Tencent's signature needs. Two, not one: `keys.rs`
/// registers the pair separately because there is no join format both sides
/// would have to agree on, and a mis-split is an auth failure with no
/// explanation.
const TENCENT_SECRET_ID: &str = "tencent-secret-id";
const TENCENT_SECRET_KEY: &str = "tencent-secret-key";

/// The `providers` key in `project.json` this reads. The same string
/// `commands::Capability::Mesh` writes.
const MESH_CAPABILITY: &str = "mesh";

/* ── reviewing a turnaround ───────────────────────────────────────────────── */

/// One rendered take of one view.
///
/// A *take* rather than a cell of a batch, because a per-view reroll is by
/// definition a second answer for one position: the Turnaround preset locks one
/// seed across all eight views, so re-rolling the back view has to use a
/// different seed and would fall outside its own batch if batches were the only
/// unit here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnaroundTake {
    pub generation_id: Id,
    pub asset_id: Id,
    pub seed: u64,
    pub created_at: DateTime<Utc>,
    pub backend: String,
    pub model: String,
}

/// Every take for one of the eight views, newest first.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnaroundSlot {
    pub view_type: String,
    pub takes: Vec<TurnaroundTake>,
}

/// One complete eight-view run, identified by the seed the preset locked.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnaroundBatch {
    pub seed: u64,
    pub created_at: DateTime<Utc>,
    /// In `View::ALL` order, which is the order the mesh request sends them.
    pub generation_ids: Vec<Id>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnaroundSheet {
    /// Exactly eight, in `View::ALL` order, whether or not they have takes.
    pub views: Vec<TurnaroundSlot>,
    /// Complete runs, newest first. Empty is the ordinary state before the
    /// Turnaround preset has ever been generated for this entity.
    pub batches: Vec<TurnaroundBatch>,
    /// View names with nothing rendered yet, in `View::ALL` order.
    pub missing: Vec<String>,
}

/// What this entity has rendered towards a mesh.
///
/// Reads receipts only — no image bytes and no mesh bytes — so opening the 3D
/// tab over a share stays as cheap as `mesh_concepts` already is.
#[tauri::command]
pub fn turnaround_sheet(state: State<'_, AppState>, node_id: Id) -> CommandResult<TurnaroundSheet> {
    state.with(|project| Ok(sheet(&project.list_generations(node_id)?)))
}

fn sheet(generations: &[Generation]) -> TurnaroundSheet {
    let mut slots: Vec<TurnaroundSlot> = View::ALL
        .iter()
        .map(|view| TurnaroundSlot { view_type: view.to_string(), takes: Vec::new() })
        .collect();
    // Seed → the take chosen for each view of that run. A run that generated a
    // view twice (which the preset cannot do, but a hand-edited project can)
    // keeps the newest, because that is the one the sheet shows.
    let mut by_seed: HashMap<u64, HashMap<View, TurnaroundTake>> = HashMap::new();

    for generation in generations {
        let Some(view) = generation.view_type.as_deref().and_then(View::parse) else { continue };
        let Some(asset_id) = generation.output_asset_ids.first().copied() else { continue };
        let take = TurnaroundTake {
            generation_id: generation.id,
            asset_id,
            seed: generation.seed,
            created_at: generation.created_at,
            backend: generation.backend.clone(),
            model: generation.model.clone(),
        };
        let run = by_seed.entry(generation.seed).or_default();
        match run.get(&view) {
            Some(existing) if existing.created_at >= take.created_at => {}
            _ => {
                run.insert(view, take.clone());
            }
        }
        let index = View::ALL.iter().position(|candidate| *candidate == view).unwrap_or_default();
        slots[index].takes.push(take);
    }

    for slot in &mut slots {
        slot.takes.sort_by(|left, right| {
            let newest = right.created_at.cmp(&left.created_at);
            newest.then(right.generation_id.cmp(&left.generation_id))
        });
    }

    let mut batches: Vec<TurnaroundBatch> = by_seed
        .into_iter()
        .filter(|(_, run)| run.len() == View::ALL.len())
        .map(|(seed, run)| TurnaroundBatch {
            seed,
            created_at: run.values().map(|take| take.created_at).max().unwrap_or_else(Utc::now),
            generation_ids: View::ALL
                .iter()
                .map(|view| run[view].generation_id)
                .collect(),
        })
        .collect();
    batches.sort_by(|left, right| {
        right.created_at.cmp(&left.created_at).then(right.seed.cmp(&left.seed))
    });

    let missing = slots
        .iter()
        .filter(|slot| slot.takes.is_empty())
        .map(|slot| slot.view_type.clone())
        .collect();

    TurnaroundSheet { views: slots, batches, missing }
}

/* ── what the mesh backend will take ──────────────────────────────────────── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshOptions {
    /// `None` when `project.json` selects no mesh provider at all.
    pub provider: Option<String>,
    pub label: String,
    pub model: String,
    /// Hunyuan3D only, and only when the project recorded one.
    pub region: Option<String>,
    /// Including the front view. One means this backend reconstructs from a
    /// single image, which is what the local ComfyUI tier does.
    pub max_views: usize,
    pub face_count_min: u32,
    pub face_count_max: u32,
    pub default_face_count: u32,
    pub pbr: bool,
    /// In preference order, most useful first.
    pub generate_types: Vec<String>,
    /// Whether starting a job spends money. Drives the consent gate below.
    pub requires_billing: bool,
    /// Whether `mesh_start` could run right now.
    pub ready: bool,
    /// One sentence saying why not, empty when ready.
    pub detail: String,
}

/// What the selected mesh backend accepts, and whether it could run.
///
/// Capability questions are answered from a backend built for the purpose,
/// exactly as `generate::image_reference_report` builds a Gemini backend with a
/// placeholder key to read `Capabilities` without a call. Readiness is answered
/// separately, because "this model takes eight views" is true whether or not
/// this machine has a key.
#[tauri::command]
pub fn mesh_options(
    state: State<'_, AppState>,
    keys: State<'_, Keys>,
    machine: State<'_, MachineSettings>,
) -> CommandResult<MeshOptions> {
    let selection = state.with(|project| Ok(mesh_selection(project)))?;
    let Some(selection) = selection else {
        return Ok(MeshOptions {
            provider: None,
            label: String::new(),
            model: String::new(),
            region: None,
            max_views: 0,
            face_count_min: *FACE_COUNT.start(),
            face_count_max: *FACE_COUNT.end(),
            default_face_count: DEFAULT_FACE_COUNT,
            pbr: false,
            generate_types: Vec::new(),
            requires_billing: false,
            ready: false,
            detail: "This project has no 3D backend selected. Choose one in Settings.".into(),
        });
    };

    let (label, model, caps) = describe_backend(&selection)?;
    let (ready, detail) = mesh_readiness(&selection, &keys, &machine);
    Ok(MeshOptions {
        provider: Some(selection.provider.clone()),
        label: label.to_owned(),
        model,
        region: selection.region.clone(),
        max_views: caps.max_views,
        face_count_min: *caps.face_count.start(),
        face_count_max: *caps.face_count.end(),
        default_face_count: DEFAULT_FACE_COUNT
            .clamp(*caps.face_count.start(), *caps.face_count.end()),
        pbr: caps.pbr,
        generate_types: caps.generate_types.iter().map(|kind| kind.to_string()).collect(),
        requires_billing: caps.requires_billing,
        ready,
        detail,
    })
}

#[derive(Debug, Clone)]
struct MeshSelection {
    provider: String,
    model: Option<String>,
    region: Option<String>,
}

fn mesh_selection(project: &Project) -> Option<MeshSelection> {
    let selected = project.meta().providers.get(MESH_CAPABILITY)?.as_object()?;
    let text = |key: &str| {
        selected
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let provider = text("provider")?;
    Some(MeshSelection { provider, model: text("model"), region: text("region") })
}

/// Label, resolved model and capabilities, with no credential involved.
fn describe_backend(
    selection: &MeshSelection,
) -> CommandResult<(&'static str, String, MeshCapabilities)> {
    match selection.provider.as_str() {
        tencent::ID => {
            let model =
                selection.model.clone().unwrap_or_else(|| tencent::DEFAULT_MODEL.to_owned());
            // Capabilities are a pure function of the model id; the credential
            // is never used and the object never makes a call.
            let backend = HunyuanBackend::new(
                tencent::Credentials::new(
                    "capability-preview",
                    tencent::SecretKey::new("capability-preview"),
                ),
                region_of(selection),
            )
            .map_err(provider_unavailable)?;
            let caps = backend.capabilities(&model);
            Ok((tencent::LABEL, model, caps))
        }
        comfy::ID => {
            let model = selection
                .model
                .clone()
                .unwrap_or_else(|| wobu_imagine::comfy_mesh::DEFAULT_MODEL.to_owned());
            let backend =
                ComfyMeshBackend::new("http://127.0.0.1:8188").map_err(provider_unavailable)?;
            let caps = backend.capabilities(&model);
            Ok((wobu_imagine::comfy_mesh::LABEL, model, caps))
        }
        other => Err(WobuError::new(
            Code::Invalid,
            format!("This build has no 3D adapter for {other}."),
        )),
    }
}

fn region_of(selection: &MeshSelection) -> tencent::Region {
    selection
        .region
        .as_deref()
        .and_then(tencent::Region::parse)
        .unwrap_or(tencent::Region::ApSingapore)
}

fn mesh_readiness(
    selection: &MeshSelection,
    keys: &Keys,
    machine: &MachineSettings,
) -> (bool, String) {
    match selection.provider.as_str() {
        tencent::ID => {
            let has_id = keys.secret(TENCENT_SECRET_ID).is_some();
            let has_key = keys.secret(TENCENT_SECRET_KEY).is_some();
            if has_id && has_key {
                (true, String::new())
            } else {
                (
                    false,
                    "Tencent Hunyuan3D is selected for 3D, but this machine is missing its \
                     SecretId/SecretKey pair. Add both in Settings."
                        .into(),
                )
            }
        }
        comfy::ID => match machine.comfy_mesh() {
            Ok(_) => (true, String::new()),
            Err(error) => (false, error.to_string()),
        },
        other => (false, format!("This build has no 3D adapter for {other}.")),
    }
}

fn provider_unavailable(error: ImageError) -> WobuError {
    WobuError::new(Code::ProviderUnavailable, error.to_string())
}

/// The real backend, with this machine's credential.
async fn execution_backend(
    selection: &MeshSelection,
    keys: &Keys,
    machine: &MachineSettings,
) -> CommandResult<Arc<dyn MeshBackend>> {
    match selection.provider.as_str() {
        tencent::ID => {
            let missing = || {
                WobuError::new(
                    Code::ProviderNoKey,
                    "Tencent Hunyuan3D is selected for 3D, but this machine is missing its \
                     SecretId/SecretKey pair. Add both in Settings.",
                )
            };
            let secret_id = keys.secret(TENCENT_SECRET_ID).ok_or_else(missing)?;
            let secret_key = keys.secret(TENCENT_SECRET_KEY).ok_or_else(missing)?;
            let credentials = tencent::Credentials::new(
                secret_id.expose(),
                tencent::SecretKey::new(secret_key.expose()),
            );
            let backend = HunyuanBackend::new(credentials, region_of(selection))
                .map_err(provider_unavailable)?;
            Ok(Arc::new(backend))
        }
        comfy::ID => Ok(Arc::new(machine.comfy_mesh().map_err(provider_unavailable)?)),
        other => Err(WobuError::new(
            Code::Invalid,
            format!("This build has no 3D adapter for {other}."),
        )),
    }
}

/* ── starting one ─────────────────────────────────────────────────────────── */

/// One reviewed view, resolved to the file it came from.
#[derive(Debug)]
struct ChosenView {
    view: View,
    generation_id: Id,
    /// The seed that rendered this view. Carried so the mesh receipt can record
    /// the front view's, which is the only seed a mesh has any claim to.
    seed: u64,
    rel_path: String,
    mime: String,
}

/// Queue one reconstruction from reviewed turnaround views.
///
/// Everything that can be known before money moves is checked here rather than
/// in the task: the project is writable, the receipts exist and are tagged, the
/// images are on disk and are a shape the provider takes, the face count is in
/// range, the generate mode exists for this model, and — when the backend bills
/// — the caller has said so out loud.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri exposes these as named bridge arguments.
pub async fn mesh_start(
    app: AppHandle,
    state: State<'_, AppState>,
    jobs: State<'_, Jobs>,
    keys: State<'_, Keys>,
    machine: State<'_, MachineSettings>,
    node_id: Id,
    generation_ids: Vec<Id>,
    face_count: Option<u32>,
    enable_pbr: Option<bool>,
    generate_type: Option<String>,
    accept_cost: Option<bool>,
) -> CommandResult<String> {
    if generation_ids.is_empty() {
        return Err(WobuError::new(
            Code::Invalid,
            "Choose at least the front view before reconstructing.",
        ));
    }

    let (root, project_id, subject_name, selection, chosen) = state.with(|project| {
        if project.is_read_only() {
            return Err(WobuError::new(
                Code::ReadOnly,
                "This project is read-only, so a generated mesh could not be saved.",
            ));
        }
        let selection = mesh_selection(project).ok_or_else(no_mesh_provider)?;
        let subject_name = project
            .world_nodes()?
            .iter()
            .find(|node| node.id == node_id)
            .map(|node| node.name.clone())
            .ok_or_else(|| {
                WobuError::new(Code::NoSuchNode, "That entity is not in this project any more.")
            })?;
        let assets: HashMap<Id, Asset> =
            project.list_assets()?.into_iter().map(|asset| (asset.id, asset)).collect();
        let chosen = resolve_views(project, node_id, &generation_ids, &assets)?;
        Ok((
            project.root().to_path_buf(),
            project.id(),
            subject_name,
            selection,
            chosen,
        ))
    })?;

    let (_, model, caps) = describe_backend(&selection)?;
    if caps.requires_billing && !accept_cost.unwrap_or(false) {
        return Err(WobuError::new(
            Code::Invalid,
            "This 3D backend charges for every submitted job and does not report the amount \
             back, so reconstruction has to be confirmed before it starts.",
        ));
    }
    let face_count = face_count.unwrap_or(DEFAULT_FACE_COUNT);
    if !caps.face_count.contains(&face_count) {
        return Err(WobuError::new(
            Code::Invalid,
            format!(
                "This model reconstructs between {} and {} faces.",
                caps.face_count.start(),
                caps.face_count.end()
            ),
        ));
    }
    let generate_type = parse_generate_type(generate_type.as_deref())?;
    if !caps.supports(generate_type) {
        return Err(WobuError::new(
            Code::Invalid,
            format!("This model has no {generate_type} reconstruction mode."),
        ));
    }
    let enable_pbr = enable_pbr.unwrap_or(false);
    if enable_pbr && !caps.pbr {
        return Err(WobuError::new(
            Code::Invalid,
            "This 3D backend does not produce PBR materials.",
        ));
    }

    // A backend that takes fewer views than were reviewed keeps the leading
    // ones in `View::ALL` order — front first — and the receipt records exactly
    // which were sent. Silently sending eight to a single-image backend is a
    // paid call refused after the upload; silently sending one without saying
    // so is a worse mesh nothing on screen explains.
    let sent: Vec<ChosenView> = chosen.into_iter().take(caps.max_views.max(1)).collect();
    let view_names: Vec<String> = sent.iter().map(|view| view.view.to_string()).collect();
    let turnaround_generation_ids: Vec<Id> = sent.iter().map(|view| view.generation_id).collect();
    let front_seed = sent.first().map(|view| view.seed).unwrap_or_default();

    let backend = execution_backend(&selection, &keys, &machine).await?;
    let read_root = root.clone();
    let model_for_request = model.clone();
    let request = blocking("Reading the turnaround stopped unexpectedly.", move || {
        build_request(&read_root, sent, &model_for_request)
    })
    .await??
    .with_face_count(face_count)
    .with_pbr(enable_pbr)
    .with_generate_type(generate_type);

    let mut params = serde_json::Map::new();
    params.insert("faceCount".into(), json!(face_count));
    params.insert("enablePbr".into(), json!(enable_pbr));
    params.insert("generateType".into(), json!(generate_type.to_string()));
    params.insert("meshViews".into(), json!(view_names));
    // Deliberately no `estimatedCostUsdMicros`. The spend ledger reads that
    // field as a figure somebody stood behind, and zero here would be a claim
    // that a billed Hunyuan3D job was free. `billedJobs`, written when the
    // provider answers, is the only cost fact this pipeline has.
    if let Some(region) = &selection.region {
        params.insert("region".into(), json!(region));
    }

    // `SubmitHunyuanTo3DProJob` takes no seed and `Generation.seed` is not
    // optional, so this is either the seed that produced the front view or it
    // is a number nothing used.
    let seed = front_seed;
    let generation = Generation {
        id: new_id(),
        node_id,
        created_at: Utc::now(),
        preset: "turnaround".into(),
        view_type: None,
        user_prompt: String::new(),
        // 3.1 has no text-and-image conditioning path, so no prompt rides along
        // to this stage. Recording one would be a receipt claiming an input the
        // provider never saw.
        compiled_prompt: String::new(),
        negative_prompt: String::new(),
        backend: selection.provider.clone(),
        model: model.clone(),
        seed,
        params,
        output_asset_ids: Vec::new(),
        influence_snapshot: InfluenceSnapshot { layers: Vec::new() },
    };

    let task = MeshTask {
        label: format!("Mesh {subject_name}"),
        subject_id: node_id,
        project_id,
        root,
        backend,
        request,
        generation,
        turnaround_generation_ids,
        requires_billing: caps.requires_billing,
        app,
    };
    Ok(jobs.queue().submit(task).to_string())
}

fn no_mesh_provider() -> WobuError {
    let message = "This project has no 3D backend selected. Choose one in Settings.";
    WobuError::new(Code::Invalid, message)
}

fn parse_generate_type(name: Option<&str>) -> CommandResult<GenerateType> {
    let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
        return Ok(GenerateType::Normal);
    };
    [GenerateType::Normal, GenerateType::Geometry, GenerateType::LowPoly, GenerateType::Sketch]
        .into_iter()
    .find(|candidate| candidate.as_str().eq_ignore_ascii_case(name))
    .ok_or_else(|| {
        WobuError::new(Code::Invalid, "That is not a reconstruction mode.")
            .with_detail(name.to_owned())
    })
}

/// Turn chosen receipts into a de-duplicated, view-ordered slate.
///
/// Order matters twice: the provider takes exactly one image per view and
/// refuses duplicates, and a backend that accepts fewer views than were chosen
/// keeps a prefix of this list — which has to start at the front view, because
/// a single-image request *is* the front view.
fn resolve_views(
    project: &Project,
    node_id: Id,
    generation_ids: &[Id],
    assets: &HashMap<Id, Asset>,
) -> CommandResult<Vec<ChosenView>> {
    let mut by_view: HashMap<View, ChosenView> = HashMap::new();
    for generation_id in generation_ids {
        let generation = project.get_generation(*generation_id)?.ok_or_else(|| {
            WobuError::new(Code::Invalid, "That turnaround view is not in this project any more.")
                .with_detail(generation_id.to_string())
        })?;
        if generation.node_id != node_id {
            return Err(WobuError::new(
                Code::Invalid,
                "A chosen turnaround view belongs to a different entity.",
            )
            .with_detail(generation_id.to_string()));
        }
        let view = generation.view_type.as_deref().and_then(View::parse).ok_or_else(|| {
            WobuError::new(
                Code::Invalid,
                "A chosen generation is not one of the eight tagged turnaround views.",
            )
            .with_detail(generation_id.to_string())
        })?;
        if by_view.contains_key(&view) {
            return Err(WobuError::new(
                Code::Invalid,
                format!("Two takes were chosen for the {view} view; the provider accepts one."),
            ));
        }
        let asset_id = generation.output_asset_ids.first().copied().ok_or_else(|| {
            WobuError::new(
                Code::Invalid,
                format!("The {view} view has a receipt but produced no image."),
            )
        })?;
        let asset = assets.get(&asset_id).ok_or_else(|| {
            let message = "A chosen turnaround image is no longer in this project.";
            WobuError::new(Code::NoSuchAsset, message).with_detail(asset_id.to_string())
        })?;
        by_view.insert(
            view,
            ChosenView {
                view,
                generation_id: *generation_id,
                seed: generation.seed,
                rel_path: asset.rel_path.clone(),
                mime: asset.mime.clone(),
            },
        );
    }
    if !by_view.contains_key(&View::Front) {
        return Err(WobuError::new(
            Code::Invalid,
            "The front view is required: a single-image reconstruction is the front view, and a \
             multi-view one sends it first.",
        ));
    }
    Ok(View::ALL.into_iter().filter_map(|view| by_view.remove(&view)).collect())
}

/// Read the chosen images and let `wobu-imagine` judge them.
///
/// A complete eight goes through [`Turnaround`], which additionally proves the
/// combined payload fits the provider's pre-encoding cap — the one constraint
/// no single image can violate on its own and every one of them can together.
fn build_request(
    root: &std::path::Path,
    chosen: Vec<ChosenView>,
    model: &str,
) -> CommandResult<MeshRequest> {
    let mut views = Vec::with_capacity(chosen.len());
    for view in chosen {
        let path = root.join(&view.rel_path);
        let bytes = std::fs::read(&path).map_err(|error| {
            WobuError::new(Code::Io, "A chosen turnaround image could not be read.")
                .with_detail(format!("{}: {error}", view.rel_path))
        })?;
        views.push(MeshView::new(view.view, bytes, view.mime));
    }
    if views.len() == View::ALL.len() {
        let turnaround = Turnaround::new(views).map_err(unsupported)?;
        return Ok(MeshRequest::from_turnaround(model, turnaround));
    }
    Ok(MeshRequest::from_views(model, views))
}

/// A refusal from `wobu-imagine` is the user's to read, not a bug report.
///
/// `Error::Unsupported` maps to the `internal` code inside that crate because
/// there it means "the adapter was asked for something it declared it could not
/// do". Reaching it from here means the *pictures* are wrong — too small, the
/// wrong format, too heavy together — which is an ordinary thing to be told.
fn unsupported(error: ImageError) -> WobuError {
    WobuError::new(Code::Invalid, error.to_string())
}

/* ── the job ──────────────────────────────────────────────────────────────── */

struct MeshTask {
    label: String,
    subject_id: Id,
    project_id: Id,
    root: PathBuf,
    backend: Arc<dyn MeshBackend>,
    request: MeshRequest,
    /// The receipt as it stands before the provider answered. `created_at`,
    /// `params.outcome` and `params.meshOutput` are filled in on the way out.
    generation: Generation,
    turnaround_generation_ids: Vec<Id>,
    requires_billing: bool,
    app: AppHandle,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MeshReady {
    subject_id: Id,
    generation: Generation,
    asset_id: Id,
}

#[async_trait]
impl Task for MeshTask {
    fn kind(&self) -> JobKind {
        JobKind::Mesh
    }

    fn subject_id(&self) -> Option<String> {
        Some(self.subject_id.to_string())
    }

    fn label(&self) -> String {
        self.label.clone()
    }

    async fn run(&mut self, ctx: &JobContext) -> Outcome {
        let mut progress = MeshProgress { ctx: ctx.clone() };
        let outcome = self.backend.generate(&self.request, &mut progress, ctx.cancel()).await;
        let usage = outcome.usage;

        let mesh = match outcome.result {
            Ok(mesh) => mesh,
            Err(error) => {
                // Billed *and* failed is the common case here, not the rare
                // one: a mesh job is minutes long, so almost every way it can
                // go wrong happens after the money was spent. A receipt is the
                // only record the user will have of that.
                if usage.is_billed() && let Err(save) = self.record_failure(&error, usage).await {
                    return Outcome::failed(save);
                }
                if matches!(error, ImageError::Cancelled) {
                    return Outcome::Cancelled;
                }
                return Outcome::failed(mesh_failure(&error, usage, self.requires_billing));
            }
        };

        // `wobu_store::store_mesh_glb` writes a validated GLB and nothing else,
        // and `wobu_core::asset::mesh_path` names the file `.glb`. Writing an
        // OBJ archive under that name is a file whose extension lies, so this
        // stops instead — after a job that was, in Hunyuan's case, paid for,
        // which is why the receipt below still records it.
        if mesh.format != MeshFormat::Glb {
            let error = ImageError::NotAMesh {
                detail: format!(
                    "the provider returned {} and this build stores self-contained GLB only",
                    mesh.format
                ),
            };
            if usage.is_billed() && let Err(save) = self.record_failure(&error, usage).await {
                return Outcome::failed(save);
            }
            return Outcome::failed(mesh_failure(&error, usage, self.requires_billing));
        }

        let root = self.root.clone();
        let project_id = self.project_id;
        let subject_id = self.subject_id;
        let bytes = mesh.mesh.bytes;
        let turnaround_generation_ids = self.turnaround_generation_ids.clone();
        let generation = self.generation.clone();

        let saved = tauri::async_runtime::spawn_blocking(move || {
            persist_mesh(PersistMesh {
                root,
                project_id,
                subject_id,
                generation,
                bytes: &bytes,
                turnaround_generation_ids,
                usage,
            })
        })
        .await;

        match saved {
            Ok(Ok(ready)) => {
                // The same event an image emits, so the 3D gallery and the
                // Concepts history invalidate through one path rather than two.
                let _ = self.app.emit(
                    GENERATION_RECORDED,
                    json!({
                        "subjectId": ready.subject_id,
                        "generation": ready.generation,
                        "asset": null,
                    }),
                );
                Outcome::done_with(ready)
            }
            Ok(Err(error)) => Outcome::failed(
                Failure::new(error.code.as_str(), error.message)
                    .billed(if usage.is_billed() { Billed::Charged } else { Billed::Nothing }),
            ),
            Err(join) => Outcome::failed(
                Failure::new("internal", "The generated mesh could not be saved.")
                    .with_detail(join.to_string())
                    .billed(if usage.is_billed() { Billed::Charged } else { Billed::Unknown }),
            ),
        }
    }
}

impl MeshTask {
    /// Persist "this cost money and produced nothing", the way `generate.rs`
    /// does for a billed image failure.
    async fn record_failure(&self, error: &ImageError, usage: MeshUsage) -> Result<(), Failure> {
        let root = self.root.clone();
        let project_id = self.project_id;
        let app = self.app.clone();
        let subject_id = self.subject_id;
        let generation = self.generation.clone();
        let code = error.code();

        let recorded = tauri::async_runtime::spawn_blocking(move || {
            persist_failed_receipt(&root, project_id, generation, code, usage)
        })
        .await;

        match recorded {
            Ok(Ok(generation)) => {
                let _ = app.emit(
                    GENERATION_RECORDED,
                    json!({
                        "subjectId": subject_id,
                        "generation": generation,
                        "asset": null,
                    }),
                );
                Ok(())
            }
            Ok(Err(error)) => {
                Err(Failure::new(error.code.as_str(), error.message).billed(Billed::Charged))
            }
            Err(join) => Err(Failure::new("internal", "The billed mesh receipt could not be saved.")
                .with_detail(join.to_string())
                .billed(Billed::Charged)),
        }
    }
}

/// Everything `persist_mesh` needs, so its argument list is a shape rather than
/// seven positional values.
struct PersistMesh<'a> {
    root: PathBuf,
    project_id: Id,
    subject_id: Id,
    generation: Generation,
    bytes: &'a [u8],
    turnaround_generation_ids: Vec<Id>,
    usage: MeshUsage,
}

/// The join #110 was missing, in one function: bytes to a content-addressed
/// GLB, and a receipt that says which mesh and which eight views it came from.
///
/// `params.meshOutput` is the whole point. `commands::mesh_concepts` reads it
/// and nothing else — a generation without it is invisible to the 3D gallery no
/// matter how many meshes are on disk, which is exactly the state the app was
/// in before this file existed.
fn persist_mesh(input: PersistMesh<'_>) -> CommandResult<MeshReady> {
    let PersistMesh {
        root,
        project_id,
        subject_id,
        mut generation,
        bytes,
        turnaround_generation_ids,
        usage,
    } = input;
    let mut project = Project::open(&root)?;
    if project.id() != project_id {
        return Err(WobuError::new(
            Code::Invalid,
            "The project at this location changed while the mesh was generating.",
        ));
    }
    let stored = project.store_mesh_glb(bytes)?;
    generation.created_at = Utc::now();
    generation.params.insert("outcome".into(), json!("done"));
    generation.params.insert("billedJobs".into(), json!(usage.billed_jobs));
    generation.params.insert(
        "meshOutput".into(),
        serde_json::to_value(MeshOutput {
            asset_id: stored.asset.id,
            turnaround_generation_ids,
        })
        .unwrap_or(serde_json::Value::Null),
    );
    let generation = project.record_generation(generation)?;
    Ok(MeshReady { subject_id, generation, asset_id: stored.asset.id })
}

/// "This cost money and produced nothing", written down.
///
/// Deliberately carries no `meshOutput`: a receipt naming an asset id that was
/// never stored would put an empty tile in the 3D gallery forever.
fn persist_failed_receipt(
    root: &std::path::Path,
    project_id: Id,
    mut generation: Generation,
    code: &str,
    usage: MeshUsage,
) -> CommandResult<Generation> {
    let mut project = Project::open(root)?;
    if project.id() != project_id {
        return Err(WobuError::new(
            Code::Invalid,
            "The project at this location changed while the mesh was generating.",
        ));
    }
    generation.created_at = Utc::now();
    generation.params.insert("outcome".into(), json!("failed"));
    generation.params.insert("errorCode".into(), json!(code));
    generation.params.insert("billedJobs".into(), json!(usage.billed_jobs));
    Ok(project.record_generation(generation)?)
}

/// A mesh job is one call, so progress is the adapter's own step count rather
/// than a batch position.
struct MeshProgress {
    ctx: JobContext,
}

impl ProgressSink for MeshProgress {
    fn step(&mut self, done: u32, total: u32, note: Option<&str>) {
        let mut progress = Progress::new(done.min(total), total.max(1));
        if let Some(note) = note {
            progress = progress.with_note(note);
        }
        self.ctx.progress(progress);
    }

    fn preview(&mut self, _image: &str, _step: Option<u32>) {
        // Hunyuan3D sends no intermediate render, and the local graph's
        // Preview3D output is a mesh rather than a picture the image preview
        // channel could carry.
    }
}

fn mesh_failure(error: &ImageError, usage: MeshUsage, requires_billing: bool) -> Failure {
    let billed = if usage.is_billed() {
        Billed::Charged
    } else {
        match error {
            // The provider took the request and then answered with something
            // unusable. If it bills per job, it has probably billed for it.
            ImageError::Refused { .. }
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
    if usage.is_billed() {
        failure = failure.cost_note(format!(
            "{} billed job{}",
            usage.billed_jobs,
            if usage.billed_jobs == 1 { "" } else { "s" }
        ));
    }
    if let ImageError::RateLimited { retry_after: Some(wait), .. } = error {
        failure = failure.after(*wait);
    }
    failure
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(id: Id, view: Option<&str>, seed: u64, at: &str, asset: Option<Id>) -> Generation {
        Generation {
            id,
            node_id: Id::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
            created_at: at.parse::<DateTime<Utc>>().unwrap(),
            preset: "turnaround".into(),
            view_type: view.map(str::to_owned),
            user_prompt: String::new(),
            compiled_prompt: "kael".into(),
            negative_prompt: String::new(),
            backend: "comfyui".into(),
            model: "flux-dev".into(),
            seed,
            params: Default::default(),
            output_asset_ids: asset.into_iter().collect(),
            influence_snapshot: InfluenceSnapshot { layers: Vec::new() },
        }
    }

    fn full_run(seed: u64, at: &str) -> Vec<Generation> {
        View::ALL
            .into_iter()
            .map(|view| receipt(new_id(), Some(view.as_str()), seed, at, Some(new_id())))
            .collect()
    }

    #[test]
    fn an_empty_history_still_answers_with_the_eight_slots_it_is_waiting_for() {
        // The 3D tab has to be able to say *what* is missing before anything has
        // been generated; "no turnaround" and "seven of eight" are different
        // sentences and only one of them is a reroll away from reconstruction.
        let sheet = sheet(&[]);
        assert_eq!(sheet.views.len(), 8);
        assert_eq!(sheet.missing.len(), 8);
        assert_eq!(sheet.missing[0], "front");
        assert!(sheet.batches.is_empty());
    }

    #[test]
    fn a_receipt_with_no_image_is_not_a_take() {
        // A billed failure is still a receipt, and it is tagged with its view.
        // Offering it as a reconstruction input would send a paid request with
        // an asset id that resolves to nothing.
        let sheet = sheet(&[receipt(new_id(), Some("front"), 7, "2026-08-01T12:00:00Z", None)]);
        assert!(sheet.views[0].takes.is_empty());
        assert_eq!(sheet.missing.len(), 8);
    }

    #[test]
    fn only_a_complete_run_of_one_seed_is_offered_as_a_batch() {
        let mut history = full_run(11, "2026-08-01T12:00:00Z");
        history.truncate(7);
        assert!(sheet(&history).batches.is_empty(), "seven views is not a turnaround");

        let history = full_run(11, "2026-08-01T12:00:00Z");
        let sheet = sheet(&history);
        assert_eq!(sheet.batches.len(), 1);
        assert_eq!(sheet.batches[0].seed, 11);
        assert_eq!(sheet.batches[0].generation_ids.len(), 8);
        assert!(sheet.missing.is_empty());
    }

    #[test]
    fn batches_are_newest_first_and_takes_within_a_view_are_too() {
        // The sheet is what the review step reads. Newest first is what makes
        // "the batch I just generated" the default rather than the first one
        // this entity ever had.
        let mut history = full_run(11, "2026-08-01T12:00:00Z");
        history.extend(full_run(22, "2026-08-02T12:00:00Z"));
        let sheet = sheet(&history);
        assert_eq!(sheet.batches.iter().map(|b| b.seed).collect::<Vec<_>>(), [22, 11]);
        assert_eq!(sheet.views[0].takes.len(), 2);
        assert_eq!(sheet.views[0].takes[0].seed, 22);
    }

    #[test]
    fn a_rerolled_view_is_a_take_on_its_own_seed_and_not_a_batch() {
        // The whole reason takes exist. The Turnaround preset locks one seed
        // across eight views, so re-rolling the back view *must* use a
        // different one — and a design that only knew about batches would
        // either lose the reroll or invent an eight-view run out of one image.
        let mut history = full_run(11, "2026-08-01T12:00:00Z");
        history.push(receipt(new_id(), Some("back"), 99, "2026-08-03T12:00:00Z", Some(new_id())));
        let sheet = sheet(&history);
        assert_eq!(sheet.batches.len(), 1, "the reroll did not create a second run");
        let back = sheet.views.iter().find(|slot| slot.view_type == "back").unwrap();
        assert_eq!(back.takes.len(), 2);
        assert_eq!(back.takes[0].seed, 99, "the reroll is the default take");
    }

    #[test]
    fn a_view_name_from_outside_the_eight_is_ignored_rather_than_placed() {
        let ninth =
            receipt(new_id(), Some("three_quarter"), 1, "2026-08-01T12:00:00Z", Some(new_id()));
        let sheet = sheet(&[ninth]);
        assert!(sheet.views.iter().all(|slot| slot.takes.is_empty()));
    }

    #[test]
    fn the_generate_mode_names_are_the_providers_and_nothing_else_parses() {
        assert_eq!(parse_generate_type(None).unwrap(), GenerateType::Normal);
        assert_eq!(parse_generate_type(Some("  ")).unwrap(), GenerateType::Normal);
        assert_eq!(parse_generate_type(Some("Geometry")).unwrap(), GenerateType::Geometry);
        assert_eq!(parse_generate_type(Some("geometry")).unwrap(), GenerateType::Geometry);
        assert!(parse_generate_type(Some("Blockout")).is_err());
    }

    #[test]
    fn a_billed_failure_is_reported_as_charged_with_what_it_cost() {
        // The queue decides whether to retry from `Billed`. Reporting a paid
        // Hunyuan job as free would make the queue retry it on the user's card.
        let failure = mesh_failure(&ImageError::NoMesh, MeshUsage::billed(1), true);
        assert_eq!(failure.billed, Billed::Charged);
        assert_eq!(failure.cost_note.as_deref(), Some("1 billed job"));

        // Unbilled but past the point of no return on a paid backend is
        // "nobody can tell", which the queue also treats as charged.
        let unknown = mesh_failure(&ImageError::NoMesh, MeshUsage::free(), true);
        assert_eq!(unknown.billed, Billed::Unknown);

        // The local tier costs nothing, so the same error is retryable-free.
        let local = mesh_failure(&ImageError::NoMesh, MeshUsage::free(), false);
        assert_eq!(local.billed, Billed::Nothing);
        assert!(local.cost_note.is_none());
    }
}

/// The half of #110 that is a *chain* rather than a function: reviewed
/// receipts, a provider that answers, and a mesh the 3D gallery can find.
///
/// Driven against a fake [`MeshBackend`] on a real temporary project. There are
/// no Tencent credentials in this tree and a live job costs money, so what is
/// proved here is everything either side of the network: which views reach the
/// adapter and in what order, what the options do to the request, and that the
/// bytes coming back become a GLB plus the one receipt field the gallery reads.
#[cfg(test)]
mod orchestration {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use wobu_core::{AssetKind, Node, NodeKind};
    use wobu_imagine::{GeneratedMesh, MeshFile, MeshOutcome};

    use super::*;

    /// A private directory per test. `tempfile` is not a dependency of this
    /// crate, and `sync.rs`'s tests make the same call for the same reason.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wobu-mesh-{name}-{}", new_id()));
        std::fs::create_dir_all(&dir).expect("a temp directory");
        dir
    }

    /// A PNG header, which is all `image::probe` and `dimensions::read` need.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0]);
        out
    }

    /// A GLB whose header is self-consistent, which is exactly what
    /// `wobu_store::assets::validate_mesh` insists on before it writes one.
    fn glb(payload: &[u8]) -> Vec<u8> {
        let mut out = b"glTF".to_vec();
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
        while out.len() < 20 {
            out.push(0);
        }
        let len = out.len() as u32;
        out[8..12].copy_from_slice(&len.to_le_bytes());
        out
    }

    struct Scene {
        dir: PathBuf,
        project: Project,
        node: Node,
        /// View name → the generation that rendered it.
        views: HashMap<String, Id>,
    }

    /// A node with a complete, rendered turnaround behind it.
    fn scene(name: &str, size: u32) -> Scene {
        let dir = scratch(name);
        let mut project = Project::create(&dir, "Ashfall").expect("a project");
        let node = project.create_node(NodeKind::Character, "Kael", None).expect("a node");
        let mut views = HashMap::new();
        for (index, view) in View::ALL.into_iter().enumerate() {
            let asset = project
                .import_asset(&png(size, size + index as u32), AssetKind::Generated)
                .expect("an imported view");
            let generation = project
                .record_generation(turnaround_receipt(node.id, view, asset.asset.id))
                .expect("a recorded view");
            views.insert(view.to_string(), generation.id);
        }
        Scene { dir, project, node, views }
    }

    fn turnaround_receipt(node_id: Id, view: View, asset_id: Id) -> Generation {
        Generation {
            id: new_id(),
            node_id,
            created_at: Utc::now(),
            preset: "turnaround".into(),
            view_type: Some(view.to_string()),
            user_prompt: String::new(),
            compiled_prompt: "kael, ash-grey coat".into(),
            negative_prompt: String::new(),
            backend: "comfyui".into(),
            model: "flux-dev".into(),
            seed: 4_242,
            params: Default::default(),
            output_asset_ids: vec![asset_id],
            influence_snapshot: InfluenceSnapshot { layers: Vec::new() },
        }
    }

    fn assets_of(project: &Project) -> HashMap<Id, Asset> {
        project.list_assets().expect("assets").into_iter().map(|a| (a.id, a)).collect()
    }

    fn ordered(scene: &Scene) -> Vec<Id> {
        View::ALL.into_iter().map(|view| scene.views[view.as_str()]).collect()
    }

    /// Records what it was asked for and answers with whatever it was told to.
    struct FakeBackend {
        seen: Mutex<Option<MeshRequest>>,
        answer: Mutex<Option<GeneratedMesh>>,
    }

    impl FakeBackend {
        fn returning(mesh: GeneratedMesh) -> FakeBackend {
            FakeBackend { seen: Mutex::new(None), answer: Mutex::new(Some(mesh)) }
        }
    }

    #[async_trait]
    impl MeshBackend for FakeBackend {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn label(&self) -> &'static str {
            "Fake mesh backend"
        }
        fn default_model(&self) -> &'static str {
            "3.1"
        }
        fn capabilities(&self, _model: &str) -> MeshCapabilities {
            MeshCapabilities {
                max_views: 8,
                face_count: FACE_COUNT,
                pbr: true,
                generate_types: vec![GenerateType::Normal, GenerateType::Geometry],
                text_to_mesh: true,
                requires_billing: true,
            }
        }
        async fn generate(
            &self,
            request: &MeshRequest,
            _progress: &mut dyn ProgressSink,
            _cancel: &wobu_imagine::Cancel,
        ) -> MeshOutcome {
            *self.seen.lock().unwrap() = Some(request.clone());
            match self.answer.lock().unwrap().take() {
                Some(mesh) => MeshOutcome::new(MeshUsage::billed(1), Ok(mesh)),
                None => MeshOutcome::new(MeshUsage::billed(1), Err(ImageError::NoMesh)),
            }
        }
    }

    #[test]
    fn a_reviewed_turnaround_becomes_a_mesh_the_3d_gallery_can_find() {
        // The whole of #110 in one test. Before this file existed the last two
        // assertions were unreachable: nothing in the tree wrote `meshOutput`,
        // so `mesh_concepts` could never pair a stored GLB with a receipt.
        let scene = scene("full-chain", 1024);
        let assets = assets_of(&scene.project);
        let chosen = resolve_views(&scene.project, scene.node.id, &ordered(&scene), &assets)
            .expect("eight reviewed views resolve");
        let request = build_request(scene.dir.join("ashfall.wobu").as_path(), chosen, "3.1")
            .expect("a provider-ready request")
            .with_face_count(250_000)
            .with_generate_type(GenerateType::Geometry);

        let backend = FakeBackend::returning(GeneratedMesh {
            format: MeshFormat::Glb,
            mesh: MeshFile::new("model.glb", glb(b"kael")),
            extras: vec![],
            preview: None,
        });
        let outcome = tauri::async_runtime::block_on(backend.generate(
            &request,
            &mut wobu_imagine::Discard,
            &wobu_imagine::Cancel::new(),
        ));

        // What the provider was actually handed: eight images, front first, in
        // the order it names them, with the options the user chose.
        let sent = backend.seen.lock().unwrap().clone().expect("the adapter was called");
        let names: Vec<String> = sent.views().iter().map(|view| view.view.to_string()).collect();
        assert_eq!(names, View::ALL.map(|view| view.to_string()).to_vec());
        assert_eq!(sent.face_count, 250_000);
        assert_eq!(sent.generate_type, GenerateType::Geometry);
        assert!(!sent.enable_pbr, "a request nobody edited costs the provider's default");

        let mesh = outcome.result.expect("the fake answered");
        let root = scene.project.root().to_path_buf();
        let ready = persist_mesh(PersistMesh {
            root: root.clone(),
            project_id: scene.project.id(),
            subject_id: scene.node.id,
            generation: mesh_receipt(scene.node.id),
            bytes: &mesh.mesh.bytes,
            turnaround_generation_ids: ordered(&scene),
            usage: outcome.usage,
        })
        .expect("the mesh is stored and the receipt written");

        let reopened = Project::open(&root).expect("the project reopens");
        let stored = reopened.list_meshes();
        assert_eq!(stored.len(), 1, "one content-addressed GLB landed");
        assert_eq!(stored[0].id, ready.asset_id);

        // `mesh_concepts` joins exactly these two facts and nothing else.
        let output = ready.generation.mesh_output().expect("the receipt names its mesh");
        assert_eq!(output.asset_id, ready.asset_id);
        assert_eq!(output.turnaround_generation_ids, ordered(&scene));
        assert_eq!(ready.generation.params["outcome"], json!("done"));
        assert_eq!(ready.generation.params["billedJobs"], json!(1));

        std::fs::remove_dir_all(&scene.dir).ok();
    }

    fn mesh_receipt(node_id: Id) -> Generation {
        Generation {
            id: new_id(),
            node_id,
            created_at: Utc::now(),
            preset: "turnaround".into(),
            view_type: None,
            user_prompt: String::new(),
            compiled_prompt: String::new(),
            negative_prompt: String::new(),
            backend: "fake".into(),
            model: "3.1".into(),
            seed: 4_242,
            params: Default::default(),
            output_asset_ids: Vec::new(),
            influence_snapshot: InfluenceSnapshot { layers: Vec::new() },
        }
    }

    #[test]
    fn a_billed_failure_leaves_a_receipt_that_claims_no_mesh() {
        // The common case on a paid backend, because a mesh job is minutes long.
        // A receipt with a `meshOutput` here would be a permanent empty tile in
        // the 3D gallery pointing at an asset that was never written.
        let scene = scene("billed-failure", 1024);
        let recorded = persist_failed_receipt(
            scene.project.root(),
            scene.project.id(),
            mesh_receipt(scene.node.id),
            "provider.bad_response",
            MeshUsage::billed(1),
        )
        .expect("the receipt is written");

        assert_eq!(recorded.params["outcome"], json!("failed"));
        assert_eq!(recorded.params["errorCode"], json!("provider.bad_response"));
        assert_eq!(recorded.params["billedJobs"], json!(1));
        assert_eq!(recorded.mesh_output(), None);
        assert!(Project::open(scene.project.root()).unwrap().list_meshes().is_empty());

        std::fs::remove_dir_all(&scene.dir).ok();
    }

    #[test]
    fn the_reviewed_views_are_ordered_deduplicated_and_must_include_the_front() {
        let scene = scene("resolve-views", 1024);
        let assets = assets_of(&scene.project);
        let node_id = scene.node.id;

        // Chosen in any order, sent in the provider's.
        let mut shuffled = ordered(&scene);
        shuffled.reverse();
        let resolved =
            resolve_views(&scene.project, node_id, &shuffled, &assets).expect("all eight resolve");
        let names: Vec<String> = resolved.iter().map(|view| view.view.to_string()).collect();
        assert_eq!(names, View::ALL.map(|view| view.to_string()).to_vec());

        // Two takes of one view is a duplicate the provider refuses *after* the
        // upload, so it is refused here instead.
        let front = scene.views["front"];
        let duplicate = vec![front, front];
        let error = resolve_views(&scene.project, node_id, &duplicate, &assets).unwrap_err();
        assert!(error.message.contains("front"), "{}", error.message);

        // A single-image reconstruction *is* the front view.
        let no_front: Vec<Id> = ordered(&scene).into_iter().filter(|id| *id != front).collect();
        let error = resolve_views(&scene.project, node_id, &no_front, &assets).unwrap_err();
        assert!(error.message.contains("front view is required"), "{}", error.message);

        std::fs::remove_dir_all(&scene.dir).ok();
    }

    #[test]
    fn a_generation_that_is_not_a_tagged_view_cannot_be_sent_as_one() {
        // `mesh_concepts` shows an ordinary portrait beside a mesh happily
        // enough. Sending one as the `top` view would be a paid reconstruction
        // of the wrong pictures.
        let mut scene = scene("untagged", 1024);
        let asset = scene
            .project
            .import_asset(&png(900, 900), AssetKind::Generated)
            .expect("an imported portrait");
        let mut portrait = turnaround_receipt(scene.node.id, View::Front, asset.asset.id);
        portrait.preset = "character_sheet".into();
        portrait.view_type = None;
        let portrait = scene.project.record_generation(portrait).expect("a recorded portrait");

        let error = resolve_views(
            &scene.project,
            scene.node.id,
            &[portrait.id],
            &assets_of(&scene.project),
        )
        .unwrap_err();
        assert!(error.message.contains("eight tagged turnaround views"), "{}", error.message);

        std::fs::remove_dir_all(&scene.dir).ok();
    }

    #[test]
    fn the_provider_envelope_is_enforced_before_anything_is_signed_and_sent() {
        // `Turnaround::new` proves format, dimensions and the combined payload
        // cap — the last of which no single view can break and all eight can.
        // Failing here is free; failing at the provider is a signed, billed
        // call that comes back `InvalidParameterValue`.
        let scene = scene("too-small", 100);
        let assets = assets_of(&scene.project);
        let chosen = resolve_views(&scene.project, scene.node.id, &ordered(&scene), &assets)
            .expect("the receipts themselves are fine");
        let error =
            build_request(scene.project.root(), chosen, "3.1").expect_err("the images are not");
        assert_eq!(error.code, Code::Invalid, "this is the pictures, not a bug in the adapter");
        assert!(error.message.contains("128"), "{}", error.message);

        std::fs::remove_dir_all(&scene.dir).ok();
    }

    #[test]
    fn a_provider_answer_that_is_not_a_glb_is_refused_rather_than_written_as_one() {
        // `wobu_core::asset::mesh_path` names every stored mesh `.glb`. An OBJ
        // archive under that name is a file whose extension lies and a viewer
        // that reports a corrupt mesh.
        let scene = scene("not-a-glb", 1024);
        let error = persist_mesh(PersistMesh {
            root: scene.project.root().to_path_buf(),
            project_id: scene.project.id(),
            subject_id: scene.node.id,
            generation: mesh_receipt(scene.node.id),
            bytes: b"v 0 0 0\n",
            turnaround_generation_ids: ordered(&scene),
            usage: MeshUsage::billed(1),
        })
        .unwrap_err();
        assert!(error.message.to_lowercase().contains("mesh"), "{}", error.message);
        assert!(Project::open(scene.project.root()).unwrap().list_meshes().is_empty());

        std::fs::remove_dir_all(&scene.dir).ok();
    }
}
