//! [`ImageBackend`] over a local ComfyUI.
//!
//! The primary image target, and the one with no per-image cost: it runs on the
//! user's own GPU, needs no credentials, and
//! [`Capabilities::requires_billing`](crate::Capabilities::requires_billing) is
//! `false` on every model. `docs/08-providers.md` calls that asymmetry the point.
//!
//! Four things this adapter does that a thinner one would not, each of them
//! costing the user something real if it is skipped:
//!
//! 1. **Workflows are graphs, patched by node id.** A template is parsed, the
//!    inputs its binding table points at are set, and the graph is
//!    serialised once at the end. Substituting placeholders into the serialised
//!    JSON breaks the first time a prompt contains a brace — see `workflow.rs`.
//! 2. **Capabilities are probed, never assumed.** Two ComfyUI installs share a
//!    name and nothing else. Declaring a ControlNet this machine does not have
//!    produces a 400 with a Python traceback in it, which is a failure the user
//!    cannot act on — see `probe.rs`.
//! 3. **Cancelling interrupts the render.** Closing the websocket stops the
//!    reporting; the graph keeps the GPU. `ComfyBackend::stop` tells ComfyUI to
//!    stop, and picks between two calls that are not interchangeable.
//! 4. **A failure says which of three things went wrong.** Not running, running
//!    somewhere else, and running but missing a node are three different
//!    sentences pointing at three different fixes. A status code is none of them.
//!
//! ## What is not here
//!
//! **Reference images.** Core ComfyUI has no image-prompt mechanism: style and
//! object references need IPAdapter, which is a third-party pack, and structure
//! references need a ControlNet model that most installs do not have. So the
//! shipped workflows are text-to-image and this backend declares no reachable
//! reference mechanisms — see `no_reference_mechanisms`. ComfyUI's counting
//! budget stays unlimited, because a missing graph input is not a vendor quota.
//! That is not the same as ignoring references:
//! [`negotiate`](crate::negotiate) reports every withheld picture on the card it
//! came from, which is the behaviour the crate exists to provide, and
//! [`Installed::controlnets`] is public so the UI can say *why*.

mod probe;
pub(crate) mod socket;
pub(crate) mod wire;
mod workflow;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc as Shared, LazyLock, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use wobu_influence::ImageBudget;

use crate::Cancel;
use crate::aspect::Resolution;
use crate::backend::{
    GeneratedImage, ImageBackend, ImageOutcome, ImageRequest, ImageUsage, ProgressSink,
};
use crate::capability::{Capabilities, ReferenceMechanisms};
// Crate-level rather than a module of this one: the Gemini adapter reads back
// the size of what it was sent for the same reason, and a second header reader
// is a second place to get a JPEG frame marker wrong.
use crate::dimensions;
use crate::error::{Error, Result};

use socket::{Ended, until_cancelled};
use workflow::{Slot, Workflow};

pub use probe::{Installed, Server};

/// The `backend` in `project.json` and the `backend` field of every
/// `Generation`. There is no `wobu/comfyui` keychain entry, because there is no
/// key.
pub const ID: &str = "comfyui";

/// The name a person sees, including inside the errors built here.
pub const LABEL: &str = "ComfyUI";

/// Where ComfyUI listens unless somebody moved it. `--port` and `--listen` both
/// exist and both get used, which is why the wrong-port diagnosis is one of the
/// three.
pub const DEFAULT_URL: &str = "http://127.0.0.1:8188";

/// Used when a project names this backend but no model.
///
/// A checkpoint filename, per `backend.rs`: `ImageRequest::model` is
/// backend-specific and opaque, and for ComfyUI it is the name of a file in the
/// models folder. There is no id every install has, so this is a guess — and a
/// guess is safe here in a way it would not be on a paid backend, because a
/// server that does not have this file answers with a list of the files it does
/// have and nothing has been spent. See [`ComfyBackend::suggested_model`] for the
/// answer that is not a guess.
pub const DEFAULT_MODEL: &str = "flux1-dev.safetensors";

