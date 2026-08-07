//! Safe orchestration of the optional local per-entity LoRA trainer.

use std::collections::{HashMap, HashSet};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio::process::Command;
use wobu_core::{Asset, AssetRole, Id, LoraPin, Node};
use wobu_jobs::{Billed, Failure, JobContext, JobKind, Outcome, Progress, Task};
use wobu_llm::Cancel;
use wobu_store::{Project, SaveOutcome};

use crate::error::{Code, CommandResult, WobuError};
use crate::state::{AppState, Jobs, WORLD_CHANGED};

const TRAINER: &str = "wobu-lora-trainer";
const PROTOCOL: u32 = 1;
const REQUIRED_IMAGES: usize = 15;
const STAGING_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrainerModel {
    name: String,
    family: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrainerCapabilities {
    protocol: u32,
    trainer: String,
    models: Vec<TrainerModel>,
    max_inputs: usize,
    can_install_comfy: bool,
    #[serde(default)]
    installed_loras: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoraStatus {
    subject_id: Id,
    pinned_count: usize,
    invalid_pinned_count: usize,
    required_count: usize,
    eligible: bool,
    trainer_state: &'static str,
    trainer_detail: String,
    selected_model: Option<String>,
    pin: Option<LoraPin>,
    application_state: &'static str,
    application_detail: String,
}

#[derive(Debug, Clone)]
struct TrainingInput {
    // Deliberately only a manifest entry. Full-resolution bytes move directly
    // from `source` to the private staging file and never live in this task.
    asset_id: Id,
    hash: String,
    source: PathBuf,
    bytes: u64,
}

struct TrainingContext {
    root: PathBuf,
    project_id: Id,
    read_only: bool,
    subject: Node,
    initial_stamp: wobu_store::atomic::Stamp,
    provider: Option<String>,
    model: Option<String>,
    pin: Option<LoraPin>,
    pin_integrity: Option<Result<(), String>>,
    inputs: Vec<TrainingInput>,
    invalid_inputs: usize,
}

#[tauri::command]
pub async fn lora_status(state: State<'_, AppState>, subject_id: Id) -> CommandResult<LoraStatus> {
    let context = training_context(&state, subject_id)?;
    let context = tauri::async_runtime::spawn_blocking(move || verify_training_inputs(context))
        .await
        .map_err(|error| {
            WobuError::new(Code::Internal, "Pinned LoRA inputs could not be checked.")
                .with_detail(error.to_string())
        })??;
    let capabilities = trainer_capabilities().await;
    let (trainer_state, trainer_detail, model_supported) = match &capabilities {
        Ok(capabilities) if capabilities.protocol != PROTOCOL => (
            "incompatible",
            format!(
                "{} speaks protocol {}; Wobu requires {}.",
                capabilities.trainer, capabilities.protocol, PROTOCOL
            ),
            false,
        ),
        Ok(capabilities) => {
            let supported = context.model.as_deref().is_some_and(|model| {
                capabilities.models.iter().any(|candidate| candidate.name == model)
            });
            (
                "available",
                if supported {
                    format!("{} is ready for the selected model.", capabilities.trainer)
                } else {
                    "The trainer does not advertise the selected checkpoint.".to_string()
                },
                supported,
            )
        }
        Err(detail) => ("unavailable", detail.clone(), false),
    };
    let (application_state, application_detail) =
        pin_application_status(&context, capabilities.as_ref().ok());
    let enough = context.inputs.len() >= REQUIRED_IMAGES;
    let can_install = capabilities.as_ref().is_ok_and(|capabilities| {
        capabilities.protocol == PROTOCOL && capabilities.can_install_comfy
    });
    Ok(LoraStatus {
        subject_id,
        pinned_count: context.inputs.len(),
        invalid_pinned_count: context.invalid_inputs,
        required_count: REQUIRED_IMAGES,
        eligible: enough
            && model_supported
            && can_install
            && !context.read_only
            && context.provider.as_deref() == Some("comfyui"),
        trainer_state,
        trainer_detail,
        selected_model: context.model,
        pin: context.pin,
        application_state,
        application_detail,
    })
}

#[tauri::command]
pub async fn lora_train_start(
    app: AppHandle,
    state: State<'_, AppState>,
    jobs: State<'_, Jobs>,
    subject_id: Id,
) -> CommandResult<String> {
    let context = training_context(&state, subject_id)?;
    if context.read_only {
        return Err(WobuError::new(
            Code::ReadOnly,
            "This project is read-only, so LoRA weights cannot be attached.",
        ));
    }
    let capabilities = trainer_capabilities().await.map_err(|detail| {
        WobuError::new(Code::ProviderUnavailable, "The local LoRA trainer is unavailable.")
            .with_detail(detail)
    })?;
    if capabilities.protocol != PROTOCOL {
        return Err(WobuError::new(
            Code::Invalid,
            format!(
                "The local trainer speaks protocol {}; Wobu requires {PROTOCOL}.",
                capabilities.protocol
            ),
        ));
    }
    if context.provider.as_deref() != Some("comfyui") || !capabilities.can_install_comfy {
        return Err(WobuError::new(
            Code::Invalid,
            "Local LoRA training requires a trainer that can install into the selected local ComfyUI.",
        ));
    }
    let model = context.model.clone().ok_or_else(|| {
        WobuError::new(Code::Invalid, "Select a local image checkpoint before training a LoRA.")
    })?;
    let advertised =
        capabilities.models.iter().find(|candidate| candidate.name == model).ok_or_else(|| {
            WobuError::new(Code::Invalid, "The trainer does not support the selected checkpoint.")
        })?;
    if context.inputs.len() < REQUIRED_IMAGES {
        return Err(WobuError::new(
            Code::Invalid,
            format!("Pin at least {REQUIRED_IMAGES} valid full references before training."),
        ));
    }
    if capabilities.max_inputs < REQUIRED_IMAGES {
        return Err(WobuError::new(
            Code::Invalid,
            "The trainer's advertised input limit is below Wobu's safety minimum.",
        ));
    }
    let task = TrainLoraTask {
        root: context.root,
        project_id: context.project_id,
        subject_id,
        subject: context.subject,
        initial_stamp: context.initial_stamp,
        model,
        model_family: advertised.family.clone(),
        trainer: capabilities.trainer,
        inputs: context.inputs,
        max_inputs: capabilities.max_inputs,
        app,
    };
    let id = jobs.queue().submit(task);
    Ok(id.to_string())
}

fn training_context(state: &State<'_, AppState>, subject_id: Id) -> CommandResult<TrainingContext> {
    let (root, project_id, read_only, subject, stamp, assets, provider, model) =
        state.with(|project| {
            let subject = project.get_node(subject_id)?;
            let stamp = project.node_stamp(subject_id)?.ok_or_else(|| {
                WobuError::new(Code::NoSuchNode, "That entity is not in this project any more.")
            })?;
            let image =
                project.meta().providers.get("image").and_then(serde_json::Value::as_object);
            let provider = image
                .and_then(|value| value.get("provider"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let model = image
                .and_then(|value| value.get("model"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            Ok((
                project.root().to_path_buf(),
                project.id(),
                project.is_read_only(),
                subject,
                stamp,
                project.list_assets()?,
                provider,
                model,
            ))
        })?;
    let indexed: HashMap<Id, Asset> = assets.into_iter().map(|asset| (asset.id, asset)).collect();
    let mut seen = HashSet::new();
    let mut inputs = Vec::new();
    let mut invalid_inputs = 0usize;
    for link in
        subject.asset_links.iter().filter(|link| link.enabled && link.role == AssetRole::FullRef)
    {
        if !seen.insert(link.asset_id) {
            continue;
        }
        let Some(asset) = indexed.get(&link.asset_id) else {
            invalid_inputs += 1;
            continue;
        };
        inputs.push(TrainingInput {
            asset_id: asset.id,
            hash: asset.hash.clone(),
            source: root.join(&asset.rel_path),
            bytes: asset.bytes,
        });
    }
    Ok(TrainingContext {
        root,
        project_id,
        read_only,
        initial_stamp: stamp,
        provider,
        model,
        pin: subject.lora.clone(),
        pin_integrity: None,
        subject,
        inputs,
        invalid_inputs,
    })
}

fn verify_training_inputs(mut context: TrainingContext) -> CommandResult<TrainingContext> {
    let mut invalid = context.invalid_inputs;
    context.inputs.retain(|input| {
        let valid = verify_training_input(input);
        invalid += usize::from(!valid);
        valid
    });
    context.invalid_inputs = invalid;
    context.pin_integrity = context.pin.as_ref().map(|pin| {
        let expected = wobu_core::asset::lora_path(&pin.hash)
            .ok_or_else(|| "The pin has an invalid content hash.".to_string())?;
        if pin.rel_path != expected {
            return Err("The pin path does not match its content hash.".into());
        }
        if !pin.strength.is_finite() || !(0.0..=2.0).contains(&pin.strength) {
            return Err("The LoRA strength is invalid.".into());
        }
        let path = context.root.join(&pin.rel_path);
        let metadata = regular_file(&path)?;
        if metadata.len() != pin.bytes {
            return Err("The project-owned weight file has the wrong size.".into());
        }
        let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
        if wobu_store::atomic::hash_bytes(&bytes) != pin.hash {
            return Err("The project-owned weight file failed its content hash check.".into());
        }
        wobu_store::lora::validate(&bytes).map_err(|error| error.to_string())
    });
    Ok(context)
}

fn verify_training_input(input: &TrainingInput) -> bool {
    if !regular_file(&input.source).is_ok_and(|metadata| metadata.len() == input.bytes) {
        return false;
    }
    let Ok(mut source) = std::fs::File::open(&input.source) else {
        return false;
    };
    let mut sink = io::sink();
    copy_hashed(&mut source, &mut sink, &Cancel::new()).is_ok_and(|hash| hash == input.hash)
}

fn pin_application_status(
    context: &TrainingContext,
    capabilities: Option<&TrainerCapabilities>,
) -> (&'static str, String) {
    let Some(pin) = &context.pin else {
        return ("none", "No trained LoRA is attached to this entity.".into());
    };
    if let Some(Err(detail)) = &context.pin_integrity {
        let state = if detail.contains("No such file") || detail.contains("not found") {
            "weight_missing"
        } else {
            "weight_corrupt"
        };
        return (state, detail.clone());
    }
    if context.provider.as_deref() != Some("comfyui") {
        return (
            "provider_unsupported",
            "The selected image provider cannot apply local LoRA weights.".into(),
        );
    }
    if context.model.as_deref() != Some(pin.base_model.as_str()) {
        return (
            "model_mismatch",
            format!("This LoRA was trained for {}, not the selected checkpoint.", pin.base_model),
        );
    }
    let Some(capabilities) = capabilities else {
        return (
            "not_installed",
            "The local trainer cannot confirm the ComfyUI installation.".into(),
        );
    };
    if !capabilities.installed_loras.iter().any(|name| name == &pin.provider_name) {
        return (
            "not_installed",
            "The project weights exist, but this ComfyUI has not installed them.".into(),
        );
    }
    (
        "ready",
        format!(
            "{} will be applied automatically at strength {:.2}.",
            pin.provider_name, pin.strength
        ),
    )
}

async fn trainer_capabilities() -> Result<TrainerCapabilities, String> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(TRAINER)
            .args(["capabilities", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "The local trainer did not answer within five seconds.".to_string())?
    .map_err(|error| format!("{TRAINER} could not be started: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("The local trainer refused its capability probe: {}", detail.trim()));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("The local trainer returned invalid capability JSON: {error}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest<'a> {
    protocol: u32,
    subject_id: Id,
    subject_name: &'a str,
    base_model: &'a str,
    model_family: &'a str,
    trigger_token: &'a str,
    images: Vec<ManifestImage>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestImage {
    asset_id: Id,
    hash: String,
    path: String,
}

struct StageRequest {
    subject_id: Id,
    subject_name: String,
    model: String,
    model_family: String,
    inputs: Vec<TrainingInput>,
    max_inputs: usize,
}

struct StagedTraining {
    manifest_path: PathBuf,
    output_path: PathBuf,
    input_hashes: Vec<String>,
}

enum StageError {
    Cancelled,
    Failed(WobuError),
}

impl From<WobuError> for StageError {
    fn from(error: WobuError) -> Self {
        StageError::Failed(error)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrainerResult {
    provider_name: String,
    base_model: String,
    model_family: String,
    trigger_token: String,
}

struct TrainLoraTask {
    root: PathBuf,
    project_id: Id,
    subject_id: Id,
    subject: Node,
    initial_stamp: wobu_store::atomic::Stamp,
    model: String,
    model_family: String,
    trainer: String,
    inputs: Vec<TrainingInput>,
    max_inputs: usize,
    app: AppHandle,
}

#[async_trait]
impl Task for TrainLoraTask {
    fn kind(&self) -> JobKind {
        JobKind::TrainLora
    }

    fn subject_id(&self) -> Option<String> {
        Some(self.subject_id.to_string())
    }

    fn label(&self) -> String {
        format!("Train LoRA for {}", self.subject.name)
    }

    async fn run(&mut self, ctx: &JobContext) -> Outcome {
        let stage = self.root.join(".wobu").join("tmp").join(format!("lora-{}", ctx.id()));
        let trigger = format!("wobu_{}", self.subject_id.to_string()[..8].to_ascii_lowercase());
        let request = StageRequest {
            subject_id: self.subject_id,
            subject_name: self.subject.name.clone(),
            model: self.model.clone(),
            model_family: self.model_family.clone(),
            // Cheap metadata clone for the blocking worker; no image body is
            // present in `TrainingInput` to duplicate.
            inputs: self.inputs.clone(),
            max_inputs: self.max_inputs,
        };
        let worker_stage = stage.clone();
        let worker_trigger = trigger.clone();
        let cancel = ctx.cancel().clone();
        let staged = tauri::async_runtime::spawn_blocking(move || {
            stage_training(&worker_stage, request, &worker_trigger, &cancel)
        })
        .await;
        let staged = match staged {
            Ok(Ok(staged)) => staged,
            Ok(Err(StageError::Cancelled)) => return Outcome::Cancelled,
            Ok(Err(StageError::Failed(error))) => return Outcome::failed(local_failure(error)),
            Err(error) => {
                let _ = std::fs::remove_dir_all(&stage);
                return Outcome::failed(
                    Failure::new("internal", "The LoRA dataset could not be staged.")
                        .with_detail(error.to_string()),
                );
            }
        };
        if ctx.is_cancelled() {
            let _ = std::fs::remove_dir_all(&stage);
            return Outcome::Cancelled;
        }
        let result = self.run_trainer(ctx, &staged.manifest_path, &staged.output_path).await;
        let outcome = match result {
            Ok(result) => {
                self.publish(result, &staged.output_path, &trigger, staged.input_hashes).await
            }
            Err(TrainRunError::Cancelled) => Outcome::Cancelled,
            Err(TrainRunError::Failed(detail)) => Outcome::failed(
                Failure::new("lora.trainer_failed", "The local LoRA trainer failed.")
                    .with_detail(detail)
                    .billed(Billed::Nothing),
            ),
        };
        let _ = std::fs::remove_dir_all(&stage);
        outcome
    }
}

impl TrainLoraTask {
    async fn run_trainer(
        &self,
        ctx: &JobContext,
        manifest: &Path,
        output: &Path,
    ) -> Result<TrainerResult, TrainRunError> {
        let mut child = Command::new(TRAINER)
            .arg("train")
            .arg("--manifest")
            .arg(manifest)
            .arg("--output")
            .arg(output)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| TrainRunError::Failed(error.to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TrainRunError::Failed("trainer stdout was unavailable".into()))?;
        let mut stdout = BufReader::with_capacity(8 * 1024, stdout);
        let mut result = None;
        loop {
            let next = tokio::select! {
                _ = ctx.cancel().cancelled() => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    return Err(TrainRunError::Cancelled);
                }
                line = bounded_line(&mut stdout) => line.map_err(TrainRunError::Failed)?,
            };
            let Some(line) = next else { break };
            let value: serde_json::Value = serde_json::from_str(&line).map_err(|error| {
                TrainRunError::Failed(format!("invalid progress JSON: {error}"))
            })?;
            match value.get("type").and_then(serde_json::Value::as_str) {
                Some("progress") => {
                    let done = value
                        .get("done")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                        .unwrap_or(0);
                    let total = value
                        .get("total")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                        .unwrap_or(100);
                    let note = value
                        .get("note")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("Training locally");
                    ctx.progress(Progress::new(done, total).with_note(note));
                }
                Some("result") => {
                    result = Some(serde_json::from_value(value).map_err(|error| {
                        TrainRunError::Failed(format!("invalid result JSON: {error}"))
                    })?);
                }
                Some(other) => {
                    return Err(TrainRunError::Failed(format!("unknown protocol event {other}")));
                }
                None => return Err(TrainRunError::Failed("protocol event has no type".into())),
            }
        }
        let status =
            child.wait().await.map_err(|error| TrainRunError::Failed(error.to_string()))?;
        if !status.success() {
            return Err(TrainRunError::Failed(format!("trainer exited with {status}")));
        }
        result.ok_or_else(|| TrainRunError::Failed("trainer exited without a result".into()))
    }

    async fn publish(
        &self,
        result: TrainerResult,
        output: &Path,
        trigger: &str,
        input_hashes: Vec<String>,
    ) -> Outcome {
        if result.base_model != self.model
            || result.model_family != self.model_family
            || result.trigger_token != trigger
        {
            return Outcome::failed(Failure::new(
                "lora.protocol",
                "The trainer result does not match its manifest.",
            ));
        }
        if !safe_provider_name(&result.provider_name) {
            return Outcome::failed(Failure::new(
                "lora.protocol",
                "The trainer returned an unsafe provider filename.",
            ));
        }
        let metadata = match regular_file(output) {
            Ok(metadata) => metadata,
            Err(detail) => {
                return Outcome::failed(
                    Failure::new(
                        "lora.invalid_output",
                        "The trainer did not produce a regular weight file.",
                    )
                    .with_detail(detail),
                );
            }
        };
        if metadata.len() == 0 || metadata.len() > wobu_store::lora::MAX_WEIGHT_BYTES {
            return Outcome::failed(Failure::new(
                "lora.invalid_output",
                "The trainer weight file is empty or over 2 GB.",
            ));
        }
        let root = self.root.clone();
        let project_id = self.project_id;
        let subject_id = self.subject_id;
        let mut initial_subject = self.subject.clone();
        let initial_stamp = self.initial_stamp.clone();
        let trainer = self.trainer.clone();
        let model = self.model.clone();
        let family = self.model_family.clone();
        let output = output.to_path_buf();
        let app = self.app.clone();
        let saved = tauri::async_runtime::spawn_blocking(move || -> CommandResult<LoraPin> {
            let bytes = std::fs::read(&output).map_err(|error| {
                WobuError::new(Code::Io, "The trained weights could not be read.")
                    .with_detail(error.to_string())
            })?;
            wobu_store::lora::validate(&bytes)?;
            let hash = blake3::hash(&bytes).to_hex().to_string();
            let (path, _) = wobu_store::lora::publish(&root, &hash, &bytes)?;
            let rel_path =
                path.strip_prefix(&root).map(wobu_store::paths::to_rel_string).map_err(|_| {
                    WobuError::new(
                        Code::Internal,
                        "The LoRA destination escaped the project folder.",
                    )
                })?;
            let pin = LoraPin {
                hash,
                rel_path,
                bytes: bytes.len() as u64,
                trainer,
                protocol: PROTOCOL,
                base_model: model,
                model_family: family,
                provider_name: result.provider_name,
                trigger_token: result.trigger_token,
                input_asset_hashes: input_hashes,
                created_at: Utc::now(),
                strength: 0.8,
            };
            let mut project = Project::open(&root)?;
            if project.id() != project_id {
                return Err(WobuError::new(
                    Code::Invalid,
                    "The project at this location changed during training.",
                ));
            }
            if initial_subject.id != subject_id {
                return Err(WobuError::new(
                    Code::Internal,
                    "The training subject identity changed in memory.",
                ));
            }
            initial_subject.lora = Some(pin.clone());
            match project.save_node_expected(initial_subject, &initial_stamp)? {
                SaveOutcome::Saved(_) => Ok(pin),
                SaveOutcome::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
            }
        })
        .await;
        match saved {
            Ok(Ok(pin)) => {
                let _ = app.emit(WORLD_CHANGED, ());
                Outcome::done_with(pin)
            }
            Ok(Err(error)) => Outcome::failed(local_failure(error)),
            Err(error) => Outcome::failed(
                Failure::new("internal", "The trained LoRA could not be published.")
                    .with_detail(error.to_string()),
            ),
        }
    }
}

#[derive(Debug)]
enum TrainRunError {
    Cancelled,
    Failed(String),
}

fn stage_training(
    stage: &Path,
    request: StageRequest,
    trigger: &str,
    cancel: &Cancel,
) -> Result<StagedTraining, StageError> {
    stage_training_with(stage, request, trigger, cancel, |input| {
        std::fs::File::open(&input.source).map(|file| Box::new(file) as Box<dyn io::Read + Send>)
    })
}

fn stage_training_with(
    stage: &Path,
    request: StageRequest,
    trigger: &str,
    cancel: &Cancel,
    mut open_source: impl FnMut(&TrainingInput) -> io::Result<Box<dyn io::Read + Send>>,
) -> Result<StagedTraining, StageError> {
    let staged = stage_training_inner(stage, &request, trigger, cancel, &mut open_source);
    if staged.is_err() {
        let _ = std::fs::remove_dir_all(stage);
    }
    staged
}

fn stage_training_inner(
    stage: &Path,
    request: &StageRequest,
    trigger: &str,
    cancel: &Cancel,
    open_source: &mut impl FnMut(&TrainingInput) -> io::Result<Box<dyn io::Read + Send>>,
) -> Result<StagedTraining, StageError> {
    if cancel.is_cancelled() {
        return Err(StageError::Cancelled);
    }
    std::fs::create_dir_all(stage).map_err(|error| {
        WobuError::new(Code::Io, "The LoRA staging folder could not be created.")
            .with_detail(error.to_string())
    })?;
    secure_dir(stage)?;
    let input_dir = stage.join("inputs");
    std::fs::create_dir(&input_dir).map_err(|error| {
        WobuError::new(Code::Io, "The LoRA input staging folder could not be created.")
            .with_detail(error.to_string())
    })?;
    secure_dir(&input_dir)?;
    let mut images = Vec::with_capacity(request.max_inputs.min(request.inputs.len()));
    for input in &request.inputs {
        if images.len() == request.max_inputs {
            break;
        }
        if cancel.is_cancelled() {
            return Err(StageError::Cancelled);
        }
        let Ok(metadata) = regular_file(&input.source) else { continue };
        if metadata.len() == 0 || metadata.len() != input.bytes {
            continue;
        }
        let Ok(mut source) = open_source(input) else { continue };
        let extension =
            input.source.extension().and_then(|value| value.to_str()).unwrap_or("image");
        let staged = input_dir.join(format!("{}.{}", input.hash, extension));
        let mut file =
            std::fs::OpenOptions::new().write(true).create_new(true).open(&staged).map_err(
                |error| {
                    WobuError::new(Code::Io, "A private training copy could not be created.")
                        .with_detail(error.to_string())
                },
            )?;
        secure_file(&staged)?;
        let hash = match copy_hashed(&mut source, &mut file, cancel) {
            Ok(hash) => hash,
            Err(CopyError::Cancelled) => return Err(StageError::Cancelled),
            Err(CopyError::Read) => {
                drop(file);
                let _ = std::fs::remove_file(&staged);
                continue;
            }
            Err(CopyError::Write(error)) => {
                return Err(WobuError::new(
                    Code::Io,
                    "A private training copy could not be written.",
                )
                .with_detail(error.to_string())
                .into());
            }
        };
        if hash != input.hash {
            drop(file);
            let _ = std::fs::remove_file(&staged);
            continue;
        }
        file.sync_all().map_err(|error| {
            WobuError::new(Code::Io, "A private training copy could not be synced.")
                .with_detail(error.to_string())
        })?;
        drop(file);
        images.push(ManifestImage {
            asset_id: input.asset_id,
            hash: input.hash.clone(),
            path: staged.to_string_lossy().into_owned(),
        });
    }
    if images.len() < REQUIRED_IMAGES {
        return Err(WobuError::new(
            Code::Invalid,
            format!("Pin at least {REQUIRED_IMAGES} valid full references before training."),
        )
        .into());
    }
    if cancel.is_cancelled() {
        return Err(StageError::Cancelled);
    }
    let input_hashes = images.iter().map(|image| image.hash.clone()).collect();
    let manifest = Manifest {
        protocol: PROTOCOL,
        subject_id: request.subject_id,
        subject_name: &request.subject_name,
        base_model: &request.model,
        model_family: &request.model_family,
        trigger_token: trigger,
        images,
    };
    let manifest_path = stage.join("manifest.json");
    let encoded = serde_json::to_vec_pretty(&manifest).map_err(|error| {
        WobuError::new(Code::Internal, "The LoRA manifest could not be encoded.")
            .with_detail(error.to_string())
    })?;
    let mut manifest_file =
        std::fs::OpenOptions::new().write(true).create_new(true).open(&manifest_path).map_err(
            |error| {
                WobuError::new(Code::Io, "The LoRA manifest could not be staged.")
                    .with_detail(error.to_string())
            },
        )?;
    secure_file(&manifest_path)?;
    manifest_file.write_all(&encoded).map_err(|error| {
        WobuError::new(Code::Io, "The LoRA manifest could not be written.")
            .with_detail(error.to_string())
    })?;
    manifest_file.sync_all().map_err(|error| {
        WobuError::new(Code::Io, "The LoRA manifest could not be synced.")
            .with_detail(error.to_string())
    })?;
    if cancel.is_cancelled() {
        return Err(StageError::Cancelled);
    }
    Ok(StagedTraining {
        manifest_path,
        output_path: stage.join("output.safetensors"),
        input_hashes,
    })
}

#[derive(Debug)]
enum CopyError {
    Cancelled,
    Read,
    Write(io::Error),
}

fn copy_hashed(
    source: &mut impl io::Read,
    destination: &mut impl io::Write,
    cancel: &Cancel,
) -> Result<String, CopyError> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; STAGING_BUFFER_BYTES];
    loop {
        if cancel.is_cancelled() {
            return Err(CopyError::Cancelled);
        }
        let read = source.read(&mut buffer).map_err(|_| CopyError::Read)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        destination.write_all(&buffer[..read]).map_err(CopyError::Write)?;
    }
    if cancel.is_cancelled() {
        return Err(CopyError::Cancelled);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

async fn bounded_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Option<String>, String> {
    const MAX: usize = 64 * 1024;
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(|error| error.to_string())?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                String::from_utf8(line)
                    .map(Some)
                    .map_err(|_| "trainer protocol was not UTF-8".into())
            };
        }
        let take =
            available.iter().position(|byte| *byte == b'\n').map_or(available.len(), |at| at + 1);
        if line.len().saturating_add(take) > MAX {
            return Err("trainer emitted an oversized protocol line".into());
        }
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        if line.last() == Some(&b'\n') {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return String::from_utf8(line)
                .map(Some)
                .map_err(|_| "trainer protocol was not UTF-8".into());
        }
    }
}

#[cfg(unix)]
fn secure_dir(path: &Path) -> CommandResult<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(|error| {
        WobuError::new(Code::Io, "The LoRA staging folder could not be made private.")
            .with_detail(error.to_string())
    })
}

#[cfg(not(unix))]
fn secure_dir(_path: &Path) -> CommandResult<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_file(path: &Path) -> CommandResult<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|error| {
        WobuError::new(Code::Io, "A LoRA staging file could not be made private.")
            .with_detail(error.to_string())
    })
}

#[cfg(not(unix))]
fn secure_file(_path: &Path) -> CommandResult<()> {
    Ok(())
}

fn regular_file(path: &Path) -> Result<std::fs::Metadata, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err("path is not a regular non-symlink file".into());
    }
    Ok(metadata)
}

fn safe_provider_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && name.ends_with(".safetensors")
        && !name.starts_with('/')
        && !name.contains('\\')
        && name.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

fn local_failure(error: WobuError) -> Failure {
    let mut failure = Failure::new(error.code.as_str(), error.message).billed(Billed::Nothing);
    if let Some(detail) = error.detail {
        failure = failure.with_detail(detail);
    }
    failure
}
#[cfg(test)]
mod tests;
