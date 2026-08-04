//! The other direction: Wobu as an MCP *client*, talking to servers the user
//! runs.
//!
//! ## What enabling one of these actually does
//!
//! It launches a program. Not a sandboxed plugin, not a script in an
//! interpreter Wobu controls — a process, named by the user, with the arguments
//! they typed, running as them, with their environment. There is no version of
//! this feature that is smaller than that, because that is what an MCP stdio
//! server is. So the honest thing is to say so in the settings pane, default
//! every switch to off, and make the code here as boring as possible:
//!
//! - **No shell.** `tokio::process::Command` invokes the executable directly.
//!   Nothing is passed to `sh -c`, so a semicolon in a field is a character in
//!   an argument rather than a second command.
//! - **stderr goes nowhere.** A server that chatters on stderr — most of them —
//!   must not be able to fill a pipe nobody drains and deadlock itself.
//! - **The child dies with us.** `kill_on_drop`, plus an explicit kill on the
//!   way out of [`Registry::shutdown`], because a Wobu that exits leaving four
//!   language servers running is a Wobu people learn to distrust.
//! - **Every request has a deadline.** A server that accepts a call and never
//!   answers must not take a Wobu command down with it.
//!
//! ## Framing
//!
//! One JSON object per line, `\n`-delimited, in both directions. That is what
//! stdio MCP is; there is no `Content-Length` header framing here (that is LSP,
//! which MCP is often mistaken for).

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::oneshot;

use crate::config::ClientServer;
use crate::dispatch::SUPPORTED_PROTOCOL_VERSIONS;
use crate::{Error, Result};

/// How long a server gets to come up and complete the handshake. Generous,
/// because the first launch of an `npx`-shaped server downloads a package.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long any later call gets. Long enough for a tool that does real work,
/// short enough that a wedged server does not become a wedged Wobu.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// One tool a user's server offers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Kept rather than dropped so that the enhance and generation-planning
    /// paths can offer these to a model without a second round trip.
    pub input_schema: Value,
}

/// What a probe found.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteServer {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub protocol_version: String,
    pub tools: Vec<RemoteTool>,
}

/// The live connections, keyed by the configured server's id.
///
/// A `std::sync::Mutex` and not a `tokio` one, on purpose: nothing here holds
/// the lock across an `await`, and a synchronous lock is what lets
/// [`Registry::shutdown`] run from the app's exit handler, where there is no
/// runtime left to block on.
#[derive(Default)]
pub struct Registry {
    live: Mutex<HashMap<String, Arc<Connection>>>,
}

impl Registry {
    pub fn new() -> Registry {
        Registry::default()
    }

    /// The tools one configured server offers, connecting if necessary.
    pub async fn tools(&self, spec: &ClientServer) -> Result<RemoteServer> {
        let connection = self.ensure(spec).await?;
        Ok(RemoteServer {
            id: spec.id.clone(),
            name: connection.reported_name.clone(),
            version: connection.reported_version.clone(),
            protocol_version: connection.protocol_version.clone(),
            tools: connection.tools.clone(),
        })
    }

    /// Call a tool on one configured server.
    ///
    /// The caller is responsible for having checked that the user enabled this
    /// server. That check is in the shell, beside the settings that answer it,
    /// rather than here where a `ClientServer` value could have come from
    /// anywhere.
    pub async fn call(&self, spec: &ClientServer, tool: &str, arguments: Value) -> Result<Value> {
        self.ensure(spec)
            .await?
            .request("tools/call", json!({ "name": tool, "arguments": arguments }))
            .await
    }

    /// Drop one connection, killing its process. Called when a server is
    /// disabled, edited or removed.
    pub fn disconnect(&self, id: &str) {
        let taken = self.live.lock().ok().and_then(|mut live| live.remove(id));
        drop(taken);
    }

    /// Every child, gone. Synchronous and best-effort: this runs on the way out
    /// of the process, where the alternative to best-effort is a hang.
    pub fn shutdown(&self) {
        let taken: Vec<_> = self
            .live
            .lock()
            .ok()
            .map(|mut live| live.drain().map(|(_, connection)| connection).collect())
            .unwrap_or_default();
        for connection in &taken {
            connection.kill();
        }
        drop(taken);
    }

    fn existing(&self, spec: &ClientServer) -> Option<Arc<Connection>> {
        let live = self.live.lock().ok()?;
        let connection = live.get(&spec.id)?;
        // A server whose command or arguments have been edited is a different
        // server wearing the same id. Reusing the running one would mean the
        // user changing a path in Settings, seeing no error, and continuing to
        // talk to the old binary.
        (connection.signature == signature(spec)).then(|| Arc::clone(connection))
    }

