//! The listener, through a real socket.
//!
//! `dispatch.rs` proves what the protocol answers; this proves what the *door*
//! answers, which is a different question and the one the privacy claim rests
//! on. Every test here speaks raw HTTP/1.1 over a `TcpStream` rather than going
//! through a client library, because the things being checked — a missing
//! header, a wrong method, a request from a web page — are precisely the things
//! a well-behaved client would never send.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use wobu_mcp::dispatch::Silent;
use wobu_mcp::world::WorldResult;
use wobu_mcp::{Dispatcher, NodePatch, Running, Server, Token, World, WorldError};

struct TestWorld;

impl World for TestWorld {
    fn overview(&self) -> WorldResult {
        Ok(json!({ "name": "Ashfall", "nodeCount": 2 }))
    }
    fn list_nodes(&self, _kind: Option<&str>) -> WorldResult {
        Ok(json!({ "nodes": [] }))
    }
    fn get_node(&self, id: &str) -> WorldResult {
        Ok(json!({ "id": id }))
    }
    fn search_nodes(&self, _query: &str, _limit: usize) -> WorldResult {
        Ok(json!({ "nodes": [] }))
    }
    fn node_links(&self, _id: &str) -> WorldResult {
        Ok(json!({ "links": [] }))
    }
    fn influence_stack(&self, _subject: &str, _preset: Option<&str>) -> WorldResult {
        Ok(json!({ "layers": [] }))
    }
    fn compile_prompt(&self, _subject: &str, _preset: Option<&str>) -> WorldResult {
        Ok(json!({ "prompt": "" }))
    }
    fn list_generations(&self, _node: &str, _limit: usize) -> WorldResult {
        Ok(json!({ "generations": [] }))
    }
    fn get_generation(&self, _id: &str) -> WorldResult {
        Ok(json!({}))
    }
    fn create_node(&self, _kind: &str, _name: &str, _parent: Option<&str>) -> WorldResult {
        Err(WorldError::new("the test world refuses to be written to"))
    }
    fn update_node(&self, _id: &str, _patch: &NodePatch) -> WorldResult {
        Err(WorldError::new("the test world refuses to be written to"))
    }
    fn link_nodes(&self, _node: &str, _to: &str, _role: &str, _weight: Option<f32>) -> WorldResult {
        Err(WorldError::new("the test world refuses to be written to"))
    }
}

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

async fn start(allow_writes: bool) -> (Running, Arc<AtomicBool>) {
    let writes = Arc::new(AtomicBool::new(allow_writes));
    let dispatcher = Arc::new(Dispatcher::new(
        Arc::new(TestWorld),
        Arc::clone(&writes),
        Arc::new(Silent),
        "wobu",
        "test",
    ));
    // Port 0: the OS picks, so the suite never fights the user's real Wobu for
    // 9628 and never leaves a fixed port in TIME_WAIT between runs.
    let running = Server::start(0, Token::from_raw(TOKEN), dispatcher).await.unwrap();
    (running, writes)
}

/// One raw request. Returns the whole response, headers and all, as text.
async fn raw(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.expect("loopback is up");
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
}

fn post(body: &str, headers: &str) -> String {
    format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    )
}

fn authorised(body: &str) -> String {
    post(body, &format!("Authorization: Bearer {TOKEN}\r\n"))
}

fn status(response: &str) -> &str {
    response.lines().next().unwrap_or_default()
}

fn body(response: &str) -> Value {
    let (_, body) = response.split_once("\r\n\r\n").expect("a response has a body separator");
    serde_json::from_str(body.trim()).unwrap_or(Value::Null)
}

async fn rpc(port: u16, method: &str, params: Value) -> Value {
    let message =
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
    body(&raw(port, &authorised(&message)).await)
}

#[tokio::test]
async fn the_listener_is_on_loopback_and_nowhere_else() {
    let (running, _) = start(false).await;
    // Not a behavioural test so much as the assertion that makes the privacy
    // claim checkable: if this ever reads 0.0.0.0, the policy is wrong.
    assert!(running.addr().ip().is_loopback(), "{}", running.addr());
    assert!(running.endpoint().starts_with("http://127.0.0.1:"));
}

