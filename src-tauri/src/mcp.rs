//! Wiring `wobu-mcp` to the open project, and the switches that decide whether
//! any of it happens.
//!
//! The protocol lives in the crate; this file is the part with consequences.
//! It owns the settings, the listener handle, the implementation of
//! [`wobu_mcp::World`] against the project in the window, and the activity log
//! that lets a person see what an agent has been doing in their world.
//!
//! ## Where the settings live, and why not in `settings.json`
//!
//! `app_data_dir()/mcp.json`, `0600` on Unix, beside `machine.rs`'s file rather
//! than inside it. Two reasons. The bearer token is a credential, and a file
//! that holds one should be openable, greppable and deletable on its own — "how
//! do I revoke this" has a one-line answer. And MCP is per installation for the
//! same reason a ComfyUI endpoint is: a port on this machine and a command on
//! this machine's `PATH` are not things a collaborator on the other end of a
//! share can use, so none of it belongs in `project.json`.
//!
//! ## What is off, and what "off" means
//!
//! Both halves are off in [`ServerSettings::default`] and
//! [`ClientSettings::default`], which is also what an absent or unparseable
//! settings file resolves to. "Off" for the server means no socket exists:
//! [`Running`] is dropped and its accept loop stops, rather than a flag being
//! consulted per request. "Off" for the client means no process exists:
//! [`Registry::shutdown`] kills the children.
//!
//! ## The write path
//!
//! Writes are a second, independent opt-in, and the flag is an `AtomicBool`
//! shared with the dispatcher rather than a value captured at start — so
//! unticking the box stops the *next* call rather than the next launch. Every
//! call, refused or not, is stamped, kept in a small ring, written to the
//! diagnostics log and emitted as [`MCP_ACTIVITY`]; the disclosure this feature
//! rests on is worth very little without somebody being able to check it.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};
use wobu_core::{Id, LinkRole, Node, NodeKind, default_preset};
use wobu_influence::{Budget, Shot, Sliders, World as InfluenceWorld, compile, fragments, resolve};
use wobu_mcp::client::{Registry, RemoteServer};
use wobu_mcp::config::{ClientServer, ClientSettings, ServerSettings, Token};
use wobu_mcp::dispatch::{Audit, CallRecord};
use wobu_mcp::world::{NodePatch, World, WorldError, WorldResult};
use wobu_mcp::{Dispatcher, Running, Server};
use wobu_store::{Project, SaveOutcome, paths};

use crate::diag;
use crate::error::{Code, CommandResult, WobuError};
use crate::state::AppState;

/// One MCP tool call, as it happened. `src/components/McpSection.tsx` listens.
pub const MCP_ACTIVITY: &str = "mcp:activity";

/// How many calls the activity list remembers. Enough to answer "what just
/// happened", short enough that it is never a second copy of the world.
const ACTIVITY_LIMIT: usize = 50;

/* ── persisted shape ──────────────────────────────────────────────────────── */

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    #[serde(default)]
    server: ServerSettings,
    #[serde(default)]
    client: ClientSettings,
}

/* ── views ────────────────────────────────────────────────────────────────── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpView {
    pub server: ServerView,
    pub client: ClientView,
    /// The catalogue, so the pane's disclosure is generated from the same list
    /// the protocol advertises rather than retyped beside it.
    pub tools: Vec<ToolView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    pub enabled: bool,
    /// Whether a socket is actually open. Distinct from `enabled`, because a
    /// port that was taken means the two disagree and the user needs to know
    /// which one is true.
    pub running: bool,
    pub port: u16,
    pub endpoint: Option<String>,
    pub allow_writes: bool,
    /// Six characters and an ellipsis. The whole token is only ever sent by
    /// [`mcp_server_token`], which the user has to ask for.
    pub token_preview: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolView {
    pub name: String,
    pub title: String,
    pub description: String,
    pub write: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientView {
    pub enabled: bool,
    pub servers: Vec<ClientServerView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientServerView {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub enabled: bool,
    /// Whether this server carries environment overrides. The values are never
    /// sent to the webview — they are the likeliest place for one of the user's
    /// own API keys to be — so the pane says that they exist and leaves editing
    /// them to the file.
    pub has_env: bool,
}

/// What the pane sends when a server is added or edited.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientServerInput {
    /// Absent for a new server.
    pub id: Option<String>,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
}

/// One line of the activity log.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub at: String,
    pub tool: String,
    pub write: bool,
    pub ok: bool,
    pub detail: Option<String>,
}

/* ── managed state ────────────────────────────────────────────────────────── */