/// Long enough for a machine that is swapping a 20GB checkpoint off a spinning
/// disk, short enough that a wrong port is not mistaken for a slow one.
/// Deliberately *only* a connect timeout: progress arrives on the websocket, and
/// a whole-request timeout would abandon a render the user is still waiting for.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// One process-long HTTP transport for probes, queue checks and generations.
/// Connections stay isolated by origin inside reqwest's pool, so custom ComfyUI
/// addresses can safely share the transport without sharing server state.
static CLIENT: LazyLock<std::result::Result<Shared<reqwest::Client>, String>> =
    LazyLock::new(|| {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map(Shared::new)
            .map_err(|error| error.to_string())
    });

fn shared_client() -> Result<Shared<reqwest::Client>> {
    CLIENT
        .as_ref()
        .map(Shared::clone)
        .map_err(|detail| Error::Unavailable { detail: detail.clone() })
}

/// A ComfyUI server, and what it turned out to have.
///
/// Constructing one does no IO — [`ComfyBackend::new`] cannot fail on a machine
/// with no ComfyUI on it, because the Inspector has to draw a backend dropdown
/// before anything is running. [`connect`](ComfyBackend::connect) is the one that
/// asks.
pub struct ComfyBackend {
    base: String,
    client: Shared<reqwest::Client>,
    /// What ties a run to our websocket. One per backend rather than one per
    /// generation: ComfyUI keys previews to the client that queued the graph, so
    /// a client id minted per call would need a socket opened per call before
    /// the id existed.
    client_id: String,
    /// `None` until something has asked. Every read is short and nothing is held
    /// across an await, because [`capabilities`](ImageBackend::capabilities) is
    /// synchronous and the generate path is not.
    probed: RwLock<Option<Probed>>,
    ceiling: Option<Resolution>,
}

#[derive(Debug, Clone)]
struct Probed {
    installed: Installed,
    server: Server,
}

impl fmt::Debug for ComfyBackend {
    /// Hand-written to match the text adapters', even though there is no key to
    /// leak: `Installed` is a list of every model file on the user's disk, and a
    /// `{backend:?}` in a log line is not somewhere to put it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComfyBackend")
            .field("base", &self.base)
            .field("probed", &self.probed.read().map(|p| p.is_some()).unwrap_or(false))
            .finish()
    }
}

impl ComfyBackend {
    /// A backend pointed at a server, without asking whether it is there.
    ///
    /// `base_url` may be written the way people write it — `localhost:8188`,
    /// `http://192.168.1.4:8188/`, with or without a trailing slash — because it
    /// is a field somebody types into Settings and not a constant.
    pub fn new(base_url: impl AsRef<str>) -> Result<ComfyBackend> {
        let client = shared_client()?;
        Ok(ComfyBackend {
            base: normalise(base_url.as_ref()),
            client,
            client_id: client_id(),
            probed: RwLock::new(None),
            ceiling: None,
        })
    }

    /// A backend that has been asked what it has.
    ///
    /// The normal way to build one, and the only way
    /// [`capabilities`](ImageBackend::capabilities) can answer with anything but
    /// its most conservative guess — that method is synchronous, so the question
    /// has to have been asked before it is called.
    pub async fn connect(base_url: impl AsRef<str>) -> Result<ComfyBackend> {
        let backend = ComfyBackend::new(base_url)?;
        backend.probe(&Cancel::new()).await?;
        Ok(backend)
    }

