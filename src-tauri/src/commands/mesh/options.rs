//! What this machine could reconstruct a mesh with, and whether it can now.
//!
//! Read before `mesh_start` so the sheet can say *why* it is not offering to
//! reconstruct — no provider chosen, no key, a region the account cannot reach
//! — rather than failing at the moment the user commits to a paid job.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;
use wobu_core::Id;
use wobu_imagine::{
    ComfyMeshBackend, DEFAULT_FACE_COUNT, Error as ImageError, FACE_COUNT, HunyuanBackend,
    MeshBackend, MeshCapabilities, View, comfy, tencent,
};

use crate::error::{Code, CommandResult, WobuError};
use crate::keys::Keys;
use crate::machine::MachineSettings;
use crate::state::AppState;

use super::{MESH_CAPABILITY, TENCENT_SECRET_ID, TENCENT_SECRET_KEY};
use crate::commands::providers::ProviderChoice;

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
pub async fn mesh_options(
    state: State<'_, AppState>,
    keys: State<'_, Keys>,
    machine: State<'_, MachineSettings>,
) -> CommandResult<MeshOptions> {
    let selection = state.with(|project| Ok(ProviderChoice::of(project, MESH_CAPABILITY)))?;
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
    let (ready, detail) = mesh_readiness(&selection, &keys, &machine).await?;
    Ok(MeshOptions {
        provider: Some(selection.provider.clone()),
        label: label.to_owned(),
        model,
        region: selection.setting("region"),
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

/// Label, resolved model and capabilities, with no credential involved.
///
/// The model is resolved by [`ProviderChoice::model`], the same call image
/// generation makes: 3D has no per-request model override, so it passes `None`
/// and takes the project's pin or the adapter's default.
pub(super) fn describe_backend(
    selection: &ProviderChoice,
) -> CommandResult<(&'static str, String, MeshCapabilities)> {
    match selection.provider.as_str() {
        tencent::ID => {
            let model = selection.model(None, tencent::DEFAULT_MODEL);
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
            let model = selection.model(None, wobu_imagine::comfy_mesh::DEFAULT_MODEL);
            let backend =
                ComfyMeshBackend::new("http://127.0.0.1:8188").map_err(provider_unavailable)?;
            let caps = backend.capabilities(&model);
            Ok((wobu_imagine::comfy_mesh::LABEL, model, caps))
        }
        other => {
            Err(WobuError::new(Code::Invalid, format!("This build has no 3D adapter for {other}.")))
        }
    }
}

fn region_of(selection: &ProviderChoice) -> tencent::Region {
    selection
        .setting("region")
        .as_deref()
        .and_then(tencent::Region::parse)
        .unwrap_or(tencent::Region::ApSingapore)
}

async fn mesh_readiness(
    selection: &ProviderChoice,
    keys: &Keys,
    machine: &MachineSettings,
) -> CommandResult<(bool, String)> {
    match selection.provider.as_str() {
        tencent::ID => {
            let has_id = keys.secret(TENCENT_SECRET_ID).await?.is_some();
            let has_key = keys.secret(TENCENT_SECRET_KEY).await?.is_some();
            if has_id && has_key {
                Ok((true, String::new()))
            } else {
                Ok((
                    false,
                    "Tencent Hunyuan3D is selected for 3D, but this machine is missing its \
                     SecretId/SecretKey pair. Add both in Settings."
                        .into(),
                ))
            }
        }
        comfy::ID => match machine.comfy_mesh() {
            Ok(_) => Ok((true, String::new())),
            Err(error) => Ok((false, error.to_string())),
        },
        other => Ok((false, format!("This build has no 3D adapter for {other}."))),
    }
}

fn provider_unavailable(error: ImageError) -> WobuError {
    WobuError::new(Code::ProviderUnavailable, error.to_string())
}

/// The real backend, with this machine's credential.
pub(super) async fn execution_backend(
    selection: &ProviderChoice,
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
            let secret_id = keys.secret(TENCENT_SECRET_ID).await?.ok_or_else(missing)?;
            let secret_key = keys.secret(TENCENT_SECRET_KEY).await?.ok_or_else(missing)?;
            let credentials = tencent::Credentials::new(
                secret_id.expose(),
                tencent::SecretKey::new(secret_key.expose()),
            );
            let backend = HunyuanBackend::new(credentials, region_of(selection))
                .map_err(provider_unavailable)?;
            Ok(Arc::new(backend))
        }
        comfy::ID => Ok(Arc::new(machine.comfy_mesh().map_err(provider_unavailable)?)),
        other => {
            Err(WobuError::new(Code::Invalid, format!("This build has no 3D adapter for {other}.")))
        }
    }
}

/* ── starting one ─────────────────────────────────────────────────────────── */

/// One reviewed view, resolved to the file it came from.
#[derive(Debug)]
pub(super) struct ChosenView {
    pub(super) view: View,
    pub(super) generation_id: Id,
    /// The seed that rendered this view. Carried so the mesh receipt can record
    /// the front view's, which is the only seed a mesh has any claim to.
    pub(super) seed: u64,
    pub(super) rel_path: String,
    pub(super) mime: String,
}