pub struct McpState {
    path: PathBuf,
    stored: RwLock<Stored>,
    /// Shared with the dispatcher. The one piece of configuration that is read
    /// per call rather than at start; see the module header.
    allow_writes: Arc<AtomicBool>,
    running: Mutex<Option<Running>>,
    /// Why the listener is not up although `enabled` says it should be.
    last_error: Mutex<Option<String>>,
    registry: Arc<Registry>,
    activity: Arc<Mutex<VecDeque<Activity>>>,
    /// Filled in `setup`. `None` before then, which is also the state every
    /// method here is written to survive — nothing may start a listener during
    /// managed-state construction, because that would be a socket opened before
    /// any settings were consulted.
    wiring: Mutex<Option<Wiring>>,
    /// Serialises start/stop so that two clicks cannot leave a `Running` value
    /// orphaned by a second one overwriting it.
    applying: tokio::sync::Mutex<()>,
}

struct Wiring {
    app: AppHandle,
    world: AppState,
}

impl Default for McpState {
    fn default() -> Self {
        McpState::load_from(paths::app_data_dir().join("mcp.json"))
    }
}

impl McpState {
    fn load_from(path: PathBuf) -> McpState {
        // A file that will not parse is read as "off", not as a reason to fail
        // to start: the alternative is an app that refuses to launch because of
        // a feature the user may never have turned on.
        let stored = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Stored>(&raw).ok())
            .unwrap_or_default();
        let allow_writes = Arc::new(AtomicBool::new(stored.server.allow_writes));
        McpState {
            path,
            stored: RwLock::new(stored),
            allow_writes,
            running: Mutex::new(None),
            last_error: Mutex::new(None),
            registry: Arc::new(Registry::new()),
            activity: Arc::new(Mutex::new(VecDeque::new())),
            wiring: Mutex::new(None),
            applying: tokio::sync::Mutex::new(()),
        }
    }

    pub fn view(&self) -> McpView {
        let stored = self.stored.read();
        let running = self.running.lock();
        McpView {
            server: ServerView {
                enabled: stored.server.enabled,
                running: running.is_some(),
                port: running.as_ref().map_or(stored.server.port, Running::port),
                endpoint: running.as_ref().map(Running::endpoint),
                allow_writes: self.allow_writes.load(Ordering::SeqCst),
                token_preview: stored.server.token.as_ref().map(Token::preview),
                error: self.last_error.lock().clone(),
            },
            client: ClientView {
                enabled: stored.client.enabled,
                servers: stored.client.servers.iter().map(server_view).collect(),
            },
            tools: wobu_mcp::catalogue()
                .iter()
                .map(|tool| ToolView {
                    name: tool.name.to_owned(),
                    title: tool.title.to_owned(),
                    description: tool.description.to_owned(),
                    write: tool.write,
                })
                .collect(),
        }
    }

    fn save(&self) -> CommandResult<()> {
        let stored = self.stored.read().clone();
        save(&self.path, &stored)
    }

    /// Bring the listener into line with the settings. Stops first, always, so
    /// there is no path where a port change leaves the old socket open.
    async fn apply_server(&self) -> CommandResult<()> {
        let _serialised = self.applying.lock().await;

        // Dropped outside the lock on `running` so that nothing is held while a
        // `Drop` runs.
        let previous = self.running.lock().take();
        drop(previous);
        *self.last_error.lock() = None;

        let (enabled, port, token) = {
            let stored = self.stored.read();
            (stored.server.enabled, stored.server.port, stored.server.token.clone())
        };
        if !enabled {
            diag::info("mcp: server stopped");
            return Ok(());
        }
        let Some(token) = token else {
            // Only reachable if a hand-edited file says `enabled` with no token.
            return Err(WobuError::new(
                Code::Internal,
                "Wobu's MCP server has no access token. Turn it off and on again to make one.",
            ));
        };
        let Some(dispatcher) = self.dispatcher() else {
            return Err(WobuError::new(
                Code::Internal,
                "Wobu is still starting up. Try again in a moment.",
            ));
        };

        match Server::start(port, token, dispatcher).await {
            Ok(running) => {
                diag::info(format!("mcp: server listening on {}", running.endpoint()));
                *self.running.lock() = Some(running);
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                *self.last_error.lock() = Some(message.clone());
                Err(WobuError::new(Code::Io, format!("Wobu's MCP server could not start. {message}")))
            }
        }
    }

    fn dispatcher(&self) -> Option<Arc<Dispatcher>> {
        let wiring = self.wiring.lock();
        let wiring = wiring.as_ref()?;
        let world = ProjectWorld { state: wiring.world.handle() };
        let audit = Recorder {
            app: wiring.app.clone(),
            activity: Arc::clone(&self.activity),
        };
        Some(Arc::new(Dispatcher::new(
            Arc::new(world),
            Arc::clone(&self.allow_writes),
            Arc::new(audit),
            "wobu",
            env!("CARGO_PKG_VERSION"),
        )))
    }

    fn activity(&self) -> Vec<Activity> {
        self.activity.lock().iter().rev().cloned().collect()
    }

    /// Every socket closed and every child killed. Called from `RunEvent::Exit`.
    pub fn shut_down(&self) {
        let running = self.running.lock().take();
        drop(running);
        self.registry.shutdown();
    }
}