    /// Override the resolution ceiling the probe inferred from VRAM.
    ///
    /// For the user who knows their own machine: tiled VAE decoding and a model
    /// offloaded to system RAM both go far past what the card alone would allow,
    /// and neither is visible from `/system_stats`.
    pub fn with_max_resolution(mut self, ceiling: Resolution) -> ComfyBackend {
        self.ceiling = Some(ceiling);
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// Ask the server what it has, and remember the answer.
    ///
    /// Two requests: `/system_stats` says whether this is ComfyUI at all and how
    /// much VRAM it has, and `/object_info` says which nodes and which model
    /// files. Both are re-read on every call, so this doubles as "reconnect"
    /// after the user installs a node pack.
    pub async fn probe(&self, cancel: &Cancel) -> Result<Installed> {
        let stats = self.get(probe::SYSTEM_STATS, cancel).await?;
        let Some(server) = Server::parse(&stats) else {
            return Err(not_comfyui(&self.base));
        };
        let object_info = self.get(probe::OBJECT_INFO, cancel).await?;
        let Some(installed) = Installed::parse(&object_info) else {
            return Err(not_comfyui(&self.base));
        };
        if let Ok(mut probed) = self.probed.write() {
            *probed = Some(Probed { installed: installed.clone(), server });
        }
        Ok(installed)
    }

    /// What the last probe found, without asking again.
    pub fn installed(&self) -> Option<Installed> {
        self.probed.read().ok()?.as_ref().map(|probed| probed.installed.clone())
    }

    /// What the last probe found out about the machine.
    pub fn server(&self) -> Option<Server> {
        self.probed.read().ok()?.as_ref().map(|probed| probed.server.clone())
    }

    /// A model this server actually has, for a project that names none.
    ///
    /// Unlike [`DEFAULT_MODEL`] this is not a guess — it is the first checkpoint
    /// in the loader's own list. `None` before a probe, or on a server with no
    /// models installed at all, which is a fresh clone and is worth saying out
    /// loud rather than failing at the first Generate.
    pub fn suggested_model(&self) -> Option<String> {
        let installed = self.installed()?;
        installed.checkpoints().first().or_else(|| installed.unets().first()).cloned()
    }

    /// One line for the status bar.
    ///
    /// Not a `Result`: "ComfyUI is not running" is the status, not a failure to
    /// find out what the status is, and a status bar with an error type in it
    /// would have to invent a sentence for the `Err` arm anyway.
    pub async fn health(&self, model: &str) -> Health {
        // `/prompt` answered with GET is the cheapest thing ComfyUI serves and
        // the only one that reports queue depth without also listing every
        // pending graph.
        match self.get("prompt", &Cancel::new()).await {
            Ok(body) => Health::Connected {
                model: model.to_owned(),
                queue: serde_json::from_slice::<Value>(&body)
                    .ok()
                    .and_then(|value| {
                        value.pointer("/exec_info/queue_remaining").and_then(Value::as_u64)
                    })
                    .unwrap_or(0) as u32,
            },
            Err(error) => Health::Unreachable { detail: error.to_string() },
        }
    }

    /// Tell ComfyUI to stop, rather than stopping listening.
    ///
    /// The two calls are not interchangeable and picking the wrong one is worse
    /// than not stopping at all:
    ///
    /// - `POST /queue {"delete": [id]}` removes a graph that has not started. It
    ///   names the prompt, so it can only ever affect ours.
    /// - `POST /interrupt` stops whatever is executing **and takes no prompt
    ///   id**. Called while our graph is still queued it kills the render in
    ///   front of us — somebody else's, or the user's own from the ComfyUI web UI
    ///   — and ours starts immediately afterwards, so the user has stopped a
    ///   stranger's work and not their own.
    ///
    /// So the delete goes first unconditionally, and the interrupt only after
    /// `/queue` has been re-read and confirms ours is the one on the GPU. The
    /// re-read is what closes the window between the last websocket event and
    /// the cancellation: `started` from the watch loop is a fact about the past.
    pub(crate) async fn stop(&self, prompt_id: &str, started: bool) {
        // Errors are dropped throughout: the user has already pressed Stop and
        // is going to get `Error::Cancelled` whatever these say. A ComfyUI that
        // has gone away has also stopped rendering.
        let uncancellable = Cancel::new();
        if !started {
            let _ = self
                .post("queue", serde_json::json!({ "delete": [prompt_id] }), &uncancellable)
                .await;
        }
        // Asked again rather than trusted: `started` is a fact about the last
        // event the watch loop saw, and a graph can leave the queue in the
        // moment between that and the Stop. Without the re-read, an interrupt
        // sent on the strength of a stale `false` would stop the render in front
        // of ours.
        if self.is_running(prompt_id, &uncancellable).await {
            let _ = self.post("interrupt", Value::Null, &uncancellable).await;
        }
    }

    async fn is_running(&self, prompt_id: &str, cancel: &Cancel) -> bool {
        let Ok(body) = self.get("queue", cancel).await else {
            return false;
        };
        let Ok(queue) = serde_json::from_slice::<Value>(&body) else {
            return false;
        };
        running(&queue, prompt_id)
    }

    /// One render, from a patched graph to bytes we are willing to keep.
    ///
    /// Split out of [`generate`](ImageBackend::generate) so the whole path can be
    /// written with `?`. The wrapper is what turns the `Result` back into an
    /// [`ImageOutcome`], and it is the only place a usage figure is decided.
    async fn render(
        &self,
        request: &ImageRequest,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> Result<GeneratedImage> {
        // A job cancelled while it was queued must not queue a graph. Locally
        // the cost is not money, it is the next job in the queue waiting behind
        // a render nobody wants.
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }

        let installed = match self.installed() {
            Some(installed) => installed,
            None => self.probe(cancel).await?,
        };
        let Some(family) = installed.family_of(&request.model) else {
            return Err(no_such_model(&request.model, &installed));
        };
        let workflow = Workflow::for_family(family);
        let missing = installed.missing(&workflow.classes());
        if !missing.is_empty() {
            return Err(missing_nodes(&missing));
        }
        let graph = workflow.patch(request)?;
        let nodes = classes(&graph);

        // Before `/prompt`, never after: a graph whose nodes are all cached —
        // the second image of a batch, where only the seed changed — can be
        // finished before a socket opened afterwards receives anything, and the
        // render appears to hang until it times out.
        let socket = match until_cancelled(socket::connect(&self.ws_url()), cancel).await {
            None => return Err(Error::Cancelled),
            Some(socket) => socket?,
        };

        let queued = self.post("prompt", wire::prompt_body(graph, &self.client_id), cancel).await?;
        let Some(prompt_id) = wire::queued(&queued) else {
            return Err(Error::Unavailable {
                detail: "ComfyUI accepted the workflow but gave it no prompt id".into(),
            });
        };

        let watched =
            socket::watch(socket, &prompt_id, workflow.output, &nodes, progress, cancel).await;
        let images = match watched.ended {
            Ended::Images(images) => images,
            Ended::Failed(error) => return Err(error),
            Ended::Cancelled => {
                // The whole reason this is not `return Err(Cancelled)` on its
                // own. Dropping the socket above has already stopped the
                // reporting; without this the graph runs to completion on a GPU
                // the user asked for back.
                self.stop(&prompt_id, watched.started).await;
                return Err(Error::Cancelled);
            }
        };

        let image = &images[0];
        let bytes = self.get(&format!("view?{}", image.query()), cancel).await?;
        let Some((width, height)) = dimensions::read(&bytes) else {
            return Err(Error::NotAnImage {
                detail: format!(
                    "ComfyUI wrote {}, and the {} bytes fetched back are not a PNG, JPEG or WebP",
                    image.filename,
                    bytes.len(),
                ),
            });
        };
        Ok(GeneratedImage {
            mime: dimensions::mime(&bytes).to_owned(),
            bytes,
            // Read back rather than echoed, per the trait's fifth contract
            // point: a workflow with a hires pass returns something larger than
            // the latent it started from, and the requested numbers would be a
            // size that never existed.
            width,
            height,
            // We chose it and wrote it into the graph, so this is a fact rather
            // than a hope — which is what lets a render the user liked be
            // repeated.
            seed: Some(request.seed),
            // No local checkpoint watermarks its own output, and claiming one
            // would put a badge on a card for something that is not there.
            watermark: None,
        })
    }

    pub(crate) fn ws_url(&self) -> String {
        let host = self.base.strip_prefix("http").unwrap_or(&self.base);
        format!("ws{host}/ws?clientId={}", self.client_id)
    }

    pub(crate) async fn get(&self, path: &str, cancel: &Cancel) -> Result<Vec<u8>> {
        self.send(self.client.get(format!("{}/{path}", self.base)), path, cancel).await
    }

    async fn post(&self, path: &str, body: Value, cancel: &Cancel) -> Result<Vec<u8>> {
        let request = self
            .client
            .post(format!("{}/{path}", self.base))
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&body).unwrap_or_default());
        self.send(request, path, cancel).await
    }

    /// Queue a graph under this backend's websocket client id.
    pub(crate) async fn queue(
        &self,
        graph: serde_json::Map<String, Value>,
        cancel: &Cancel,
    ) -> Result<String> {
        let queued = self.post("prompt", wire::prompt_body(graph, &self.client_id), cancel).await?;
        wire::queued(&queued).ok_or_else(|| Error::Unavailable {
            detail: "ComfyUI accepted the workflow but gave it no prompt id".into(),
        })
    }

    /// Upload one workflow input and return the name `LoadImage` accepts.
    pub(crate) async fn upload_image(
        &self,
        filename: &str,
        mime: &str,
        bytes: &[u8],
        cancel: &Cancel,
    ) -> Result<String> {
        let part = reqwest::multipart::Part::bytes(bytes.to_vec())
            .file_name(filename.to_owned())
            .mime_str(mime)
            .map_err(|error| Error::Unsupported { detail: error.to_string() })?;
        let form = reqwest::multipart::Form::new()
            .part("image", part)
            .text("type", "input")
            .text("overwrite", "true");
        let request = self.client.post(format!("{}/upload/image", self.base)).multipart(form);
        let body = self.send(request, "upload/image", cancel).await?;
        let value: Value = serde_json::from_slice(&body).map_err(|_| Error::Unavailable {
            detail: "ComfyUI accepted the input image but returned no usable filename".into(),
        })?;
        let name = value.get("name").and_then(Value::as_str).unwrap_or_default();
        let subfolder = value.get("subfolder").and_then(Value::as_str).unwrap_or_default();
        if name.is_empty() {
            return Err(Error::Unavailable {
                detail: "ComfyUI accepted the input image but returned no usable filename".into(),
            });
        }
        Ok(if subfolder.is_empty() { name.to_owned() } else { format!("{subfolder}/{name}") })
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        path: &str,
        cancel: &Cancel,
    ) -> Result<Vec<u8>> {
        let response = match until_cancelled(request.send(), cancel).await {
            None => return Err(Error::Cancelled),
            Some(Err(e)) => return Err(unreachable(&self.base, &e)),
            Some(Ok(response)) => response,
        };
        let status = response.status().as_u16();
        let body = match until_cancelled(response.bytes(), cancel).await {
            None => return Err(Error::Cancelled),
            // A body we could not read still leaves a status worth reporting.
            Some(body) => body.map(|bytes| bytes.to_vec()).unwrap_or_default(),
        };
        match status {
            200..=299 => Ok(body),
            401 | 403 => Err(Error::Unavailable {
                detail: format!(
                    "the ComfyUI endpoint at {} requires authentication (HTTP {status}). Wobu does not store credentials in endpoint URLs or send proxy authentication headers",
                    self.base,
                ),
            }),
            // A 404 on a path every ComfyUI serves is the wrong service on the
            // right port far more often than it is a broken ComfyUI, and the
            // two send the user somewhere completely different.
            404 => Err(not_comfyui(&self.base)),
            _ if path == "prompt" => Err(wire::rejected(&body, status)),
            _ => Err(Error::Unavailable {
                detail: format!("ComfyUI answered /{path} with HTTP {status}"),
            }),
        }
    }
}

