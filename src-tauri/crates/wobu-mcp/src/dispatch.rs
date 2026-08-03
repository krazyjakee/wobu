//! One MCP message in, at most one answer out — with no socket in sight.
//!
//! The transport is in `server.rs` and is thirty lines of hyper. Everything
//! that decides *what happens* is here, synchronous and free of I/O, which is
//! why the tests at the bottom can drive the entire protocol — handshake, tool
//! listing, the write gate, the audit trail — against a fake world with no
//! runtime, no port and no project on disk.
//!
//! ## Errors are results, not faults
//!
//! A JSON-RPC error means the request was wrong: unknown method, bad params,
//! malformed message. A tool that ran and could not do the thing — no project
//! open, no such node, the share unplugged — returns a *successful* response
//! whose result carries `isError: true`. That is MCP's rule and it is the right
//! one: a model can read and act on the second, and generally cannot on the
//! first. Getting it backwards makes an agent give up on a world that just
//! needs opening.
//!
//! ## The write gate
//!
//! Two independent things have to be true before a write tool runs: writes are
//! enabled *now* (an `AtomicBool` the settings pane flips, so unticking the box
//! stops the next call rather than the next restart), and the tool is in the
//! catalogue as a write. With writes off the tool is not advertised at all, and
//! a client that calls it anyway from a stale listing is told plainly that the
//! user has not granted writes — not "unknown tool", which would send it
//! guessing at names.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::jsonrpc::{
    INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, Request, Response,
};
use crate::tools;
use crate::world::{NodePatch, World, WorldError, WorldResult};

/// The revisions this server will speak. Newest first; the first entry is what
/// is offered to a client that asks for something unrecognised.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// What happened, for the person who opened the door.
///
/// Deliberately without a timestamp: the shell stamps these, so this crate does
/// not acquire a clock, and more importantly so that the time in the log is the
/// app's own rather than a second source that could disagree with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    pub tool: String,
    pub write: bool,
    pub ok: bool,
    /// The refusal or failure, when there was one. Never the arguments: a
    /// prompt an agent compiled is the user's writing, and an audit line is not
    /// the place to copy it.
    pub detail: Option<String>,
}

/// Told about every tool call before its answer is sent.
///
/// Not optional and not a callback the caller may skip: [`Dispatcher::new`]
/// requires one. The argument in `lib.rs` is that a person who turns this on is
/// entitled to see what came through, and an audit hook that a future refactor
/// could forget to install would not deliver that.
pub trait Audit: Send + Sync + 'static {
    fn record(&self, entry: CallRecord);
}

/// An [`Audit`] that does nothing, for tests and for a caller that genuinely
/// has nowhere to put one.
pub struct Silent;

impl Audit for Silent {
    fn record(&self, _entry: CallRecord) {}
}

pub struct Dispatcher {
    world: Arc<dyn World>,
    /// Read on every write call rather than captured at start, so that
    /// unticking the box in Settings takes effect on the next request instead
    /// of the next launch.
    allow_writes: Arc<AtomicBool>,
    audit: Arc<dyn Audit>,
    server_name: String,
    server_version: String,
}

impl Dispatcher {
    pub fn new(
        world: Arc<dyn World>,
        allow_writes: Arc<AtomicBool>,
        audit: Arc<dyn Audit>,
        server_name: impl Into<String>,
        server_version: impl Into<String>,
    ) -> Dispatcher {
        Dispatcher {
            world,
            allow_writes,
            audit,
            server_name: server_name.into(),
            server_version: server_version.into(),
        }
    }

    pub fn writes_allowed(&self) -> bool {
        self.allow_writes.load(Ordering::SeqCst)
    }