fn server_view(server: &ClientServer) -> ClientServerView {
    ClientServerView {
        id: server.id.clone(),
        name: server.name.clone(),
        command: server.command.clone(),
        args: server.args.clone(),
        enabled: server.enabled,
        has_env: !server.env.is_empty(),
    }
}

/* ── audit ────────────────────────────────────────────────────────────────── */

struct Recorder {
    app: AppHandle,
    activity: Arc<Mutex<VecDeque<Activity>>>,
}

impl Audit for Recorder {
    fn record(&self, entry: CallRecord) {
        let activity = Activity {
            at: chrono::Utc::now().to_rfc3339(),
            tool: entry.tool,
            write: entry.write,
            ok: entry.ok,
            detail: entry.detail,
        };
        // In the diagnostics log as well as on screen, because the pane's ring
        // is fifty entries and the question "what did it do while I was at
        // lunch" is asked about a longer window than that.
        diag::info(format!(
            "mcp: {} {}{}",
            activity.tool,
            if activity.ok { "ok" } else { "refused" },
            activity.detail.as_deref().map(|d| format!(" ({d})")).unwrap_or_default(),
        ));
        {
            let mut ring = self.activity.lock();
            ring.push_back(activity.clone());
            while ring.len() > ACTIVITY_LIMIT {
                ring.pop_front();
            }
        }
        let _ = self.app.emit(MCP_ACTIVITY, activity);
    }
}

/* ── the world an agent sees ──────────────────────────────────────────────── */

struct ProjectWorld {
    state: AppState,
}

impl ProjectWorld {
    fn with<T>(&self, f: impl FnOnce(&mut Project) -> CommandResult<T>) -> Result<T, WorldError> {
        self.state.with(f).map_err(|error| WorldError {
            message: error.message,
            retryable: error.retryable,
        })
    }
}

fn parse_id(raw: &str) -> CommandResult<Id> {
    Id::from_string(raw.trim())
        .map_err(|_| WobuError::new(Code::Invalid, format!("{raw:?} is not a Wobu id (a ULID).")))
}

fn parse_kind(raw: &str) -> CommandResult<NodeKind> {
    serde_json::from_value(Value::String(raw.trim().to_owned())).map_err(|_| {
        let known: Vec<_> = NodeKind::ALL.iter().map(|kind| kind.as_str()).collect();
        WobuError::new(
            Code::Invalid,
            format!("{raw:?} is not a Wobu kind. Known kinds: {}.", known.join(", ")),
        )
    })
}

fn parse_role(raw: &str) -> CommandResult<LinkRole> {
    serde_json::from_value(Value::String(raw.trim().to_owned())).map_err(|_| {
        let known: Vec<_> = LinkRole::ALL.iter().map(|role| role.as_str()).collect();
        WobuError::new(
            Code::Invalid,
            format!("{raw:?} is not a link role. Known roles: {}.", known.join(", ")),
        )
    })
}

fn saved(outcome: SaveOutcome) -> CommandResult<Value> {
    match outcome {
        SaveOutcome::Saved(node) => Ok(value(&*node)),
        // The same answer the editor gets: a collaborator won the race and the
        // agent's version is parked beside theirs. Merging prose is not a thing
        // to do behind somebody's back.
        SaveOutcome::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    }
}

/// Serialise, or say so. Nothing here can realistically fail — these are the
/// same types the webview receives — and a panic in a background HTTP handler
/// would take the window with it.
fn value<T: Serialize>(item: &T) -> Value {
    serde_json::to_value(item).unwrap_or_else(|error| json!({ "error": error.to_string() }))
}

