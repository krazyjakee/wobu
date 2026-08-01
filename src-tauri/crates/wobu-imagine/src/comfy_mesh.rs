//! Local Hunyuan3D 2.1 through an explicitly selected ComfyUI.
//!
//! This is not the local spelling of Tencent's hosted 3.1 backend. The open
//! weights are older, single-image shape reconstruction; the shipped graph is
//! geometry-only and needs the third-party `ComfyUI-Hunyuan3DWrapper` nodes.
//! Its stable provider id remains `comfyui` because that is what the mesh slot
//! in `project.json` selects, but its label and capabilities make the tier
//! difference visible. There is no code path from a missing Tencent key here.
//!
//! The adapter does not distribute the node pack or model weights. Tencent's
//! 2.1 community licence excludes the EU, UK and South Korea; Settings and
//! `docs/08-providers.md` disclose that restriction rather than downloading a
//! model a user may not be licensed to run.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::backend::ProgressSink;
use crate::comfy::{self, ComfyBackend, Installed};
use crate::comfy::socket::{self, Frame, Next};
use crate::comfy::wire::Event;
use crate::dimensions;
use crate::error::{Error, Result};
use crate::mesh::{
    FACE_COUNT, GenerateType, GeneratedMesh, MeshBackend, MeshCapabilities, MeshFile, MeshFormat,
    MeshInput, MeshOutcome, MeshRequest, MeshUsage, View,
};
use crate::Cancel;

pub const LABEL: &str = "Local Hunyuan3D 2.1 (ComfyUI)";
pub const DEFAULT_MODEL: &str = "hunyuan3d-dit-v2-1.ckpt";

const OUTPUT_NODE: &str = "5";
const REQUIRED_NODES: [&str; 5] = [
    "LoadImage",
    "Hy3D_2_1SimpleMeshGen",
    "Hy3DPostprocessMesh",
    "Hy3DExportMesh",
    "Preview3D",
];

/// A deliberately separate mesh adapter around the shared ComfyUI transport.
pub struct ComfyMeshBackend {
    comfy: ComfyBackend,
}

impl std::fmt::Debug for ComfyMeshBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComfyMeshBackend")
            .field("base", &self.comfy.base_url())
            .finish()
    }
}

impl ComfyMeshBackend {
    pub fn new(base_url: impl AsRef<str>) -> Result<ComfyMeshBackend> {
        Ok(ComfyMeshBackend { comfy: ComfyBackend::new(base_url)? })
    }

    pub async fn connect(base_url: impl AsRef<str>) -> Result<ComfyMeshBackend> {
        let backend = ComfyMeshBackend::new(base_url)?;
        let installed = backend.comfy.probe(&Cancel::new()).await?;
        verify_install(&installed, DEFAULT_MODEL)?;
        Ok(backend)
    }

    pub fn base_url(&self) -> &str {
        self.comfy.base_url()
    }

    async fn build(
        &self,
        request: &MeshRequest,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> Result<GeneratedMesh> {
        validate(request)?;
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }

        let installed = match self.comfy.installed() {
            Some(installed) => installed,
            None => self.comfy.probe(cancel).await?,
        };
        verify_install(&installed, &request.model)?;

        let view = &request.views()[0];
        validate_image(view.bytes.as_slice(), &view.mime)?;
        progress.step(0, 1, Some("uploading the front view"));
        let image_name = self
            .comfy
            .upload_image(&upload_name(&view.mime), &view.mime, &view.bytes, cancel)
            .await?;
        let graph = workflow(request, &image_name);
        let classes = classes(&graph);

        // A cached graph can finish before a socket opened after `/prompt`
        // receives its first event, so this order is part of the protocol.
        let socket = match socket::until_cancelled(socket::connect(&self.comfy.ws_url()), cancel).await {
            None => return Err(Error::Cancelled),
            Some(socket) => socket?,
        };
        let prompt_id = self.comfy.queue(graph, cancel).await?;
        match watch(socket, &prompt_id, &classes, progress, cancel).await {
            Ok(()) => {}
            Err(Error::Cancelled) => {
                // Delete first, then let `stop` re-read the queue and interrupt
                // only if ours has reached the GPU. Passing `true` here would
                // leave a still-queued mesh running after the user stopped it.
                self.comfy.stop(&prompt_id, false).await;
                return Err(Error::Cancelled);
            }
            Err(error) => return Err(error),
        }

        progress.step(0, 1, Some("fetching the finished mesh"));
        let history = self.comfy.get(&format!("history/{prompt_id}"), cancel).await?;
        let path = history_mesh_path(&history, &prompt_id, OUTPUT_NODE)?;
        let (filename, subfolder) = split_output_path(&path);
        let query = format!(
            "view?filename={}&subfolder={}&type=output",
            comfy::wire::escape(&filename),
            comfy::wire::escape(&subfolder),
        );
        let bytes = self.comfy.get(&query, cancel).await?;
        if bytes.len() < 12 || bytes.get(..4) != Some(b"glTF".as_slice()) {
            return Err(Error::NotAMesh {
                detail: format!(
                    "ComfyUI called `{filename}` a GLB, but its {} bytes have no GLB header",
                    bytes.len(),
                ),
            });
        }
        progress.step(1, 1, Some("mesh ready"));
        Ok(GeneratedMesh {
            format: MeshFormat::Glb,
            mesh: MeshFile::new(filename, bytes),
            extras: vec![],
            preview: None,
        })
    }
}