    /// `None` for a notification, which must never be answered.
    pub fn handle(&self, request: &Request) -> Option<Response> {
        if request.is_notification() {
            return None;
        }
        let id = request.id.clone().unwrap_or(Value::Null);
        if !request.jsonrpc.is_empty() && request.jsonrpc != crate::jsonrpc::VERSION {
            return Some(Response::error(
                id,
                INVALID_REQUEST,
                format!("this endpoint speaks JSON-RPC 2.0, not {:?}", request.jsonrpc),
            ));
        }
        Some(match request.method.as_str() {
            "initialize" => Response::result(id, self.initialize(request)),
            "ping" => Response::result(id, json!({})),
            "tools/list" => Response::result(id, self.tools_list()),
            "tools/call" => match self.tools_call(request) {
                Ok(result) => Response::result(id, result),
                Err(error) => Response::error(id, error.0, error.1),
            },
            "resources/list" => Response::result(id, resources_list()),
            "resources/templates/list" => Response::result(id, resource_templates()),
            "resources/read" => match self.resources_read(request) {
                Ok(result) => Response::result(id, result),
                Err(error) => Response::error(id, error.0, error.1),
            },
            // `prompts/list` and `completion/complete` are asked for
            // speculatively by several clients at startup. Answering "not found"
            // is correct and is what the capability advertisement already said.
            other => Response::error(id, METHOD_NOT_FOUND, format!("no such method: {other}")),
        })
    }

    fn initialize(&self, request: &Request) -> Value {
        let requested = request.params().get("protocolVersion").and_then(Value::as_str);
        let version = requested
            .filter(|asked| SUPPORTED_PROTOCOL_VERSIONS.contains(asked))
            .unwrap_or(SUPPORTED_PROTOCOL_VERSIONS[0]);
        json!({
            "protocolVersion": version,
            "capabilities": {
                // `listChanged` is false and stays false until there is a
                // notification channel to deliver it on. Advertising a change
                // notification over a request/response transport would be a
                // promise this server cannot keep.
                "tools": { "listChanged": false },
                "resources": { "subscribe": false, "listChanged": false },
            },
            "serverInfo": { "name": self.server_name, "version": self.server_version },
            "instructions": self.instructions(),
        })
    }

    fn instructions(&self) -> String {
        let mut text = String::from(
            "This is one person's Wobu world-building project, open on their own machine. \
             Read it with world_overview first. Ids are ULIDs and come from list_nodes or \
             search_nodes. compile_prompt and resolve_influence explain what a generation \
             would send without generating anything or spending anything.",
        );
        if self.writes_allowed() {
            text.push_str(
                " Writes are enabled: create_node, update_node and link_nodes change files in \
                 the user's project folder, and the user sees every call. Prefer notesRaw over \
                 rewriting a summary somebody wrote by hand.",
            );
        } else {
            text.push_str(" This connection is read-only. No tool here can change anything.");
        }
        text
    }

    fn tools_list(&self) -> Value {
        let tools: Vec<Value> =
            tools::advertised(self.writes_allowed()).map(tools::Tool::describe).collect();
        json!({ "tools": tools })
    }

    fn tools_call(&self, request: &Request) -> std::result::Result<Value, (i64, String)> {
        let params = request.params();
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((INVALID_PARAMS, "tools/call needs a tool name".to_owned()))?;
        let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let arguments = arguments.as_object().cloned().unwrap_or_default();

        let Some(tool) = tools::find(name) else {
            // A JSON-RPC error rather than an `isError` result: the request
            // itself is wrong, and a model that is told "this failed" will retry
            // a name that will never exist.
            return Err((INVALID_PARAMS, format!("no such tool: {name}")));
        };

        if tool.write && !self.writes_allowed() {
            let detail = "This Wobu has not granted write access over MCP. \
                          The user can turn it on in Settings → Agent access, under \
                          \"Let a connected agent change this world\"."
                .to_owned();
            self.audit.record(CallRecord {
                tool: name.to_owned(),
                write: true,
                ok: false,
                detail: Some("refused: writes are not enabled".to_owned()),
            });
            return Ok(error_result(&detail, false));
        }

        let outcome = self.run(name, &arguments);
        match outcome {
            Ok(value) => {
                self.audit.record(CallRecord {
                    tool: name.to_owned(),
                    write: tool.write,
                    ok: true,
                    detail: None,
                });
                Ok(ok_result(value))
            }
            Err(error) => {
                self.audit.record(CallRecord {
                    tool: name.to_owned(),
                    write: tool.write,
                    ok: false,
                    detail: Some(error.message.clone()),
                });
                Ok(error_result(&error.message, error.retryable))
            }
        }
    }