impl World for ProjectWorld {
    fn overview(&self) -> WorldResult {
        self.with(|project| {
            let summary = project.summary();
            let nodes = project.list_nodes()?;
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for node in &nodes {
                *counts.entry(node.kind.as_str()).or_default() += 1;
            }
            Ok(json!({
                "project": value(&summary),
                "nodeCount": nodes.len(),
                "countsByKind": counts,
                "kinds": NodeKind::ALL.iter().map(|kind| kind.as_str()).collect::<Vec<_>>(),
                "linkRoles": LinkRole::ALL.iter().map(|role| role.as_str()).collect::<Vec<_>>(),
            }))
        })
    }

    fn list_nodes(&self, kind: Option<&str>) -> WorldResult {
        self.with(|project| {
            let wanted = kind.map(parse_kind).transpose()?;
            let nodes: Vec<_> = project
                .list_nodes()?
                .into_iter()
                .filter(|node| wanted.is_none_or(|wanted| node.kind == wanted))
                .collect();
            Ok(json!({ "count": nodes.len(), "nodes": value(&nodes) }))
        })
    }

    fn get_node(&self, id: &str) -> WorldResult {
        self.with(|project| {
            let node = project.get_node(parse_id(id)?)?;
            Ok(value(&node))
        })
    }

    fn search_nodes(&self, query: &str, limit: usize) -> WorldResult {
        self.with(|project| {
            let hits = project.index().search(query)?;
            // Summaries rather than ids: an agent handed ids spends a call per
            // hit finding out what they were, and the index has already read
            // the rows.
            let summaries = project.list_nodes()?;
            let matched: Vec<_> = hits
                .iter()
                .filter_map(|id| summaries.iter().find(|summary| summary.id == *id))
                .take(limit)
                .collect();
            Ok(json!({ "query": query, "count": matched.len(), "nodes": value(&matched) }))
        })
    }

    fn node_links(&self, id: &str) -> WorldResult {
        self.with(|project| {
            let id = parse_id(id)?;
            let node = project.get_node(id)?;
            let backlinks = project.node_backlinks(id)?;
            Ok(json!({
                "nodeId": id.to_string(),
                "parentId": node.parent_id.map(|parent| parent.to_string()),
                "outgoing": value(&node.links),
                "incoming": value(&backlinks),
            }))
        })
    }

    fn influence_stack(&self, subject_id: &str, preset: Option<&str>) -> WorldResult {
        self.with(|project| {
            let subject = parse_id(subject_id)?;
            let nodes = project.world_nodes()?;
            let sheet = preset_for(nodes, subject, preset)?;
            let world = InfluenceWorld::new(nodes.iter());
            // No Shot layer: resolving to explain is not resolving to generate,
            // and inventing framing here would describe a shot nobody set up.
            let stack = resolve(&world, subject, None).ok_or_else(|| no_such_subject(subject))?;
            let layers: Vec<Value> = stack
                .sources()
                .iter()
                .map(|source| {
                    json!({
                        "layer": value(&source.layer),
                        "nodeId": source.node_id().map(|id| id.to_string()),
                        "name": source.name(),
                        "kind": source.node().map(|node| node.kind.as_str()),
                        "reached": value(&source.reached),
                        "distance": source.distance,
                        "weight": source.weight,
                    })
                })
                .collect();
            Ok(json!({
                "subjectId": subject.to_string(),
                "preset": value(&sheet),
                "layers": layers,
            }))
        })
    }

    fn compile_prompt(&self, subject_id: &str, preset: Option<&str>) -> WorldResult {
        self.with(|project| {
            let subject = parse_id(subject_id)?;
            let nodes = project.world_nodes()?;
            let sheet = preset_for(nodes, subject, preset)?;
            let world = InfluenceWorld::new(nodes.iter());
            // Always a Shot, unlike the stack above: the preset's framing text
            // is that layer's whole contribution, and a prompt compiled without
            // one is not the prompt a generation would send.
            let shot = Shot { label: sheet.label, weight: 1.0 };
            let stack =
                resolve(&world, subject, Some(shot)).ok_or_else(|| no_such_subject(subject))?;
            let extracted = fragments(&stack, sheet, &Sliders::neutral());
            let compiled = compile(&extracted, Budget::unlimited());
            let contributors: Vec<Value> = extracted
                .iter()
                .filter(|fragment| fragment.is_sendable())
                .map(|fragment| {
                    json!({
                        "layer": value(&fragment.layer()),
                        "nodeId": fragment.node_id().map(|id| id.to_string()),
                        "source": fragment.source_name(),
                        "section": fragment.section(),
                        "weight": fragment.weight(),
                    })
                })
                .collect();
            Ok(json!({
                "subjectId": subject.to_string(),
                "preset": value(&sheet),
                "prompt": compiled.prompt(),
                "negative": compiled.negative(),
                "contributors": contributors,
                "note": "Compiled only. Nothing was generated and nothing was spent.",
            }))
        })
    }