#[async_trait]
impl MeshBackend for ComfyMeshBackend {
    fn id(&self) -> &'static str {
        comfy::ID
    }

    fn label(&self) -> &'static str {
        LABEL
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    fn capabilities(&self, _model: &str) -> MeshCapabilities {
        MeshCapabilities {
            // Hunyuan3D Shape 2.1 takes one conditioning image. Tencent's
            // hosted 3.1 is the separate backend that takes the eight views.
            max_views: 1,
            face_count: FACE_COUNT.clone(),
            pbr: false,
            generate_types: vec![GenerateType::Geometry],
            text_to_mesh: false,
            requires_billing: false,
        }
    }

    async fn generate(
        &self,
        request: &MeshRequest,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> MeshOutcome {
        MeshOutcome::new(MeshUsage::free(), self.build(request, progress, cancel).await)
    }
}

fn validate(request: &MeshRequest) -> Result<()> {
    if request.model != DEFAULT_MODEL {
        return Err(Error::Unsupported {
            detail: format!(
                "{LABEL} is pinned to `{DEFAULT_MODEL}`, not `{}`; a differently named file is not proof it has the 2.1 architecture",
                request.model,
            ),
        });
    }
    if !FACE_COUNT.contains(&request.face_count) {
        return Err(Error::Unsupported {
            detail: format!("{} faces is outside {}–{}", request.face_count, FACE_COUNT.start(), FACE_COUNT.end()),
        });
    }
    if request.enable_pbr || request.generate_type != GenerateType::Geometry {
        return Err(Error::Unsupported {
            detail: format!(
                "{LABEL} currently exposes geometry-only generation; PBR texturing needs 21 GB VRAM and a different workflow"
            ),
        });
    }
    match &request.input {
        MeshInput::Views(views) if views.len() == 1 && views[0].view == View::Front => Ok(()),
        MeshInput::Views(views) if views.len() == 1 => Err(Error::Unsupported {
            detail: format!("{LABEL} takes a front view, not a {} view", views[0].view),
        }),
        MeshInput::Views(views) => Err(Error::Unsupported {
            detail: format!("{LABEL} takes one front image, not {} Turnaround views", views.len()),
        }),
        MeshInput::Prompt(_) => Err(Error::Unsupported {
            detail: format!("{LABEL} has no text-to-mesh workflow"),
        }),
    }
}

fn verify_install(installed: &Installed, model: &str) -> Result<()> {
    let missing: Vec<String> = REQUIRED_NODES
        .iter()
        .filter(|class| !installed.has_class(class))
        .map(|class| (*class).to_owned())
        .collect();
    if !missing.is_empty() {
        return Err(missing_nodes(&missing));
    }
    if !installed.mesh_models().iter().any(|installed| installed == model) {
        return Err(no_such_model(model, installed.mesh_models()));
    }
    Ok(())
}

fn validate_image(bytes: &[u8], labelled_mime: &str) -> Result<()> {
    if dimensions::read(bytes).is_none() {
        return Err(Error::Unsupported { detail: "the local mesh input is not a readable image".into() });
    }
    let actual = dimensions::mime(bytes);
    if actual != "image/png" && actual != "image/jpeg" {
        return Err(Error::Unsupported {
            detail: format!("{LABEL} accepts a PNG or JPEG front view, not {actual}"),
        });
    }
    if !actual.eq_ignore_ascii_case(labelled_mime) {
        return Err(Error::Unsupported {
            detail: format!("the front view is labelled {labelled_mime}, but its bytes are {actual}"),
        });
    }
    Ok(())
}