#[async_trait]
impl ImageBackend for ComfyBackend {
    fn id(&self) -> &'static str {
        ID
    }

    fn label(&self) -> &'static str {
        LABEL
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    /// What this server, running this model, can do.
    ///
    /// Every answer that could be wrong is read off the probe, and an unprobed
    /// backend gets the conservative one. That asymmetry is deliberate: a
    /// capability declared and absent is a 400 with a traceback in it, and a
    /// capability present and undeclared is a downgrade the user is told about
    /// and can fix by reconnecting.
    fn capabilities(&self, model: &str) -> Capabilities {
        let probed = self.probed.read().ok().and_then(|probed| probed.clone());
        let family = probed.as_ref().and_then(|probed| probed.installed.family_of(model));
        Capabilities {
            max_resolution: self.ceiling.unwrap_or_else(|| {
                ceiling(probed.as_ref().and_then(|probed| probed.server.vram_bytes))
            }),
            // ComfyUI takes width and height, not a ratio. An empty list means
            // the parameter is not taken, which `capability.rs` is explicit is
            // not the same as every value being refused.
            aspect_ratios: vec![],
            // ComfyUI itself imposes no vendor counting quota. Whether an image
            // can reach the graph is the independent mechanism axis below.
            image_refs: ImageBudget::unlimited(),
            // No shipped workflow has an image input yet, even on an install
            // with ControlNet models. Declaring no reachable mechanism keeps
            // that honest without pretending ComfyUI has 0/0/0 vendor buckets.
            reference_mechanisms: no_reference_mechanisms(),
            // Nothing in the influence stack routes to a LoRA yet, so this
            // changes nothing about the request. It decides whether the UI shows
            // a LoRA picker, and a picker that silently did nothing would be the
            // same failure as a silently dropped reference.
            loras: probed.as_ref().is_some_and(|probed| probed.installed.has_loras()),
            // Per model and not per backend, and this is the case that forces
            // it: Flux's guidance chain has no negative conditioning, so the
            // `flux_unet` workflow has no slot to put one in. Answering `true`
            // for it would let `negotiate` compile a `never:` list that
            // `Workflow::patch` then has to refuse.
            negative_prompt: family
                .map(|family| Workflow::for_family(family).has(Slot::Negative))
                // A model this server has never heard of, or an unprobed
                // backend. `generate` will not run either, so this is only what
                // the Inspector draws — and the checkpoint workflow is the one a
                // ComfyUI is most likely to be able to run.
                .unwrap_or(true),
            requires_billing: false,
            streaming_preview: true,
        }
    }

    fn supports_lora(&self, model: &str, provider_name: &str) -> bool {
        self.probed.read().ok().and_then(|probed| probed.clone()).is_some_and(|probed| {
            probed.installed.family_of(model).is_some()
                && probed.installed.has_loras()
                && probed.installed.loras().iter().any(|name| name == provider_name)
        })
    }

    async fn generate(
        &self,
        request: &ImageRequest,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> ImageOutcome {
        // Free on every path, success or failure, and stated once here rather
        // than at each return. A local render costs the user electricity and
        // twenty minutes of their GPU, which `capability.rs` is explicit is not
        // what `requires_billing` and `ImageUsage` measure: `free` means no
        // money moved at a provider.
        ImageOutcome::new(ImageUsage::free(), self.render(request, progress, cancel).await)
    }
}