    fn list_generations(&self, node_id: &str, limit: usize) -> WorldResult {
        self.with(|project| {
            let id = parse_id(node_id)?;
            let mut generations = project.list_generations(id)?;
            // Newest first, then truncated: an agent asking for twenty wants the
            // twenty most recent, not the twenty oldest.
            generations.reverse();
            generations.truncate(limit);
            Ok(json!({
                "nodeId": id.to_string(),
                "count": generations.len(),
                "generations": value(&generations),
            }))
        })
    }

    fn get_generation(&self, generation_id: &str) -> WorldResult {
        self.with(|project| {
            let id = parse_id(generation_id)?;
            match project.get_generation(id)? {
                Some(generation) => Ok(value(&generation)),
                None => Err(WobuError::new(
                    Code::Invalid,
                    "There is no generation with that id in this project.",
                )),
            }
        })
    }

    fn create_node(&self, kind: &str, name: &str, parent_id: Option<&str>) -> WorldResult {
        self.with(|project| {
            let kind = parse_kind(kind)?;
            let parent = parent_id.map(parse_id).transpose()?;
            Ok(value(&project.create_node(kind, name, parent)?))
        })
    }

    fn update_node(&self, id: &str, patch: &NodePatch) -> WorldResult {
        self.with(|project| {
            let mut node = project.get_node(parse_id(id)?)?;
            apply_patch(&mut node, patch);
            saved(project.save_node(node)?)
        })
    }

    fn link_nodes(
        &self,
        node_id: &str,
        to_id: &str,
        role: &str,
        weight: Option<f32>,
    ) -> WorldResult {
        self.with(|project| {
            let node = parse_id(node_id)?;
            let to = parse_id(to_id)?;
            let role = parse_role(role)?;
            saved(project.add_node_link(node, to, role, weight, None)?)
        })
    }
}

/// Only the fields that were named. See [`NodePatch`] on why absent is not
/// "clear it", and on why the generated description is not here.
fn apply_patch(node: &mut Node, patch: &NodePatch) {
    if let Some(name) = &patch.name {
        node.name = name.clone();
    }
    if let Some(summary) = &patch.summary {
        node.summary = summary.clone();
    }
    if let Some(notes) = &patch.notes_raw {
        node.notes_raw = notes.clone();
    }
    if let Some(tags) = &patch.tags {
        node.tags = tags.clone();
    }
    if let Some(attributes) = &patch.attributes {
        // Merged rather than replaced: an agent that set one attribute should
        // not silently drop the six the user filled in by hand.
        for (key, value) in attributes {
            node.attributes.insert(key.clone(), value.clone());
        }
    }
}

fn preset_for(
    nodes: &[Node],
    subject: Id,
    preset: Option<&str>,
) -> CommandResult<&'static wobu_core::Preset> {
    let node =
        nodes.iter().find(|node| node.id == subject).ok_or_else(|| no_such_subject(subject))?;
    Ok(preset.and_then(wobu_core::preset).unwrap_or_else(|| default_preset(node.kind)))
}

fn no_such_subject(id: Id) -> WobuError {
    WobuError::new(Code::NoSuchNode, "That entity is not in this project.").with_detail(id.to_string())
}

/* ── persistence ──────────────────────────────────────────────────────────── */

fn save(path: &Path, stored: &Stored) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| io_error("create", path, error))?;
    }
    let json = serde_json::to_string_pretty(stored)
        .map_err(|error| WobuError::new(Code::Internal, error.to_string()))?;
    std::fs::write(path, json).map_err(|error| io_error("write", path, error))?;
    // The token is in this file. `0600` is the same protection `machine.rs`
    // gives an endpoint, and here it is doing more work.
    restrict(path).map_err(|error| io_error("protect", path, error))
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> WobuError {
    WobuError::new(Code::Io, format!("Could not {action} Wobu's agent-access settings."))
        .with_detail(format!("{}: {error}", path.display()))
}

