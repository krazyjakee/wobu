//! Actual local HTTP/SSE fixtures; no external provider or paid model is called.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use wobu_llm::{
    AnthropicProvider, Cancel, Discard, Error, GeminiProvider, StructuredRequest, TextProvider,
};

#[derive(Clone, Copy)]
enum Vendor {
    Anthropic,
    Gemini,
}
const VENDORS: [Vendor; 2] = [Vendor::Anthropic, Vendor::Gemini];
const KEY: &str = "fixture-secret-never-record";

impl Vendor {
    fn provider(self, url: String) -> Box<dyn TextProvider> {
        match self {
            Self::Anthropic => Box::new(AnthropicProvider::new(KEY).unwrap().with_base_url(url)),
            Self::Gemini => Box::new(GeminiProvider::new(KEY).unwrap().with_base_url(url)),
        }
    }
    fn start(self) -> String {
        match self {
            Self::Anthropic => {
                frame(
                    json!({"type":"message_start","message":{"usage":{"input_tokens":90,"cache_read_input_tokens":10,"output_tokens":2}}}),
                ) + &frame(
                    json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","name":"record_narrative_text","input":{}}}),
                )
            }
            Self::Gemini => {
                frame(
                    json!({"event_type":"interaction.created","interaction":{"usage":{"total_input_tokens":100,"total_cached_tokens":10,"total_output_tokens":2}}}),
                ) + &frame(
                    json!({"event_type":"step.start","index":1,"step":{"type":"model_output"}}),
                )
            }
        }
    }
    fn delta(self, text: &str) -> String {
        match self {
            Self::Anthropic => frame(
                json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":text}}),
            ),
            Self::Gemini => frame(
                json!({"event_type":"step.delta","index":1,"delta":{"type":"text","text":text}}),
            ),
        }
    }
    fn end(self, success: bool) -> String {
        match self {
            Self::Anthropic => {
                frame(json!({"type":"content_block_stop","index":1}))
                    + &frame(
                        json!({"type":"message_delta","delta":{"stop_reason":if success {"tool_use"} else {"max_tokens"}},"usage":{"output_tokens":12}}),
                    )
                    + &frame(json!({"type":"message_stop"}))
            }
            Self::Gemini => {
                frame(json!({"event_type":"step.stop","index":1}))
                    + &frame(
                        json!({"event_type":"interaction.completed","interaction":{"status":if success {"completed"} else {"incomplete"},"usage":{"total_input_tokens":100,"total_cached_tokens":10,"total_output_tokens":12}}}),
                    )
            }
        }
    }
    fn error(self) -> String {
        match self {
            Self::Anthropic => {
                frame(json!({"type":"error","error":{"type":"rate_limit_error","message":KEY}}))
            }
            Self::Gemini => frame(
                json!({"event_type":"error","error":{"code":"RESOURCE_EXHAUSTED","message":KEY}}),
            ),
        }
    }
}
fn frame(value: Value) -> String {
    format!("data: {value}\n\n")
}
fn request() -> StructuredRequest {
    StructuredRequest {
        model: "fixture-model".into(),
        system: Some("Only authored prose".into()),
        prompt: "Complete slot-1 for speaker-1".into(),
        max_output_tokens: 256,
        schema: json!({"type":"object","additionalProperties":false,"properties":{"slots":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"text":{"type":"string"}},"required":["id","text"]}}},"required":["slots"]}),
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

struct Fixture {
    url: String,
    handle: thread::JoinHandle<(String, Value, bool)>,
}
impl Fixture {
    fn new(status: u16, body: impl Into<Vec<u8>>, quiet: bool) -> Self {
        let body = body.into();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/fixture", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let (headers, request) = read_request(&mut socket);
            let mut response = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Type: text/event-stream\r\nRetry-After: 7\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len() + usize::from(quiet)
            ).into_bytes();
            response.extend_from_slice(&body);
            // Oversized bodies may be abandoned while this write is still running.
            let _ = socket.write_all(&response);
            let _ = socket.flush();
            let closed = if quiet {
                let mut byte = [0];
                match socket.read(&mut byte) {
                    Ok(0) => true,
                    Err(error) => error.kind() == std::io::ErrorKind::ConnectionReset,
                    _ => false,
                }
            } else {
                false
            };
            (headers, request, closed)
        });
        Self { url, handle }
    }
}
fn read_request(socket: &mut TcpStream) -> (String, Value) {
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        socket.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
        assert!(bytes.len() < 32 * 1024);
    }
    let headers = String::from_utf8(bytes).unwrap();
    let length: usize = headers
        .lines()
        .find_map(|line| {
            line.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap())
        })
        .unwrap();
    let mut body = vec![0; length];
    socket.read_exact(&mut body).unwrap();
    (headers, serde_json::from_slice(&body).unwrap())
}