/// What the status bar shows.
///
/// `ComfyUI connected · flux-dev · queue 0` is the line
/// [#51](https://github.com/krazyjakee/wobu/issues/51) asks for, and the
/// disconnected form is the same line carrying the diagnosis — because the
/// status bar is where somebody looks first when Generate does nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    Connected { model: String, queue: u32 },
    Unreachable { detail: String },
}

impl Health {
    pub fn is_connected(&self) -> bool {
        matches!(self, Health::Connected { .. })
    }
}

impl fmt::Display for Health {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Health::Connected { model, queue } => {
                write!(f, "{LABEL} connected · {model} · queue {queue}")
            }
            Health::Unreachable { detail } => write!(f, "{LABEL} unreachable — {detail}"),
        }
    }
}

/// No reachable reference mechanism at all.
///
/// The shipped workflows are text-to-image, and this is the declaration that
/// makes that visible instead of silent. Core ComfyUI has no image-prompt
/// mechanism: style and object references need IPAdapter, which is a third-party
/// pack, and structure references need both a ControlNet model and a graph to
/// apply it in.
///
/// The consequence is that [`negotiate`](crate::negotiate) reports every
/// attached picture as withheld, on the card it came from. That is worse than
/// sending them and better than the two alternatives — silently leaving them out,
/// or declaring a capability that produces a 400 the user cannot act on.
fn no_reference_mechanisms() -> ReferenceMechanisms {
    ReferenceMechanisms::none()
}