#[cfg(unix)]
fn restrict(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/* ── startup ──────────────────────────────────────────────────────────────── */

/// Hand the module its `AppHandle` and the project slot, and start the listener
/// if — and only if — the settings say a person turned it on.
///
/// Called from `setup`, which is the first moment either exists. Nothing before
/// this point can open a socket, because nothing before it has a dispatcher.
pub fn init(app: &AppHandle, world: AppState) {
    let state = app.state::<McpState>();
    *state.wiring.lock() = Some(Wiring { app: app.clone(), world });

    if !state.stored.read().server.enabled {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = app.state::<McpState>().apply_server().await {
            // Not fatal and not a dialog: the pane shows `error` beside the
            // switch, which is where somebody would look.
            diag::error(format!("mcp: {}", error.message));
        }
    });
}

/// Called from `RunEvent::Exit`. Closes the port and kills every child.
pub fn shut_down(app: &AppHandle) {
    app.state::<McpState>().shut_down();
}

/* ── commands ─────────────────────────────────────────────────────────────── */

#[tauri::command]
pub fn mcp_settings(state: State<'_, McpState>) -> McpView {
    state.view()
}

/// The switch. Every field is optional so the pane can change one thing without
/// restating the others — a `port` sent as `None` must not reset the port.
#[tauri::command]
pub async fn mcp_server_set(
    state: State<'_, McpState>,
    enabled: Option<bool>,
    port: Option<u16>,
    allow_writes: Option<bool>,
) -> CommandResult<McpView> {
    {
        let mut stored = state.stored.write();
        if let Some(enabled) = enabled {
            stored.server.enabled = enabled;
            // Generated on the way *on*, never on load: a Wobu that has never
            // had this enabled has no credential sitting in a file.
            if enabled && stored.server.token.is_none() {
                stored.server.token = Some(Token::generate());
            }
        }
        if let Some(port) = port {
            if port != 0 && port < 1024 {
                return Err(WobuError::new(
                    Code::Invalid,
                    "Pick a port above 1023. Wobu will not ask the operating system for a \
                     privileged one.",
                ));
            }
            stored.server.port = port;
        }
        if let Some(allow_writes) = allow_writes {
            stored.server.allow_writes = allow_writes;
            // Before the save and before the restart: this is the flag the
            // dispatcher reads per call, so it takes effect immediately even for
            // a request that is in flight behind this one.
            state.allow_writes.store(allow_writes, Ordering::SeqCst);
        }
    }
    state.save()?;
    let outcome = state.apply_server().await;
    // The view goes back either way: a failed start still changed the setting,
    // and a pane that threw away the new state would show the old switch.
    match outcome {
        Ok(()) => Ok(state.view()),
        Err(error) => Err(error),
    }
}

/// Replace the token. Every agent configured with the old one stops working,
/// which is the entire point.
#[tauri::command]
pub async fn mcp_server_token_rotate(state: State<'_, McpState>) -> CommandResult<McpView> {
    state.stored.write().server.token = Some(Token::generate());
    state.save()?;
    state.apply_server().await?;
    Ok(state.view())
}

/// The whole token, for the user to paste into their agent.
///
/// Its own command rather than a field on the view so that it crosses the
/// bridge when somebody asks for it and not on every settings render.
#[tauri::command]
pub fn mcp_server_token(state: State<'_, McpState>) -> CommandResult<String> {
    state
        .stored
        .read()
        .server
        .token
        .as_ref()
        .map(|token| token.expose().to_owned())
        .ok_or_else(|| {
            WobuError::new(Code::Invalid, "Wobu has not made an MCP token yet. Turn the server on.")
        })
}

#[tauri::command]
pub fn mcp_activity(state: State<'_, McpState>) -> Vec<Activity> {
    state.activity()
}

/* ── client commands ──────────────────────────────────────────────────────── */

#[tauri::command]
pub fn mcp_client_set(state: State<'_, McpState>, enabled: bool) -> CommandResult<McpView> {
    state.stored.write().client.enabled = enabled;
    state.save()?;
    if !enabled {
        // Not merely "will not be used next time": the processes go away.
        state.registry.shutdown();
    }
    Ok(state.view())
}