fn workflow(request: &MeshRequest, image_name: &str) -> Map<String, Value> {
    json!({
        "1": {"class_type": "LoadImage", "inputs": {"image": image_name}},
        "2": {"class_type": "Hy3D_2_1SimpleMeshGen", "inputs": {
            "model": request.model,
            "image": ["1", 0],
            "steps": 50,
            "guidance_scale": 5.0,
            "octree_resolution": 384
        }},
        "3": {"class_type": "Hy3DPostprocessMesh", "inputs": {
            "trimesh": ["2", 0],
            "remove_floaters": true,
            "remove_degenerate_faces": true,
            "reduce_faces": true,
            "max_facenum": request.face_count,
            "smooth_normals": false
        }},
        "4": {"class_type": "Hy3DExportMesh", "inputs": {
            "trimesh": ["3", 0],
            "filename_prefix": "wobu/hunyuan3d-2.1",
            "file_format": "glb",
            "save_file": true
        }},
        // `Hy3DExportMesh` returns an ordinary string. Current ComfyUI history
        // persists UI outputs only, so its core Preview3D node is the bridge
        // that makes the relative path available through `/history`.
        "5": {"class_type": "Preview3D", "inputs": {
            "model_file": ["4", 0]
        }}
    })
    .as_object()
    .cloned()
    .unwrap_or_default()
}

async fn watch<S, E>(
    frames: S,
    prompt_id: &str,
    classes: &BTreeMap<String, String>,
    progress: &mut dyn ProgressSink,
    cancel: &Cancel,
) -> Result<()>
where
    S: futures_core::Stream<Item = std::result::Result<Frame, E>>,
    E: std::fmt::Display,
{
    let mut frames = std::pin::pin!(frames);
    let mut started = false;
    let mut running: Option<String> = None;
    loop {
        let frame = match socket::next(frames.as_mut(), cancel).await {
            Next::Frame(Ok(frame)) => frame,
            Next::Frame(Err(error)) => {
                return Err(Error::Unavailable {
                    detail: format!("the connection to ComfyUI dropped during meshing: {error}"),
                });
            }
            Next::End => {
                return Err(Error::Unavailable {
                    detail: "ComfyUI closed the connection before the mesh finished".into(),
                });
            }
            Next::Cancelled => return Err(Error::Cancelled),
        };
        let Frame::Text(text) = frame else { continue };
        let event = Event::parse(&text);
        if event.prompt_id().is_some_and(|id| id != prompt_id) {
            continue;
        }
        match event {
            Event::Status { queue_remaining } if !started => {
                let note = if queue_remaining <= 1 {
                    "queued".to_owned()
                } else {
                    format!("queued, {} ahead", queue_remaining - 1)
                };
                progress.step(0, 1, Some(&note));
            }
            Event::ExecutionStart { .. } => {
                started = true;
                progress.step(0, 1, Some("starting local Hunyuan3D 2.1"));
            }
            Event::Executing { node: Some(node), .. } => running = Some(node),
            // Current ComfyUI emits `execution_success` before it publishes the
            // history item. Its final `executing`/null frame is sent after
            // `task_done`, so waiting for that frame avoids racing the history
            // GET and agrees with ComfyUI's official WebSocket example.
            Event::Executing { node: None, .. } => return Ok(()),
            Event::Progress { value, max, .. } => {
                let note = running.as_ref().and_then(|id| classes.get(id)).map(|class| class_note(class));
                progress.step(value, max.max(1), note);
            }
            Event::ExecutionError { node_type, message, .. } => {
                return Err(Error::Unavailable {
                    detail: format!("the {node_type} node failed while making the mesh: {message}"),
                });
            }
            Event::ExecutionInterrupted { .. } => return Err(Error::Cancelled),
            Event::Status { .. }
            | Event::Executed { .. }
            | Event::ExecutionSuccess { .. }
            | Event::Other => {}
        }
    }
}

fn class_note(class: &str) -> &str {
    match class {
        "Hy3D_2_1SimpleMeshGen" => "reconstructing geometry",
        "Hy3DPostprocessMesh" => "reducing the mesh",
        "Hy3DExportMesh" => "writing GLB",
        "LoadImage" => "loading the front view",
        other => other,
    }
}

fn classes(graph: &Map<String, Value>) -> BTreeMap<String, String> {
    graph
        .iter()
        .filter_map(|(id, node)| Some((id.clone(), node.get("class_type")?.as_str()?.to_owned())))
        .collect()
}

fn history_mesh_path(body: &[u8], prompt_id: &str, output: &str) -> Result<String> {
    let value: Value = serde_json::from_slice(body).map_err(|_| Error::NoMesh)?;
    let run = value.get(prompt_id).unwrap_or(&value);
    let output = run.pointer(&format!("/outputs/{output}")).ok_or(Error::NoMesh)?;
    // Preview3D's current UI output is `result[0]`. Accept the custom export
    // node's named return as well for ComfyUI builds that persisted ordinary
    // output-node values in history.
    let paths = output.get("result").or_else(|| output.get("glb_path")).ok_or(Error::NoMesh)?;
    let path = match paths {
        Value::Array(paths) => paths.first(),
        path => Some(path),
    };
    path.and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .ok_or(Error::NoMesh)
}

fn split_output_path(path: &str) -> (String, String) {
    let normal = path.replace('\\', "/");
    match normal.rsplit_once('/') {
        Some((folder, filename)) => (filename.to_owned(), folder.to_owned()),
        None => (normal, String::new()),
    }
}