/// The resolution ceiling a card this size can be asked for.
///
/// Bands rather than a formula, and generous rather than exact. There is no
/// function from VRAM to a safe resolution — it depends on the architecture, the
/// weight dtype, whether the VAE decode is tiled and what else is on the card —
/// so a formula would be a precise-looking number that is wrong in both
/// directions. What these do is keep a 6GB laptop from being offered 2048px,
/// which is an out-of-memory twenty seconds into a render, while not capping a
/// 24GB card at a size it clears easily. [`ComfyBackend::with_max_resolution`] is
/// the lever for anyone who knows better, and they usually do.
fn ceiling(vram_bytes: Option<u64>) -> Resolution {
    const GIB: u64 = 1 << 30;
    match vram_bytes {
        Some(vram) if vram >= 20 * GIB => Resolution::new(2048, 2048),
        Some(vram) if vram >= 10 * GIB => Resolution::new(1536, 1536),
        // Also the CPU-only answer. It renders, slowly, and asking it for 2048px
        // is asking for an afternoon.
        _ => Resolution::new(1024, 1024),
    }
}

/// **Diagnosis one: it is not running.**
///
/// The commonest failure by a distance, and the one where a status code would be
/// least use — there is no status code, because nothing answered.
fn unreachable(base: &str, error: &reqwest::Error) -> Error {
    let detail = if error.is_timeout() {
        format!(
            "{LABEL} at {base} did not answer within {} seconds. It may be starting up, or \
             loading a model — try again in a moment",
            CONNECT_TIMEOUT.as_secs(),
        )
    } else if error.is_connect() {
        format!(
            "nothing is listening at {base}. Start {LABEL}, or point wobu at the address it is \
             running on — `--listen` and `--port` both move it"
        )
    } else {
        format!("could not reach {LABEL} at {base}: {error}")
    };
    Error::Unavailable { detail }
}

