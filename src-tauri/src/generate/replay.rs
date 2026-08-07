//! Re-sending a generation that already happened.
//!
//! Replay joins the chain in [`super`] at its last step and nowhere earlier: it
//! rebuilds the request from the receipt rather than compiling a new one from
//! the project, which is what makes it a replay and not a second generation
//! that happens to look similar. Anything the receipt does not record — a
//! reference since unlinked, a LoRA no longer installed — fails the replay
//! instead of being filled in from the project as it stands now.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use serde_json::{Value, json};
use tauri::{AppHandle, State};
use wobu_core::{Asset, AssetRole, FragmentTarget, Generation, Id, new_id};
use wobu_imagine::{
    AspectRatio, Capabilities, ImageBackend, ImageRequest, Reference, ReferenceMechanism,
    Resolution,
};
use wobu_influence::RefBucket;

use super::loras::{ReceiptLora, safe_lora_name, safe_trigger_token};
use super::spend::{apply_pricing_metadata, image_price};
use super::task::{PlannedBatch, PlannedImage};
use super::{BackendPurpose, ReferenceLoader, image_backend, prepare_blocking};
use crate::error::{Code, CommandResult, WobuError};
use crate::keys::Keys;
use crate::machine::MachineSettings;
use crate::state::{AppState, Jobs};

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
pub(super) fn replay_plan(
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