fn upload_name(mime: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let extension = if mime.eq_ignore_ascii_case("image/png") { "png" } else { "jpg" };
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!(
        "wobu-hy3d-{since_epoch:x}-{:x}.{extension}",
        NEXT.fetch_add(1, Ordering::Relaxed),
    )
}

fn missing_nodes(missing: &[String]) -> Error {
    Error::Unavailable {
        detail: format!(
            "{LABEL} needs current ComfyUI plus `kijai/ComfyUI-Hunyuan3DWrapper`; this server is missing {}",
            missing.join(", "),
        ),
    }
}

fn no_such_model(model: &str, installed: &[String]) -> Error {
    Error::Unavailable {
        detail: if installed.is_empty() {
            format!(
                "{LABEL} cannot find `{model}`. Download the official 2.1 shape checkpoint into ComfyUI/models/diffusion_models"
            )
        } else {
            format!("{LABEL} cannot find `{model}`. This node offers: {}", installed.join(", "))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MeshView, View};

    fn png() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&512u32.to_be_bytes());
        bytes.extend_from_slice(&512u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    fn request() -> MeshRequest {
        MeshRequest::from_views(
            DEFAULT_MODEL,
            vec![MeshView::new(View::Front, png(), "image/png")],
        )
        .with_face_count(40_000)
        .with_generate_type(GenerateType::Geometry)
    }

    #[test]
    fn this_is_the_explicit_lower_local_tier() {
        let backend: Box<dyn MeshBackend> =
            Box::new(ComfyMeshBackend::new(comfy::DEFAULT_URL).unwrap());
        assert_eq!(backend.id(), comfy::ID);
        assert_eq!(backend.label(), "Local Hunyuan3D 2.1 (ComfyUI)");
        assert_eq!(backend.default_model(), DEFAULT_MODEL);
        let capabilities = backend.capabilities(DEFAULT_MODEL);
        assert_eq!(capabilities.max_views, 1);
        assert_eq!(capabilities.generate_types, [GenerateType::Geometry]);
        assert!(!capabilities.pbr);
        assert!(!capabilities.text_to_mesh);
        assert!(!capabilities.requires_billing);
    }

    #[test]
    fn the_graph_uses_the_verified_upstream_node_classes_and_face_budget() {
        let graph = workflow(&request(), "wobu/front.png");
        assert_eq!(graph["2"]["class_type"], "Hy3D_2_1SimpleMeshGen");
        assert_eq!(graph["2"]["inputs"]["model"], DEFAULT_MODEL);
        assert_eq!(graph["3"]["class_type"], "Hy3DPostprocessMesh");
        assert_eq!(graph["3"]["inputs"]["max_facenum"], 40_000);
        assert_eq!(graph["4"]["class_type"], "Hy3DExportMesh");
        assert_eq!(graph["4"]["inputs"]["file_format"], "glb");
        assert_eq!(graph[OUTPUT_NODE]["class_type"], "Preview3D");
        assert_eq!(graph[OUTPUT_NODE]["inputs"]["model_file"], json!(["4", 0]));
    }

    #[test]
    fn multi_view_text_and_pbr_are_refused_not_silently_downgraded() {
        let mut multiview = request();
        if let MeshInput::Views(views) = &mut multiview.input {
            views.push(MeshView::new(View::Left, png(), "image/png"));
        }
        assert!(validate(&multiview).unwrap_err().to_string().contains("one front image"));
        let side = MeshRequest::from_views(
            DEFAULT_MODEL,
            vec![MeshView::new(View::Left, png(), "image/png")],
        )
        .with_generate_type(GenerateType::Geometry);
        assert!(validate(&side).unwrap_err().to_string().contains("front view"));
        let prompt = MeshRequest::from_prompt(DEFAULT_MODEL, "a vessel")
            .with_generate_type(GenerateType::Geometry);
        assert!(validate(&prompt).unwrap_err().to_string().contains("no text-to-mesh"));
        let pbr = request().with_pbr(true);
        assert!(validate(&pbr).unwrap_err().to_string().contains("geometry-only"));
    }

    #[test]
    fn history_yields_the_exported_glb_on_windows_or_posix() {
        let body = serde_json::to_vec(&json!({
            "job": {"outputs": {"5": {"result": ["wobu\\asset_00001_.glb", null, null]}}}
        }))
        .unwrap();
        let path = history_mesh_path(&body, "job", "5").unwrap();
        assert_eq!(split_output_path(&path), ("asset_00001_.glb".into(), "wobu".into()));
    }

    #[test]
    fn local_outcomes_are_never_billed() {
        let outcome = MeshOutcome::new(MeshUsage::free(), Err(Error::Cancelled));
        assert_eq!(outcome.usage, MeshUsage::free());
    }
}