#[tokio::test]
async fn a_request_with_no_token_is_refused_and_told_where_to_find_one() {
    let (running, _) = start(false).await;
    let response =
        raw(running.port(), &post(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#, "")).await;
    assert!(status(&response).contains("401"), "{}", status(&response));
    assert!(response.contains("Settings"), "{response}");
}

#[tokio::test]
async fn a_wrong_token_is_refused_even_when_it_is_a_prefix_of_the_right_one() {
    let (running, _) = start(false).await;
    let short = format!("Authorization: Bearer {}\r\n", &TOKEN[..8]);
    let response =
        raw(running.port(), &post(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#, &short)).await;
    assert!(status(&response).contains("401"), "{}", status(&response));
}

#[tokio::test]
async fn a_request_from_a_web_page_is_refused_before_the_token_is_even_looked_at() {
    // The DNS-rebinding case. A page on any origin can reach loopback; what it
    // must never get is a different answer for a right and a wrong token.
    let (running, _) = start(false).await;
    let message = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;

    let with_good_token = post(
        message,
        &format!("Origin: https://evil.example\r\nAuthorization: Bearer {TOKEN}\r\n"),
    );
    let with_bad_token =
        post(message, "Origin: https://evil.example\r\nAuthorization: Bearer nope\r\n");

    let good = raw(running.port(), &with_good_token).await;
    let bad = raw(running.port(), &with_bad_token).await;
    assert!(status(&good).contains("403"), "{}", status(&good));
    assert_eq!(status(&good), status(&bad), "a page could tell a good token from a bad one");
    // And no CORS header, so a browser could not read the body even if one had
    // been produced.
    assert!(!good.to_lowercase().contains("access-control-allow-origin"), "{good}");
}

#[tokio::test]
async fn there_is_no_event_stream_to_get() {
    let (running, _) = start(false).await;
    let response = raw(
        running.port(),
        &format!(
            "GET /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TOKEN}\r\n\
             Connection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(status(&response).contains("405"), "{}", status(&response));
}

#[tokio::test]
async fn a_correctly_authorised_client_completes_the_handshake_and_lists_read_tools() {
    let (running, _) = start(false).await;
    let initialize =
        rpc(running.port(), "initialize", json!({ "protocolVersion": "2025-06-18" })).await;
    assert_eq!(initialize["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(initialize["result"]["serverInfo"]["name"], "wobu");

    let listed = rpc(running.port(), "tools/list", json!({})).await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"world_overview"));
    assert!(!names.contains(&"create_node"), "writes are off: {names:?}");
}

#[tokio::test]
async fn a_notification_gets_no_body_at_all() {
    let (running, _) = start(false).await;
    let message = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let response = raw(running.port(), &authorised(message)).await;
    assert!(status(&response).contains("202"), "{}", status(&response));
}

#[tokio::test]
async fn a_tool_call_reaches_the_world() {
    let (running, _) = start(false).await;
    let answer =
        rpc(running.port(), "tools/call", json!({ "name": "world_overview", "arguments": {} }))
            .await;
    assert_eq!(answer["result"]["isError"], false);
    assert_eq!(answer["result"]["structuredContent"]["name"], "Ashfall");
}

#[tokio::test]
async fn writes_are_refused_over_the_wire_until_the_second_opt_in_is_on() {
    let (running, writes) = start(false).await;
    let call = json!({ "name": "create_node", "arguments": { "kind": "prop", "name": "Lamp" } });

    let refused = rpc(running.port(), "tools/call", call.clone()).await;
    assert_eq!(refused["result"]["isError"], true);
    assert!(
        refused["result"]["content"][0]["text"].as_str().unwrap().contains("Settings"),
        "{refused}"
    );

    // Flipped mid-session, with nothing restarted: the tool is now advertised
    // and the call now reaches the world (which refuses it on its own terms).
    writes.store(true, std::sync::atomic::Ordering::SeqCst);
    let listed = rpc(running.port(), "tools/list", json!({})).await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"create_node"), "{names:?}");

    let reached = rpc(running.port(), "tools/call", call).await;
    let text = reached["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("refuses to be written to"), "{text}");
}

#[tokio::test]
async fn a_batch_comes_back_as_a_batch_with_notifications_omitted() {
    let (running, _) = start(false).await;
    let message = json!([
        { "jsonrpc": "2.0", "id": 1, "method": "ping" },
        { "jsonrpc": "2.0", "method": "notifications/initialized" },
        { "jsonrpc": "2.0", "id": "two", "method": "ping" },
    ])
    .to_string();
    let answers = body(&raw(running.port(), &authorised(&message)).await);
    let answers = answers.as_array().expect("a batch answers with an array");
    assert_eq!(answers.len(), 2, "the notification was answered: {answers:?}");
    assert_eq!(answers[1]["id"], "two");
}

#[tokio::test]
async fn nonsense_comes_back_as_a_parse_error_rather_than_a_dropped_connection() {
    let (running, _) = start(false).await;
    let answer = body(&raw(running.port(), &authorised("{not json at all")).await);
    assert_eq!(answer["error"]["code"], -32700);
}

#[tokio::test]
async fn dropping_the_handle_closes_the_port() {
    // The whole of "turning it off works": Settings holds this value in an
    // `Option`, and `= None` has to actually stop the listener rather than
    // leaving a socket open behind a switch that says off.
    let (running, _) = start(false).await;
    let port = running.port();
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_ok());

    drop(running);

    // The accept loop wakes on the next poll; give it a moment rather than
    // racing it, but bound the wait so a listener that never stops fails here.
    let mut closed = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        if TcpStream::connect(("127.0.0.1", port)).await.is_err() {
            closed = true;
            break;
        }
    }
    assert!(closed, "the port was still accepting a second after the server was dropped");
}