    async fn ensure(&self, spec: &ClientServer) -> Result<Arc<Connection>> {
        if let Some(connection) = self.existing(spec)
            && connection.is_alive()
        {
            return Ok(connection);
        }
        self.disconnect(&spec.id);

        let connection = Arc::new(Connection::open(spec).await?);
        let mut live = self
            .live
            .lock()
            .map_err(|_| Error::Client("Wobu's MCP client registry is poisoned.".to_owned()))?;
        // A racing caller may have landed first. Keep theirs and drop ours,
        // which kills the process we just started rather than leaving it
        // parented to nothing.
        match live.get(&spec.id) {
            Some(winner) if winner.signature == connection.signature => Ok(Arc::clone(winner)),
            _ => {
                live.insert(spec.id.clone(), Arc::clone(&connection));
                Ok(connection)
            }
        }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// What a connection was opened against. Two specs that differ in any of these
/// are two different servers.
fn signature(spec: &ClientServer) -> String {
    let mut parts = vec![spec.command.clone()];
    parts.extend(spec.args.iter().cloned());
    for (key, value) in &spec.env {
        parts.push(format!("{key}={value}"));
    }
    parts.join("\u{1f}")
}

struct Connection {
    signature: String,
    reported_name: String,
    reported_version: Option<String>,
    protocol_version: String,
    tools: Vec<RemoteTool>,
    stdin: tokio::sync::Mutex<ChildStdin>,
    child: Mutex<Child>,
    /// Shared with the reader task, which is the half that resolves them. One
    /// allocation and not two: an earlier draft kept a private map here and
    /// routed into a different one, which is a client that never sees an answer.
    pending: Pending,
    next_id: AtomicU64,
    reader: tokio::task::JoinHandle<()>,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

impl Connection {
    async fn open(spec: &ClientServer) -> Result<Connection> {
        if spec.command.trim().is_empty() {
            return Err(Error::Client("This MCP server has no command to run.".to_owned()));
        }

        let mut command = Command::new(&spec.command);
        command
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Deliberately discarded rather than piped: nothing drains it, and
            // an undrained pipe is a server that blocks the moment it logs.
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for (key, value) in &spec.env {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(|error| {
            Error::Client(format!("Could not start {:?}: {error}", spec.command))
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Client("That server gave Wobu no stdin.".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Client("That server gave Wobu no stdout.".to_owned()))?;

        let pending: Pending = Arc::default();
        let routed = Arc::clone(&pending);
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                // Anything without a numeric id is a notification or a request
                // *from* the server. This client advertises no capabilities, so
                // there is nothing it could legitimately be asked for, and the
                // right answer to an unsolicited message is to ignore it.
                let Some(id) = message.get("id").and_then(Value::as_u64) else { continue };
                let waiting = routed.lock().ok().and_then(|mut pending| pending.remove(&id));
                if let Some(waiting) = waiting {
                    let _ = waiting.send(message);
                }
            }
            // The pipe closed: the server exited. Wake everybody waiting rather
            // than letting them sit out their deadlines one by one.
            if let Ok(mut pending) = routed.lock() {
                pending.clear();
            }
        });

        let connection = Connection {
            signature: signature(spec),
            reported_name: spec.name.clone(),
            reported_version: None,
            protocol_version: SUPPORTED_PROTOCOL_VERSIONS[0].to_owned(),
            tools: Vec::new(),
            stdin: tokio::sync::Mutex::new(stdin),
            child: Mutex::new(child),
            pending,
            next_id: AtomicU64::new(1),
            reader,
        };
        connection.handshake(spec).await
    }

    async fn handshake(mut self, spec: &ClientServer) -> Result<Connection> {
        let initialize = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            self.request_inner(
                "initialize",
                json!({
                    "protocolVersion": SUPPORTED_PROTOCOL_VERSIONS[0],
                    "capabilities": {},
                    "clientInfo": { "name": "wobu", "version": env!("CARGO_PKG_VERSION") },
                }),
            ),
        )
        .await
        .map_err(|_| {
            Error::Client(format!("{} did not answer Wobu's handshake in time.", spec.name))
        })??;

        self.protocol_version = initialize
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(SUPPORTED_PROTOCOL_VERSIONS[0])
            .to_owned();
        if let Some(info) = initialize.get("serverInfo") {
            if let Some(name) = info.get("name").and_then(Value::as_str) {
                self.reported_name = name.to_owned();
            }
            self.reported_version = info.get("version").and_then(Value::as_str).map(str::to_owned);
        }

        self.notify("notifications/initialized", json!({})).await?;

        let listing =
            tokio::time::timeout(HANDSHAKE_TIMEOUT, self.request_inner("tools/list", json!({})))
                .await
                .map_err(|_| Error::Client(format!("{} never listed its tools.", spec.name)))??;
        self.tools = listing
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| tools.iter().map(remote_tool).collect())
            .unwrap_or_default();

        Ok(self)
    }

    fn is_alive(&self) -> bool {
        if self.reader.is_finished() {
            return false;
        }
        match self.child.lock() {
            Ok(mut child) => matches!(child.try_wait(), Ok(None)),
            Err(_) => false,
        }
    }

    fn kill(&self) {
        self.reader.abort();
        if let Ok(mut child) = self.child.lock() {
            let _ = child.start_kill();
        }
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        tokio::time::timeout(CALL_TIMEOUT, self.request_inner(method, params))
            .await
            .map_err(|_| Error::Client(format!("{method} timed out.")))?
    }

    async fn request_inner(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| Error::Client("Wobu's MCP request map is poisoned.".to_owned()))?
            .insert(id, tx);

        let sent = self
            .write_line(&json!({
                "jsonrpc": crate::jsonrpc::VERSION,
                "id": id,
                "method": method,
                "params": params,
            }))
            .await;
        if let Err(error) = sent {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id);
            }
            return Err(error);
        }

        let message = rx.await.map_err(|_| {
            Error::Client(format!("That MCP server stopped before answering {method}."))
        })?;
        if let Some(error) = message.get("error") {
            let text = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("that MCP server refused the request");
            return Err(Error::Client(text.to_owned()));
        }
        Ok(message.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write_line(&json!({
            "jsonrpc": crate::jsonrpc::VERSION,
            "method": method,
            "params": params,
        }))
        .await
    }

    async fn write_line(&self, message: &Value) -> Result<()> {
        let mut line = serde_json::to_vec(message)
            .map_err(|error| Error::Client(format!("could not encode a request: {error}")))?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&line)
            .await
            .map_err(|error| Error::Client(format!("could not write to that server: {error}")))?;
        stdin
            .flush()
            .await
            .map_err(|error| Error::Client(format!("could not write to that server: {error}")))
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.kill();
    }
}