#[tauri::command]
pub fn mcp_client_server_upsert(
    state: State<'_, McpState>,
    server: ClientServerInput,
) -> CommandResult<McpView> {
    if server.command.trim().is_empty() {
        return Err(WobuError::new(Code::Invalid, "An MCP server needs a command to run."));
    }
    let name = server.name.trim();
    let name = if name.is_empty() { server.command.trim() } else { name }.to_owned();

    let id = server.id.clone().unwrap_or_else(|| wobu_core::new_id().to_string());
    {
        let mut stored = state.stored.write();
        let existing = stored.client.servers.iter().position(|entry| entry.id == id);
        // Environment overrides are not on the bridge, so an edit must carry the
        // ones already on disk forward rather than blanking them.
        let env = existing.map(|at| stored.client.servers[at].env.clone()).unwrap_or_default();
        let entry = ClientServer {
            id: id.clone(),
            name,
            command: server.command.trim().to_owned(),
            args: server.args.iter().map(|arg| arg.trim().to_owned()).collect(),
            env,
            enabled: server.enabled,
        };
        match existing {
            Some(at) => stored.client.servers[at] = entry,
            None => stored.client.servers.push(entry),
        }
    }
    state.save()?;
    // Whatever was running under this id was started from the old command line.
    state.registry.disconnect(&id);
    Ok(state.view())
}

#[tauri::command]
pub fn mcp_client_server_remove(
    state: State<'_, McpState>,
    id: String,
) -> CommandResult<McpView> {
    state.stored.write().client.servers.retain(|server| server.id != id);
    state.save()?;
    state.registry.disconnect(&id);
    Ok(state.view())
}

/// Launch one configured server and report what it offers.
///
/// This is the only command that starts a process, and it refuses unless both
/// switches are on — the master one and this server's own. "Check it works"
/// must not be a way around the thing the switches are for.
#[tauri::command]
pub async fn mcp_client_server_probe(
    state: State<'_, McpState>,
    id: String,
) -> CommandResult<RemoteServer> {
    let (registry, spec) = {
        let stored = state.stored.read();
        let spec = stored
            .client
            .active()
            .find(|server| server.id == id)
            .cloned()
            .ok_or_else(|| {
                WobuError::new(
                    Code::Invalid,
                    "Enable this server, and MCP clients overall, before Wobu will run it.",
                )
            })?;
        (Arc::clone(&state.registry), spec)
    };
    diag::info(format!("mcp: launching client server {:?}", spec.command));
    registry.tools(&spec).await.map_err(|error| {
        WobuError::new(Code::Io, format!("That MCP server did not start. {error}"))
    })
}