#[test]
fn both_real_transports_send_exact_schema_auth_and_stream_complete_raw_json_with_usage() {
    for vendor in VENDORS {
        // The raw duplicate remains visible for the compiler's strict parser.
        let raw = "{\"slots\":[],\"slots\":[{\"id\":\"slot-1\",\"text\":\"héllo\"}]}";
        let fixture =
            Fixture::new(200, vendor.start() + &vendor.delta(raw) + &vendor.end(true), false);
        let provider = vendor.provider(fixture.url);
        assert!(provider.supports_structured());
        let mut streamed = String::new();
        let outcome = runtime().block_on(provider.structured(
            &request(),
            &mut |s: &str| streamed.push_str(s),
            &Cancel::new(),
        ));
        assert_eq!(outcome.result.unwrap(), raw);
        assert_eq!(streamed, raw);
        assert_eq!(outcome.usage.input_tokens, 90);
        assert_eq!(outcome.usage.cached_input_tokens, 10);
        assert_eq!(outcome.usage.output_tokens, 12);
        let (headers, body, _) = fixture.handle.join().unwrap();
        assert!(headers.contains(KEY));
        assert_eq!(body["model"], "fixture-model");
        assert_eq!(body["stream"], true);
        assert!(!body.to_string().contains(KEY));
        match vendor {
            Vendor::Anthropic => {
                assert_eq!(body["tools"][0]["input_schema"], request().schema);
                assert_eq!(body["tool_choice"]["name"], "record_narrative_text");
                assert_eq!(body["tool_choice"]["disable_parallel_tool_use"], true);
                assert_eq!(body["system"], request().system.unwrap());
                assert_eq!(body["max_tokens"], 256);
                assert!(headers.contains("anthropic-version: 2023-06-01"));
            }
            Vendor::Gemini => {
                assert_eq!(body["response_format"]["schema"], request().schema);
                assert_eq!(body["response_format"]["mime_type"], "application/json");
                assert_eq!(body["system_instruction"], request().system.unwrap());
                assert_eq!(body["generation_config"]["max_output_tokens"], 256);
                assert_eq!(body["store"], false);
                assert!(headers.contains("api-revision: 2026-05-20"));
            }
        }
    }
}

#[test]
fn invalid_json_truncation_multiple_outputs_and_oversize_never_return_candidates() {
    for vendor in VENDORS {
        let bodies = [
            vendor.start() + &vendor.delta("not json") + &vendor.end(true),
            vendor.start() + &vendor.delta("{}") + &vendor.end(false),
            vendor.start() + &vendor.delta("{}"),
            vendor.start() + "data: malformed\n\n" + &vendor.delta("{}") + &vendor.end(true),
            vendor.start() + &vendor.start() + &vendor.delta("{}") + &vendor.end(true),
            vendor.start() + &vendor.delta(&"x".repeat(65 * 1024)) + &vendor.end(true),
            vendor.start() + &format!(":{}", "x".repeat(1024 * 1024)),
        ];
        for body in bodies {
            let fixture = Fixture::new(200, body, false);
            let provider = vendor.provider(fixture.url);
            let mut bytes = 0;
            let outcome = runtime().block_on(provider.structured(
                &request(),
                &mut |s: &str| bytes += s.len(),
                &Cancel::new(),
            ));
            assert!(outcome.result.is_err());
            assert!(bytes <= 64 * 1024);
            assert!(!outcome.result.unwrap_err().to_string().contains(KEY));
            fixture.handle.join().unwrap();
        }
    }
}

#[test]
fn only_http_429_is_safe_rate_limit_and_stream_failures_preserve_usage_without_secrets() {
    for vendor in VENDORS {
        for status in [429, 401, 500] {
            let fixture = Fixture::new(status, KEY.to_owned(), false);
            let provider = vendor.provider(fixture.url);
            let outcome =
                runtime().block_on(provider.structured(&request(), &mut Discard, &Cancel::new()));
            let error = outcome.result.unwrap_err();
            assert!(!error.to_string().contains(KEY));
            assert_eq!(matches!(error, Error::RateLimited { .. }), status == 429);
            if let Error::RateLimited { retry_after, .. } = error {
                assert_eq!(retry_after, Some(Duration::from_secs(7)));
            }
            fixture.handle.join().unwrap();
        }
        let fixture =
            Fixture::new(200, vendor.start() + &vendor.delta("{") + &vendor.error(), false);
        let provider = vendor.provider(fixture.url);
        let outcome =
            runtime().block_on(provider.structured(&request(), &mut Discard, &Cancel::new()));
        assert_eq!(outcome.usage.input_tokens, 90);
        assert_eq!(outcome.usage.output_tokens, 2);
        let error = outcome.result.unwrap_err();
        assert!(matches!(error, Error::Unavailable { .. }));
        assert!(!error.to_string().contains(KEY));
        fixture.handle.join().unwrap();
    }
}

