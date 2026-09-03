//! Running a planned batch on the job queue.
//!
//! Step 5 of the chain in [`super`], and the only route to the queue: batch,
//! scene and replay all arrive as a [`PlannedBatch`], which is where the billing
//! flag is stated once. The task owns an immutable request and a project path —
//! it never holds the open-project mutex across a provider call.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};
use wobu_core::{Asset, AssetKind, Generation, Id};
use wobu_imagine::{Error as ImageError, ImageBackend, ImageRequest, ImageUsage, ProgressSink};
use wobu_jobs::{Billed, Failure, JobContext, JobKind, Outcome, Preview, Progress, Task};
use wobu_store::Project;

use super::GENERATION_RECORDED;
use crate::error::{Code, CommandResult, WobuError};

pub(super) struct GenerateTask {
    pub(super) label: String,
    pub(super) subject_id: Id,
    pub(super) project_id: Id,
    pub(super) root: PathBuf,
    pub(super) backend: Arc<dyn ImageBackend>,
    pub(super) plans: Vec<PlannedImage>,
    pub(super) next: usize,
    pub(super) completed: Vec<GenerateReady>,
    pub(super) app: AppHandle,
    pub(super) requires_billing: bool,
    /// Replay can legitimately outlive the node its immutable receipt names.
    pub(super) archival_replay: bool,
}

pub(super) struct PlannedImage {
    pub(super) request: ImageRequest,
    pub(super) generation: Generation,
}

/// A finished plan: every provider request and every receipt this job will
/// write, decided before anything is queued and unable to change afterwards.
///
/// This is the single output of planning. Batch generation ([`plan_batch`]),
/// scene composition ([`plan_scene`]) and replay ([`replay_plan`]) all produce
/// one, and [`PlannedBatch::into_task`] is the only way any of them reaches the
/// queue — so billing and the archival-replay exemption are stated once rather
/// than three times.
pub(super) struct PlannedBatch {
    pub(super) label: String,
    pub(super) subject_id: Id,
    pub(super) plans: Vec<PlannedImage>,
    pub(super) requires_billing: bool,
    /// Replay can legitimately outlive the node its immutable receipt names.
    pub(super) archival_replay: bool,
}

impl PlannedBatch {
    pub(super) fn into_task(
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
            archival_replay: self.archival_replay,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GenerateReady {
    pub(super) subject_id: Id,
    pub(super) generation: Generation,
    pub(super) asset: Asset,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GenerateBatchReady {
    pub(super) subject_id: Id,
    pub(super) generations: Vec<Generation>,
    pub(super) assets: Vec<Asset>,
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
                            return Outcome::failed(command_failure(save_error));
                        }
                        Err(join_error) => {
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
                    // Per-image, not merely at batch completion: a later failure
                    // or cancellation must not hide work already persisted.
                    let _ = self.app.emit(GENERATION_RECORDED, &ready);
                    self.completed.push(ready);
                    self.next += 1;
                }
                Ok(Err(error)) => {
                    return Outcome::failed(command_failure(error));
                }
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

pub(super) struct JobProgress {
    pub(super) ctx: JobContext,
    pub(super) batch_index: usize,
    pub(super) batch_total: usize,
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

fn command_failure(error: WobuError) -> Failure {
    let mut failure = Failure::new(error.code.as_str(), error.message).retryable(error.retryable);
    if let Some(detail) = error.detail {
        failure = failure.with_detail(detail);
    }
    failure.billed(Billed::Unknown)
}