/// Call a tool on one of the user's own servers.
///
/// Present so the surface is complete and testable from the pane; the enhance
/// and generation-planning paths are the callers this is for, and they will use
/// it rather than reaching for the registry themselves.
#[tauri::command]
pub async fn mcp_client_call(
    state: State<'_, McpState>,
    id: String,
    tool: String,
    arguments: Option<Value>,
) -> CommandResult<Value> {
    let (registry, spec) = {
        let stored = state.stored.read();
        let spec =
            stored.client.active().find(|server| server.id == id).cloned().ok_or_else(|| {
                WobuError::new(Code::Invalid, "That MCP server is not enabled.")
            })?;
        (Arc::clone(&state.registry), spec)
    };
    registry
        .call(&spec, &tool, arguments.unwrap_or_else(|| json!({})))
        .await
        .map_err(|error| WobuError::new(Code::Io, error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use wobu_mcp::config::DEFAULT_PORT;

    fn scratch(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wobu-mcp-{}-{name}-{n}.json", std::process::id()))
    }

    #[test]
    fn a_fresh_installation_has_nothing_enabled_and_no_token_on_disk() {
        // The claim the privacy policy makes, at the only layer that can make
        // it: a Wobu nobody has configured has no listener and no credential.
        let path = scratch("fresh");
        let state = McpState::load_from(path.clone());
        let view = state.view();
        assert!(!view.server.enabled);
        assert!(!view.server.running);
        assert!(!view.server.allow_writes);
        assert!(view.server.token_preview.is_none());
        assert!(!view.client.enabled);
        assert!(view.client.servers.is_empty());
        assert!(!path.exists(), "loading settings must not write a file");
    }

    #[test]
    fn an_unreadable_settings_file_is_read_as_off_rather_than_as_a_failure_to_start() {
        let path = scratch("corrupt");
        std::fs::write(&path, "{ this is not json").unwrap();
        let state = McpState::load_from(path.clone());
        assert!(!state.view().server.enabled);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_settings_file_that_predates_this_feature_stays_off() {
        // The realistic upgrade path: a file with a couple of unrelated keys.
        let path = scratch("older");
        std::fs::write(&path, r#"{"somethingElse": true}"#).unwrap();
        let state = McpState::load_from(path.clone());
        let view = state.view();
        assert!(!view.server.enabled);
        assert!(!view.client.enabled);
        assert_eq!(view.server.port, DEFAULT_PORT);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn settings_survive_a_restart_and_the_file_is_not_world_readable() {
        let path = scratch("persist");
        {
            let state = McpState::load_from(path.clone());
            state.stored.write().server.enabled = true;
            state.stored.write().server.token = Some(Token::from_raw("deadbeef"));
            state.stored.write().client.enabled = true;
            state.save().unwrap();
        }
        let reloaded = McpState::load_from(path.clone());
        assert!(reloaded.view().server.enabled);
        assert_eq!(reloaded.view().server.token_preview.as_deref(), Some("deadbe…"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "the file holding the token was group/other readable");
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_view_names_every_tool_and_marks_exactly_the_writes() {
        // The pane's disclosure is rendered from this, so a tool added to the
        // catalogue appears in the sentence the user reads without anybody
        // remembering to update it.
        let state = McpState::load_from(scratch("tools"));
        let view = state.view();
        assert_eq!(view.tools.len(), wobu_mcp::catalogue().len());
        let writes: Vec<_> =
            view.tools.iter().filter(|tool| tool.write).map(|tool| tool.name.as_str()).collect();
        assert_eq!(writes, ["create_node", "update_node", "link_nodes"]);
        assert!(view.tools.iter().all(|tool| !tool.description.is_empty()));
    }

    #[test]
    fn a_patch_touches_only_the_fields_it_names() {
        let mut node = Node::new(NodeKind::Character, "Kael").unwrap();
        node.summary = "A courier.".into();
        node.notes_raw = "handwritten".into();
        node.tags = vec!["draft".into()];
        node.attributes.insert("age".into(), json!(31));

        apply_patch(&mut node, &NodePatch { summary: Some("A smuggler.".into()), ..NodePatch::default() });

        assert_eq!(node.summary, "A smuggler.");
        assert_eq!(node.name, "Kael", "a patch without a name renamed the node");
        assert_eq!(node.notes_raw, "handwritten");
        assert_eq!(node.tags, ["draft"]);
        assert_eq!(node.attributes["age"], json!(31));
    }

    #[test]
    fn an_attribute_patch_merges_rather_than_replacing_the_map() {
        // An agent setting one fact should not drop the six somebody typed.
        let mut node = Node::new(NodeKind::Character, "Kael").unwrap();
        node.attributes.insert("age".into(), json!(31));
        node.attributes.insert("height".into(), json!("tall"));

        let mut attributes = serde_json::Map::new();
        attributes.insert("age".into(), json!(32));
        apply_patch(&mut node, &NodePatch { attributes: Some(attributes), ..NodePatch::default() });

        assert_eq!(node.attributes["age"], json!(32));
        assert_eq!(node.attributes["height"], json!("tall"));
    }

    #[test]
    fn kinds_roles_and_ids_are_rejected_with_the_list_of_what_would_have_worked() {
        let error = parse_kind("wizard").unwrap_err();
        assert_eq!(error.code, Code::Invalid);
        assert!(error.message.contains("character"), "{}", error.message);

        let error = parse_role("befriends").unwrap_err();
        assert!(error.message.contains("related_to"), "{}", error.message);

        assert!(parse_id("not-a-ulid").is_err());
        assert!(parse_id("01ARZ3NDEKTSV4RRFFQ69G5FAV").is_ok());
    }

    #[test]
    fn a_probe_refuses_a_server_that_is_not_enabled_without_launching_it() {
        // The predicate the probe command uses, pinned here because the command
        // itself needs a Tauri `State` to call.
        let mut settings = ClientSettings {
            enabled: true,
            servers: vec![ClientServer {
                id: "one".into(),
                name: "Notes".into(),
                command: "true".into(),
                enabled: false,
                ..ClientServer::default()
            }],
        };
        assert!(settings.active().next().is_none());
        settings.servers[0].enabled = true;
        assert!(settings.active().next().is_some());
        settings.enabled = false;
        assert!(settings.active().next().is_none());
    }
}