/// **Diagnosis two: something is there, and it is not ComfyUI.**
///
/// 8188 is a plain HTTP port and a machine that runs image models usually runs
/// several things that want one. Reported as "unavailable" without this, the
/// user reads it as ComfyUI being broken and restarts a ComfyUI that is running
/// perfectly well on a different port.
fn not_comfyui(base: &str) -> Error {
    Error::Unavailable {
        detail: format!(
            "something is answering at {base}, but it is not {LABEL} — check the port \
             ({LABEL}'s default is 8188)"
        ),
    }
}

/// **Diagnosis three: it is ComfyUI, and it is missing a node.**
///
/// The message #51 asks for by name. ComfyUI's own answer is a validation
/// failure naming a node number in a graph the user never saw; the thing they
/// need is the class name, because that is what they type into a search box.
fn missing_nodes(missing: &[String]) -> Error {
    Error::Unavailable {
        detail: match missing {
            [one] => format!(
                "this workflow references the node `{one}`, which is not installed in this \
                 {LABEL} — install the custom node pack that provides it, or update {LABEL}"
            ),
            many => format!(
                "this workflow references {} nodes that are not installed in this {LABEL}, \
                 starting with `{}` — install the custom node packs that provide them, or \
                 update {LABEL}",
                many.len(),
                many[0],
            ),
        },
    }
}

/// A model this server has never heard of.
///
/// `error.rs` reserves [`Error::Unavailable`] for exactly this and says why: it
/// is "the backend is not able to serve this right now, and waiting is a
/// sensible thing to do", where the waiting is the user downloading the file.
/// The list is what turns that from a dead end into a choice.
fn no_such_model(model: &str, installed: &Installed) -> Error {
    let mut has: Vec<&str> =
        installed.checkpoints().iter().chain(installed.unets()).map(String::as_str).collect();
    has.truncate(6);
    Error::Unavailable {
        detail: match has.as_slice() {
            [] => format!(
                "this {LABEL} has no models installed at all, so `{model}` is not the only thing \
                 missing — put a checkpoint in `models/checkpoints` and reconnect"
            ),
            has => {
                format!("this {LABEL} has no model called `{model}`. It has: {}", has.join(", "),)
            }
        },
    }
}

/// Node id to class, for the sampler note on the progress bar.
fn classes(graph: &serde_json::Map<String, Value>) -> BTreeMap<String, String> {
    graph
        .iter()
        .filter_map(|(id, node)| Some((id.clone(), node.get("class_type")?.as_str()?.to_owned())))
        .collect()
}

/// Whether `/queue` says this prompt is the one on the GPU.
///
/// An entry is `[number, prompt_id, graph, extra, outputs]`, so the id is at
/// index 1. Split out to be readable and to be checked, because the consequence
/// of reading it wrong is `/interrupt` stopping a render that is not ours.
fn running(queue: &Value, prompt_id: &str) -> bool {
    queue.get("queue_running").and_then(Value::as_array).is_some_and(|entries| {
        entries.iter().any(|entry| entry.get(1).and_then(Value::as_str) == Some(prompt_id))
    })
}

/// `localhost:8188`, `http://127.0.0.1:8188/` and `HTTP://Host:8188` all name the
/// same server. This is a field somebody types, not a constant.
fn normalise(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return DEFAULT_URL.to_owned();
    }
    match trimmed.split_once("://") {
        Some(_) => trimmed.to_owned(),
        None => format!("http://{trimmed}"),
    }
}

/// Distinct per backend and per process.
///
/// Not a UUID crate for one string: ComfyUI uses this only to route events back
/// to the socket that queued a graph, so what it needs is to be different from
/// whatever the ComfyUI web UI generated for itself, and a clash means somebody
/// else's previews.
fn client_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!("wobu-{since_epoch:x}-{:x}", NEXT.fetch_add(1, Ordering::Relaxed))
}
#[cfg(test)]
mod tests;