    fn run(&self, name: &str, args: &Args) -> WorldResult {
        match name {
            "world_overview" => self.world.overview(),
            "list_nodes" => self.world.list_nodes(text(args, "kind")),
            "get_node" => self.world.get_node(required(args, "id")?),
            "search_nodes" => self.world.search_nodes(required(args, "query")?, limit(args, 20)),
            "get_node_links" => self.world.node_links(required(args, "id")?),
            "resolve_influence" => {
                self.world.influence_stack(required(args, "subjectId")?, text(args, "preset"))
            }
            "compile_prompt" => {
                self.world.compile_prompt(required(args, "subjectId")?, text(args, "preset"))
            }
            "list_generations" => {
                self.world.list_generations(required(args, "nodeId")?, limit(args, 20))
            }
            "get_generation" => self.world.get_generation(required(args, "generationId")?),
            "create_node" => self.world.create_node(
                required(args, "kind")?,
                required(args, "name")?,
                text(args, "parentId"),
            ),
            "update_node" => {
                let patch: NodePatch = args
                    .get("patch")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|error| WorldError::new(format!("that patch is not usable: {error}")))?
                    .unwrap_or_default();
                if patch.is_empty() {
                    return Err(WorldError::new(
                        "the patch names no fields, so there is nothing to change",
                    ));
                }
                self.world.update_node(required(args, "id")?, &patch)
            }
            "link_nodes" => self.world.link_nodes(
                required(args, "nodeId")?,
                required(args, "toId")?,
                required(args, "role")?,
                args.get("weight").and_then(Value::as_f64).map(|weight| weight as f32),
            ),
            other => Err(WorldError::new(format!("no such tool: {other}"))),
        }
    }

    fn resources_read(&self, request: &Request) -> std::result::Result<Value, (i64, String)> {
        let uri = request
            .params()
            .get("uri")
            .and_then(Value::as_str)
            .ok_or((INVALID_PARAMS, "resources/read needs a uri".to_owned()))?;

        let result = match uri {
            "wobu://project" => self.world.overview(),
            "wobu://nodes" => self.world.list_nodes(None),
            other => match other.strip_prefix("wobu://node/") {
                Some(id) => self.world.get_node(id),
                None => return Err((INVALID_PARAMS, format!("no such resource: {other}"))),
            },
        };

        let text = match result {
            Ok(value) => pretty(&value),
            // A resource read has no `isError` channel, so a closed project
            // does have to come back as a fault here. `INTERNAL_ERROR` rather
            // than a made-up code, because a client cannot act on either.
            Err(error) => return Err((INTERNAL_ERROR, error.message)),
        };
        Ok(json!({
            "contents": [{ "uri": uri, "mimeType": "application/json", "text": text }],
        }))
    }
}

/// The `arguments` object of a `tools/call`, already unwrapped.
type Args = serde_json::Map<String, Value>;

fn text<'a>(args: &'a Args, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty())
}

fn required<'a>(args: &'a Args, key: &str) -> std::result::Result<&'a str, WorldError> {
    text(args, key)
        .ok_or_else(|| WorldError::new(format!("{key} is required and must be a non-empty string")))
}

/// Clamped rather than trusted. An agent that asks for a million rows gets two
/// hundred: the ceiling is there because the answer is serialised into a model's
/// context, and a reply nobody can read is worse than a short one.
fn limit(args: &Args, default: usize) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .map_or(default, |asked| usize::try_from(asked).unwrap_or(usize::MAX).clamp(1, 200))
}

fn resources_list() -> Value {
    json!({
        "resources": [
            {
                "uri": "wobu://project",
                "name": "Project overview",
                "description": "The open world's name, folder and node counts.",
                "mimeType": "application/json",
            },
            {
                "uri": "wobu://nodes",
                "name": "All nodes",
                "description": "Every entity in the open world, as summaries.",
                "mimeType": "application/json",
            },
        ]
    })
}

fn resource_templates() -> Value {
    json!({
        "resourceTemplates": [{
            "uriTemplate": "wobu://node/{id}",
            "name": "One node",
            "description": "A single entity in full, by ULID.",
            "mimeType": "application/json",
        }]
    })
}

/// Both halves of a `tools/call` result: text for a model that only reads text,
/// and `structuredContent` for a client that can use the real shape. The same
/// value, serialised twice, rather than a summary and a payload that could
/// disagree.
fn ok_result(value: Value) -> Value {
    let text = pretty(&value);
    let mut result = json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false,
    });
    if value.is_object() {
        result["structuredContent"] = value;
    }
    result
}

