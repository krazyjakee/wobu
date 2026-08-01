//! Preparing and running one image generation from the Inspector.
//!
//! The command resolves and negotiates before it queues anything, so bad input,
//! a missing provider, a missing key or an unreadable reference fails without
//! starting a paid job. The task owns only an immutable request and a project
//! path; it never holds the open-project mutex across a provider call.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tauri::{AppHandle, Emitter, State};
use wobu_core::{
    Asset, AssetKind, FragmentTarget, Generation, Id, InfluenceSnapshot, Node, SnapshotFragment,
    SnapshotLayer, default_preset, new_id, preset,
};
use wobu_imagine::{
    AspectRatio, ComfyBackend, Error as ImageError, GeminiBackend, ImageBackend, ImageRequest,
    ImageUsage, ProgressSink, Reference, comfy, gemini, negotiate,
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

    let plan = prepare(Prepare {
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
        seed: seed.unwrap_or_else(|| u128::from(new_id()) as u64),
        backend,
        provider,
        app,
    })?;
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
}

/// Provider-aware reference quotas and every image that negotiation withholds.
#[tauri::command]
pub fn image_reference_report(
    state: State<'_, AppState>,
    subject_id: Id,
    preset: Option<String>,
    sliders: Option<Vec<GenerateSlider>>,
    shot: Option<GenerateShot>,
    model: Option<String>,
) -> CommandResult<ImageReferenceReport> {
    let (nodes, provider, selected_model) = state.with(|project| {
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
        Ok((project.world_nodes()?.to_vec(), provider, selected_model))
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
    let sliders = Sliders::from_pairs(
        sliders.unwrap_or_default().into_iter().map(|slider| {
            (slider.node_id, if slider.muted { 0.0 } else { slider.value })
        }),
    );
    let mut extracted = fragments(&stack, chosen, &sliders);
    append_user_prompt(
        &stack,
        &mut extracted,
        controls.prompt.as_deref().map(str::trim).unwrap_or(""),
    );
    let aspect = AspectRatio::parse(chosen.aspect)
        .ok_or_else(|| WobuError::new(Code::Internal, "The preset has an invalid aspect ratio."))?;
    let negotiated = negotiate(&extracted, aspect, &backend.capabilities(&model));
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
    Ok(ImageReferenceReport { buckets, layers })
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
    let sliders = Sliders::from_pairs(slider_values.iter().copied());
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
    let batch = chosen.generations(input.seed);
    let mut plans = Vec::with_capacity(batch.len());
    for item in batch {
        let mut extracted = match item.view {
            Some(view) => fragments_for_view(&stack, chosen, &sliders, view),
            None => fragments(&stack, chosen, &sliders),
        };
        append_user_prompt(&stack, &mut extracted, user_prompt);
        let negotiated = negotiate(&extracted, requested_aspect, &caps);
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
        params.insert("batchIndex".into(), json!(item.index));
        params.insert("batchSize".into(), json!(chosen.images));
        params.insert("requestedAspect".into(), json!(requested_aspect.to_string()));
        params.insert("aspect".into(), json!(negotiated.aspect().to_string()));
        params.insert("width".into(), json!(resolution.width));
        params.insert("height".into(), json!(resolution.height));
        let request = ImageRequest::new(input.model.clone(), compiled.prompt(), item.seed, &negotiated)
            .with_negative(compiled.negative())
            .with_references(references);
        plans.push(PlannedImage {
            request,
            generation: Generation {
                id: new_id(),
                node_id: input.subject_id,
                created_at: Utc::now(),
                preset: chosen.id.to_owned(),
                view_type: item.view.map(|view| view.view_type.to_owned()),
                user_prompt: user_prompt.to_owned(),
                compiled_prompt: compiled.prompt().to_owned(),
                negative_prompt: compiled.negative().to_owned(),
                backend: input.provider.clone(),
                model: input.model.clone(),
                seed: item.seed,
                params,
                output_asset_ids: Vec::new(),
                influence_snapshot: snapshot(
                    &stack,
                    &extracted,
                    &slider_values,
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
}

struct PlannedImage {
    request: ImageRequest,
    generation: Generation,
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
            let plan = &self.plans[batch_index];
            let mut progress = JobProgress { ctx: ctx.clone(), batch_index, batch_total };
            let outcome =
                self.backend.generate(&plan.request, &mut progress, ctx.cancel()).await;
            let image = match outcome.result {
                Ok(image) => image,
                Err(ImageError::Cancelled) => return Outcome::Cancelled,
                Err(error) => {
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
            let mut generation = plan.generation.clone();
            generation.seed = image.seed.unwrap_or(generation.seed);
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
                    // Per-image, not merely at batch completion: a later failure
                    // or cancellation must not hide work already persisted.
                    let _ = self.app.emit(GENERATION_RECORDED, &ready);
                    self.completed.push(ready);
                    self.next += 1;
                }
                Ok(Err(error)) => return Outcome::failed(command_failure(error)),
                Err(error) => {
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

fn command_failure(error: WobuError) -> Failure {
    let mut failure = Failure::new(error.code.as_str(), error.message).retryable(error.retryable);
    if let Some(detail) = error.detail {
        failure = failure.with_detail(detail);
    }
    failure.billed(Billed::Unknown)
}

fn no_image_provider() -> WobuError {
    WobuError::new(
        Code::Invalid,
        "Choose an image provider in Settings before generating.",
    )
}
