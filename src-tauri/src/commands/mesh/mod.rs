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

pub mod options;
mod task;
pub mod turnaround;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use chrono::Utc;
use serde_json::json;
use tauri::{AppHandle, State};
use wobu_core::{Asset, Generation, Id, InfluenceSnapshot, new_id};
use wobu_imagine::{
    DEFAULT_FACE_COUNT, Error as ImageError, GenerateType, MeshRequest, MeshView, Turnaround, View,
};
use wobu_store::Project;

use crate::error::{Code, CommandResult, WobuError};
use crate::keys::Keys;
use crate::machine::MachineSettings;
use crate::state::{AppState, Jobs};

use self::options::{ChosenView, describe_backend, execution_backend};
use self::task::MeshTask;
use super::blocking;
use crate::commands::providers::ProviderChoice;

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
        let selection =
            ProviderChoice::of(project, MESH_CAPABILITY).ok_or_else(no_mesh_provider)?;
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
        Ok((project.root().to_path_buf(), project.id(), subject_name, selection, chosen))
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
    if let Some(region) = selection.setting("region") {
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