#[test]
fn cancellation_drops_a_quiet_http_body_and_keeps_already_reported_usage() {
    for vendor in VENDORS {
        let fixture = Fixture::new(200, vendor.start() + &vendor.delta("{"), true);
        let provider = vendor.provider(fixture.url);
        let cancel = Cancel::new();
        let cancelling = cancel.clone();
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        let stop = thread::spawn(move || {
            seen_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            thread::sleep(Duration::from_millis(30));
            cancelling.cancel();
        });
        let outcome = runtime().block_on(provider.structured(
            &request(),
            &mut |_: &str| {
                seen_tx.send(()).unwrap();
            },
            &cancel,
        ));
        stop.join().unwrap();
        assert!(matches!(outcome.result, Err(Error::Cancelled)));
        assert_eq!(outcome.usage.input_tokens, 90);
        assert!(fixture.handle.join().unwrap().2, "cancellation must close the actual socket");
    }
}

#[test]
fn pre_cancel_and_local_validation_never_open_a_connection() {
    for vendor in VENDORS {
        let provider = vendor.provider("http://127.0.0.1:1/not-a-provider".into());
        let cancel = Cancel::new();
        cancel.cancel();
        assert!(matches!(
            runtime().block_on(provider.structured(&request(), &mut Discard, &cancel)).result,
            Err(Error::Cancelled)
        ));
        let mut invalid = request();
        invalid.max_output_tokens = 0;
        assert!(matches!(
            runtime().block_on(provider.structured(&invalid, &mut Discard, &Cancel::new())).result,
            Err(Error::SchemaRejected { .. })
        ));
    }
}

#[test]
fn corrupt_utf8_is_not_silently_repaired_inside_a_valid_json_string() {
    for vendor in VENDORS {
        let body = vendor.start() + &vendor.delta("{\"text\":\"REPLACE\"}") + &vendor.end(true);
        let mut bytes = body.into_bytes();
        let index = bytes.windows(7).position(|s| s == b"REPLACE").unwrap();
        bytes[index] = 255;
        let fixture = Fixture::new(200, bytes, false);
        let provider = vendor.provider(fixture.url);
        let outcome =
            runtime().block_on(provider.structured(&request(), &mut Discard, &Cancel::new()));
        assert!(matches!(outcome.result, Err(Error::NotJson(_))));
        fixture.handle.join().unwrap();
    }
}

#[test]
fn narrative_speaker_union_and_array_bounds_reach_each_provider_in_its_supported_dialect() {
    let speaker = json!({"oneOf":[{"enum":["player","narrator"]},{"type":"object","additionalProperties":false,"required":["entity"],"properties":{"entity":{"type":"string"}}}]});
    let schema = json!({"type":"object","additionalProperties":false,"required":["lines"],"properties":{"lines":{"type":"array","minItems":1,"maxItems":1,"items":{"type":"object","additionalProperties":false,"required":["slot_id","variant_id","speaker","text"],"properties":{"slot_id":{"type":"string"},"variant_id":{"type":"string"},"speaker":speaker,"text":{"type":"string","minLength":1,"maxLength":4000}}}}}});
    for vendor in VENDORS {
        let fixture = Fixture::new(
            200,
            vendor.start() + &vendor.delta("{\"lines\":[]}") + &vendor.end(true),
            false,
        );
        let provider = vendor.provider(fixture.url);
        let mut req = request();
        req.schema = schema.clone();
        // Semantic invalidity is deliberately left visible to the compiler.
        assert!(
            runtime()
                .block_on(provider.structured(&req, &mut Discard, &Cancel::new()))
                .result
                .is_ok()
        );
        assert_eq!(req.schema, schema, "canonical schema remains unchanged");
        let (_, body, _) = fixture.handle.join().unwrap();
        let wire = match vendor {
            Vendor::Anthropic => &body["tools"][0]["input_schema"],
            Vendor::Gemini => &body["response_format"]["schema"],
        };
        let mut expected = schema.clone();
        if matches!(vendor, Vendor::Gemini) {
            expected["properties"]["lines"]["items"]["properties"]["speaker"] =
                json!({"anyOf":speaker["oneOf"]});
        }
        assert_eq!(wire, &expected);
    }
}
