//! Running one reconstruction, and writing down what came back.
//!
//! A receipt is written whether the job succeeds or fails, because a failed
//! paid attempt still cost money and a turnaround with no record of it looks
//! like one that was never tried.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use wobu_core::{Generation, Id, MeshOutput};
use wobu_imagine::{
    Error as ImageError, MeshBackend, MeshFormat, MeshRequest, MeshUsage, ProgressSink,
};
use wobu_jobs::{Billed, Failure, JobContext, JobKind, Outcome, Progress, Task};
use wobu_store::Project;

use crate::error::{Code, CommandResult, WobuError};
use crate::generate::GENERATION_RECORDED;

pub(super) struct MeshTask {
    pub(super) label: String,
    pub(super) subject_id: Id,
    pub(super) project_id: Id,
    pub(super) root: PathBuf,
    pub(super) backend: Arc<dyn MeshBackend>,
    pub(super) request: MeshRequest,
    /// The receipt as it stands before the provider answered. `created_at`,
    /// `params.outcome` and `params.meshOutput` are filled in on the way out.
    pub(super) generation: Generation,
    pub(super) turnaround_generation_ids: Vec<Id>,
    pub(super) requires_billing: bool,
    pub(super) app: AppHandle,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MeshReady {
    pub(super) subject_id: Id,
    pub(super) generation: Generation,
    pub(super) asset_id: Id,
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
                if usage.is_billed()
                    && let Err(save) = self.record_failure(&error, usage).await
                {
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
            if usage.is_billed()
                && let Err(save) = self.record_failure(&error, usage).await
            {
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
                        "asset": null}),
                );
                Outcome::done_with(ready)
            }
            Ok(Err(error)) => Outcome::failed(
                Failure::new(error.code.as_str(), error.message).billed(if usage.is_billed() {
                    Billed::Charged
                } else {
                    Billed::Nothing
                }),
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
                        "asset": null}),
                );
                Ok(())
            }
            Ok(Err(error)) => {
                Err(Failure::new(error.code.as_str(), error.message).billed(Billed::Charged))
            }
            Err(join) => {
                Err(Failure::new("internal", "The billed mesh receipt could not be saved.")
                    .with_detail(join.to_string())
                    .billed(Billed::Charged))
            }
        }
    }
}

/// Everything `persist_mesh` needs, so its argument list is a shape rather than
/// seven positional values.
pub(super) struct PersistMesh<'a> {
    pub(super) root: PathBuf,
    pub(super) project_id: Id,
    pub(super) subject_id: Id,
    pub(super) generation: Generation,
    pub(super) bytes: &'a [u8],
    pub(super) turnaround_generation_ids: Vec<Id>,
    pub(super) usage: MeshUsage,
}

/// The join #110 was missing, in one function: bytes to a content-addressed
/// GLB, and a receipt that says which mesh and which eight views it came from.
///
/// `params.meshOutput` is the whole point. `commands::mesh_concepts` reads it
/// and nothing else — a generation without it is invisible to the 3D gallery no
/// matter how many meshes are on disk, which is exactly the state the app was
/// in before this file existed.
pub(super) fn persist_mesh(input: PersistMesh<'_>) -> CommandResult<MeshReady> {
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
        serde_json::to_value(MeshOutput { asset_id: stored.asset.id, turnaround_generation_ids })
            .unwrap_or(serde_json::Value::Null),
    );
    let generation = project.record_generation(generation)?;
    Ok(MeshReady { subject_id, generation, asset_id: stored.asset.id })
}

/// "This cost money and produced nothing", written down.
///
/// Deliberately carries no `meshOutput`: a receipt naming an asset id that was
/// never stored would put an empty tile in the 3D gallery forever.
pub(super) fn persist_failed_receipt(
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

pub(super) fn mesh_failure(
    error: &ImageError,
    usage: MeshUsage,
    requires_billing: bool,
) -> Failure {
    let billed = if usage.is_billed() {
        Billed::Charged
    } else {
        match error {
            // The provider took the request and then answered with something
            // unusable. If it bills per job, it has probably billed for it.
            ImageError::Refused { .. } | ImageError::NoMesh | ImageError::NotAMesh { .. }
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