fn remote_tool(value: &Value) -> RemoteTool {
    static EMPTY_SCHEMA: LazyLock<Value> = LazyLock::new(|| json!({ "type": "object" }));
    RemoteTool {
        name: value.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
        title: value.get("title").and_then(Value::as_str).map(str::to_owned),
        description: value.get("description").and_then(Value::as_str).map(str::to_owned),
        input_schema: value.get("inputSchema").cloned().unwrap_or_else(|| EMPTY_SCHEMA.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(command: &str, args: &[&str]) -> ClientServer {
        ClientServer {
            id: "one".into(),
            name: "Test server".into(),
            command: command.into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: Vec::new(),
            enabled: true,
        }
    }

    #[test]
    fn editing_a_command_changes_the_signature_so_the_old_process_is_not_reused() {
        let original = spec("mcp-notes", &["--root", "/a"]);
        assert_eq!(signature(&original), signature(&spec("mcp-notes", &["--root", "/a"])));
        assert_ne!(signature(&original), signature(&spec("mcp-notes", &["--root", "/b"])));
        assert_ne!(signature(&original), signature(&spec("mcp-other", &["--root", "/a"])));

        // Environment counts too: a server pointed at a different account by a
        // variable is a different server.
        let mut with_env = original.clone();
        with_env.env.push(("TOKEN".into(), "second".into()));
        assert_ne!(signature(&original), signature(&with_env));
    }

    #[test]
    fn a_tool_listing_survives_a_server_that_omits_the_optional_fields() {
        let bare = remote_tool(&json!({ "name": "search" }));
        assert_eq!(bare.name, "search");
        assert_eq!(bare.description, None);
        assert_eq!(bare.input_schema["type"], "object");

        let full = remote_tool(&json!({
            "name": "search", "title": "Search", "description": "Find things",
            "inputSchema": { "type": "object", "properties": { "q": { "type": "string" } } },
        }));
        assert_eq!(full.title.as_deref(), Some("Search"));
        assert_eq!(full.input_schema["properties"]["q"]["type"], "string");
    }

    #[tokio::test]
    async fn a_command_that_does_not_exist_reports_the_command_rather_than_an_errno() {
        let registry = Registry::new();
        let error = registry
            .tools(&spec("wobu-there-is-no-such-binary-here", &[]))
            .await
            .expect_err("a missing binary cannot connect");
        let text = error.to_string();
        assert!(text.contains("wobu-there-is-no-such-binary-here"), "{text}");
    }

    #[tokio::test]
    async fn a_process_that_says_nothing_is_given_up_on_rather_than_waited_on_forever() {
        // `true` exits immediately, closing stdout, which is the ordinary shape
        // of "that is not an MCP server". The handshake has to end, and it has
        // to end without the 30-second deadline being what ends it.
        let registry = Registry::new();
        let began = std::time::Instant::now();
        let error = registry.tools(&spec("true", &[])).await.expect_err("no handshake");
        assert!(began.elapsed() < HANDSHAKE_TIMEOUT, "waited {:?}", began.elapsed());
        assert!(error.to_string().contains("stopped before answering"), "{error}");
    }

    #[tokio::test]
    async fn nothing_is_launched_for_a_server_the_user_has_not_enabled() {
        // The registry does not police this — the shell does, beside the
        // settings that answer it — so what this pins is the predicate every
        // caller is required to use.
        let mut disabled = spec("true", &[]);
        disabled.enabled = false;
        let settings = crate::config::ClientSettings { enabled: true, servers: vec![disabled] };
        assert_eq!(settings.active().count(), 0);
    }
}