fn error_result(message: &str, retryable: bool) -> Value {
    let suffix = if retryable { " This may work if you try again." } else { "" };
    json!({
        "content": [{ "type": "text", "text": format!("{message}{suffix}") }],
        "isError": true,
    })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<CallRecord>>);

    impl Recorder {
        fn entries(&self) -> Vec<CallRecord> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Audit for Recorder {
        fn record(&self, entry: CallRecord) {
            self.0.lock().unwrap().push(entry);
        }
    }

    #[derive(Default)]
    struct FakeWorld {
        writes: Mutex<Vec<String>>,
        closed: bool,
    }

    impl FakeWorld {
        fn guard(&self) -> std::result::Result<(), WorldError> {
            if self.closed {
                Err(WorldError::retryable("No project is open in Wobu."))
            } else {
                Ok(())
            }
        }
    }

    impl World for FakeWorld {
        fn overview(&self) -> WorldResult {
            self.guard()?;
            Ok(json!({ "name": "Ashfall", "nodeCount": 2 }))
        }
        fn list_nodes(&self, kind: Option<&str>) -> WorldResult {
            self.guard()?;
            Ok(json!({ "kind": kind, "nodes": [] }))
        }
        fn get_node(&self, id: &str) -> WorldResult {
            self.guard()?;
            if id == "missing" {
                return Err(WorldError::new("no node with id missing"));
            }
            Ok(json!({ "id": id }))
        }
        fn search_nodes(&self, query: &str, limit: usize) -> WorldResult {
            self.guard()?;
            Ok(json!({ "query": query, "limit": limit }))
        }
        fn node_links(&self, id: &str) -> WorldResult {
            self.guard()?;
            Ok(json!({ "id": id }))
        }
        fn influence_stack(&self, subject_id: &str, _preset: Option<&str>) -> WorldResult {
            self.guard()?;
            Ok(json!({ "subjectId": subject_id }))
        }
        fn compile_prompt(&self, subject_id: &str, _preset: Option<&str>) -> WorldResult {
            self.guard()?;
            Ok(json!({ "subjectId": subject_id, "prompt": "a windswept ridge" }))
        }
        fn list_generations(&self, node_id: &str, _limit: usize) -> WorldResult {
            self.guard()?;
            Ok(json!({ "nodeId": node_id, "generations": [] }))
        }
        fn get_generation(&self, generation_id: &str) -> WorldResult {
            self.guard()?;
            Ok(json!({ "id": generation_id }))
        }
        fn create_node(&self, kind: &str, name: &str, _parent: Option<&str>) -> WorldResult {
            self.guard()?;
            self.writes.lock().unwrap().push(format!("create {kind} {name}"));
            Ok(json!({ "id": "new", "name": name }))
        }
        fn update_node(&self, id: &str, patch: &NodePatch) -> WorldResult {
            self.guard()?;
            self.writes.lock().unwrap().push(format!("update {id}"));
            Ok(json!({ "id": id, "name": patch.name }))
        }
        fn link_nodes(
            &self,
            node_id: &str,
            to_id: &str,
            role: &str,
            _weight: Option<f32>,
        ) -> WorldResult {
            self.guard()?;
            self.writes.lock().unwrap().push(format!("link {node_id} {to_id} {role}"));
            Ok(json!({ "ok": true }))
        }
    }

    struct Fixture {
        dispatcher: Dispatcher,
        world: Arc<FakeWorld>,
        audit: Arc<Recorder>,
        writes: Arc<AtomicBool>,
    }

    fn fixture(allow_writes: bool) -> Fixture {
        fixture_with(FakeWorld::default(), allow_writes)
    }

    fn fixture_with(world: FakeWorld, allow_writes: bool) -> Fixture {
        let world = Arc::new(world);
        let audit = Arc::new(Recorder::default());
        let writes = Arc::new(AtomicBool::new(allow_writes));
        let dispatcher = Dispatcher::new(
            Arc::clone(&world) as Arc<dyn World>,
            Arc::clone(&writes),
            Arc::clone(&audit) as Arc<dyn Audit>,
            "wobu",
            "0.1.0",
        );
        Fixture { dispatcher, world, audit, writes }
    }

    fn request(method: &str, params: Value) -> Request {
        serde_json::from_value(json!({
            "jsonrpc": "2.0", "id": 1, "method": method, "params": params,
        }))
        .unwrap()
    }

    fn call(fixture: &Fixture, tool: &str, arguments: Value) -> Value {
        let request = request("tools/call", json!({ "name": tool, "arguments": arguments }));
        let response = fixture.dispatcher.handle(&request).expect("a call is not a notification");
        serde_json::to_value(response).unwrap()
    }

    #[test]
    fn the_handshake_answers_in_the_version_the_client_asked_for_when_it_is_one_we_speak() {
        let fixture = fixture(false);
        for asked in SUPPORTED_PROTOCOL_VERSIONS {
            let response = fixture
                .dispatcher
                .handle(&request("initialize", json!({ "protocolVersion": asked })))
                .unwrap();
            assert_eq!(response.result.unwrap()["protocolVersion"], *asked);
        }

        // Something from the future, or from a client that made one up: offer
        // ours rather than echoing a version this code has never seen.
        let response = fixture
            .dispatcher
            .handle(&request("initialize", json!({ "protocolVersion": "3000-01-01" })))
            .unwrap();
        assert_eq!(
            response.result.unwrap()["protocolVersion"],
            SUPPORTED_PROTOCOL_VERSIONS[0]
        );
    }

    #[test]
    fn the_initialized_notification_is_not_answered() {
        // A response here is an unsolicited message on the very first exchange,
        // and clients differ in how gracefully they survive one.
        let fixture = fixture(false);
        let notification: Request = serde_json::from_value(json!({
            "jsonrpc": "2.0", "method": "notifications/initialized",
        }))
        .unwrap();
        assert!(fixture.dispatcher.handle(&notification).is_none());
    }

    #[test]
    fn with_writes_off_the_write_tools_are_not_even_advertised() {
        let fixture = fixture(false);
        let response = fixture.dispatcher.handle(&request("tools/list", json!({}))).unwrap();
        let listed: Vec<String> = response.result.unwrap()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect();

        for write in ["create_node", "update_node", "link_nodes"] {
            assert!(!listed.contains(&write.to_owned()), "{write} was advertised with writes off");
        }
        assert!(listed.contains(&"get_node".to_owned()));
        // And the instructions say so, so a model does not spend a turn finding
        // out by being refused.
        let initialize =
            fixture.dispatcher.handle(&request("initialize", json!({}))).unwrap().result.unwrap();
        assert!(
            initialize["instructions"].as_str().unwrap().contains("read-only"),
            "{initialize}"
        );
    }

    #[test]
    fn a_write_called_with_writes_off_is_refused_and_the_world_is_never_touched() {
        let fixture = fixture(false);
        let answer = call(&fixture, "create_node", json!({ "kind": "character", "name": "Vashk" }));

        assert_eq!(answer["result"]["isError"], true);
        let text = answer["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Settings"), "the refusal has to say where the switch is: {text}");
        assert!(fixture.world.writes.lock().unwrap().is_empty(), "a refused write still wrote");

        // Refusals are audited too. A user asking "is something poking at my
        // world" deserves to see the attempts, not just the successes.
        let audited = fixture.audit.entries();
        assert_eq!(audited.len(), 1);
        assert!(!audited[0].ok);
        assert!(audited[0].write);
    }

    #[test]
    fn turning_writes_on_takes_effect_on_the_next_call_rather_than_the_next_launch() {
        let fixture = fixture(false);
        assert_eq!(call(&fixture, "create_node", json!({"kind":"prop","name":"Lamp"}))["result"]
            ["isError"], true);

        fixture.writes.store(true, Ordering::SeqCst);

        let answer = call(&fixture, "create_node", json!({ "kind": "prop", "name": "Lamp" }));
        assert_eq!(answer["result"]["isError"], false);
        assert_eq!(fixture.world.writes.lock().unwrap().as_slice(), ["create prop Lamp"]);

        // And off again, mid-session, with no restart in between.
        fixture.writes.store(false, Ordering::SeqCst);
        assert_eq!(
            call(&fixture, "update_node", json!({ "id": "x", "patch": { "summary": "s" } }))
                ["result"]["isError"],
            true
        );
        assert_eq!(fixture.world.writes.lock().unwrap().len(), 1, "a write slipped through");
    }

    #[test]
    fn a_tool_that_ran_and_failed_is_a_successful_response_carrying_is_error() {
        // The distinction the module header is about: a model can recover from
        // "no such node" and generally cannot from a JSON-RPC fault.
        let fixture = fixture(true);
        let answer = call(&fixture, "get_node", json!({ "id": "missing" }));
        assert!(answer.get("error").is_none(), "{answer}");
        assert_eq!(answer["result"]["isError"], true);
        assert!(answer["result"]["content"][0]["text"].as_str().unwrap().contains("missing"));
    }

    #[test]
    fn a_closed_project_reads_as_retryable_rather_than_as_a_dead_end() {
        let fixture = fixture_with(FakeWorld { closed: true, ..FakeWorld::default() }, true);
        let answer = call(&fixture, "world_overview", json!({}));
        let text = answer["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("No project is open"), "{text}");
        assert!(text.contains("try again"), "a retryable failure should say so: {text}");
    }

    #[test]
    fn an_unknown_tool_is_a_protocol_fault_and_not_a_tool_failure() {
        let fixture = fixture(true);
        let answer = call(&fixture, "delete_everything", json!({}));
        assert_eq!(answer["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn a_successful_call_carries_the_same_data_as_text_and_as_structure() {
        let fixture = fixture(true);
        let answer = call(&fixture, "compile_prompt", json!({ "subjectId": "01ABC" }));
        let structured = &answer["result"]["structuredContent"];
        assert_eq!(structured["prompt"], "a windswept ridge");
        let text: Value =
            serde_json::from_str(answer["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(&text, structured, "the two halves of one answer disagreed");
    }

    #[test]
    fn an_empty_patch_is_refused_rather_than_written_as_a_no_op_save() {
        // A no-op save still rewrites the file, bumps `updated_at` and can lose
        // a race with a collaborator — an expensive way to do nothing.
        let fixture = fixture(true);
        let answer = call(&fixture, "update_node", json!({ "id": "01ABC", "patch": {} }));
        assert_eq!(answer["result"]["isError"], true);
        assert!(fixture.world.writes.lock().unwrap().is_empty());
    }

    #[test]
    fn a_missing_required_argument_is_reported_by_name() {
        let fixture = fixture(true);
        let answer = call(&fixture, "get_node", json!({}));
        assert_eq!(answer["result"]["isError"], true);
        assert!(answer["result"]["content"][0]["text"].as_str().unwrap().contains("id"));
    }

    #[test]
    fn a_search_limit_is_clamped_rather_than_trusted() {
        let fixture = fixture(true);
        let answer =
            call(&fixture, "search_nodes", json!({ "query": "ash", "limit": 100_000_000 }));
        assert_eq!(answer["result"]["structuredContent"]["limit"], 200);
    }

    #[test]
    fn resources_read_serves_one_node_by_uri_and_refuses_anything_else() {
        let fixture = fixture(true);
        let ok = fixture
            .dispatcher
            .handle(&request("resources/read", json!({ "uri": "wobu://node/01ABC" })))
            .unwrap();
        let contents = &ok.result.unwrap()["contents"][0];
        assert_eq!(contents["mimeType"], "application/json");
        assert!(contents["text"].as_str().unwrap().contains("01ABC"));

        // Not a path traversal so much as a reminder that the uri space is a
        // whitelist rather than a router.
        let refused = fixture
            .dispatcher
            .handle(&request("resources/read", json!({ "uri": "file:///etc/passwd" })))
            .unwrap();
        assert_eq!(refused.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn every_read_tool_in_the_catalogue_is_reachable() {
        // The catalogue and the match arm in `run` are two lists that have to
        // agree, and nothing in the type system makes them.
        let fixture = fixture(true);
        for tool in tools::catalogue() {
            let answer = call(&fixture, tool.name, json!({}));
            let text = answer["result"]["content"][0]["text"].as_str().unwrap_or_default();
            assert!(!text.starts_with("no such tool"), "{} is advertised but unrouted", tool.name);
        }
    }

    #[test]
    fn a_message_that_is_not_json_rpc_two_is_rejected() {
        let fixture = fixture(true);
        let request: Request = serde_json::from_value(json!({
            "jsonrpc": "1.0", "id": 1, "method": "ping",
        }))
        .unwrap();
        let response = fixture.dispatcher.handle(&request).unwrap();
        assert_eq!(response.error.unwrap().code, INVALID_REQUEST);
    }
}
