//! Everything ComfyUI says, and what it means.
//!
//! Split out from the adapter for the reason `wobu-llm`'s `wire.rs` is: this is
//! the half that can be driven from recorded payloads, and the half where a
//! misread field is a wrong picture rather than a compile error.
//!
//! Two framings live here, because the run is reported over two of them:
//!
//! - **Text frames** carry JSON events — queue depth, sampler steps, the
//!   filename of the finished image, and the Python exception when a node
//!   throws.
//! - **Binary frames** carry latent previews, and they are not JSON. A reader
//!   that assumes every websocket message is text drops every preview on the
//!   floor and reports nothing, which looks exactly like a backend that has none.
//!
//! ## Turning a status code into something a person can act on
//!
//! [`rejected`] is the other half of what this file is for. `/prompt` answers a
//! graph it will not run with a 400 whose body says which node and why, and
//! every one of those cases sends the user somewhere different: install a node
//! pack, download a checkpoint, or file a bug against us. Reporting "ComfyUI
//! returned 400" throws away the only part of the answer that was useful.

use serde_json::Value;

use crate::error::Error;

/// One thing the websocket said.
///
/// Named for the `type` field ComfyUI sends. Everything not listed is
/// [`Event::Other`] rather than an error: ComfyUI adds event types between
/// releases — `progress_state` and `execution_success` both arrived that way —
/// and an adapter that refused to decode an unknown one would break on an
/// upgrade that changed nothing it uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    /// Queue depth, which is what the status bar counts. Sent unprompted and to
    /// every client, so it arrives before and after any run of ours.
    Status {
        queue_remaining: u32,
    },
    /// Our graph has left the queue and is on the GPU. The moment after which
    /// stopping means `/interrupt` rather than dropping a queue entry.
    ExecutionStart {
        prompt_id: String,
    },
    /// Which node is running. `node: None` is the end of the run in older
    /// builds, which is why it is an `Option` rather than a string.
    Executing {
        prompt_id: Option<String>,
        node: Option<String>,
    },
    /// One sampler step. `max` is per node, not per graph.
    Progress {
        prompt_id: Option<String>,
        value: u32,
        max: u32,
    },
    /// A node finished and produced files. Only the output node's message
    /// carries the image we asked for.
    Executed {
        prompt_id: String,
        node: String,
        images: Vec<ImageRef>,
    },
    /// A node threw. Carries the Python exception, which is the only description
    /// of the failure that exists.
    ExecutionError {
        prompt_id: String,
        node_type: String,
        message: String,
    },
    /// The run was stopped — by our `/interrupt`, or by somebody pressing the
    /// button in the ComfyUI web UI, which is why it is an event and not just
    /// the reply to our own call.
    ExecutionInterrupted {
        prompt_id: String,
    },
    /// The graph finished, in builds that send it.
    ExecutionSuccess {
        prompt_id: String,
    },
    Other,
}

/// A file ComfyUI wrote, addressed the way `/view` takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageRef {
    pub(crate) filename: String,
    pub(crate) subfolder: String,
    /// `output`, `temp` or `input`. Carried rather than assumed because the same
    /// filename exists under more than one of them.
    pub(crate) kind: String,
}

impl ImageRef {
    /// The `/view` query that fetches these bytes.
    ///
    /// Percent-encoded here rather than by a URL crate, because the only
    /// characters that can appear are the ones ComfyUI puts in a filename and
    /// the ones a `filename_prefix` allows — and a whole dependency to escape
    /// two of them would be the larger risk.
    pub(crate) fn query(&self) -> String {
        format!(
            "filename={}&subfolder={}&type={}",
            escape(&self.filename),
            escape(&self.subfolder),
            escape(&self.kind),
        )
    }
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

impl Event {
    /// Decode one text frame.
    ///
    /// Total: a frame that is not JSON, or is JSON of a shape this does not
    /// know, is [`Event::Other`]. The alternative is an adapter that fails a
    /// render because a ComfyUI release added a field.
    pub(crate) fn parse(text: &str) -> Event {
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            return Event::Other;
        };
        let data = message.get("data").unwrap_or(&Value::Null);
        let string = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_owned);
        let number = |key: &str| data.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
        match message.get("type").and_then(Value::as_str) {
            Some("status") => Event::Status {
                queue_remaining: data
                    .pointer("/status/exec_info/queue_remaining")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
            },
            Some("execution_start") => match string("prompt_id") {
                Some(prompt_id) => Event::ExecutionStart { prompt_id },
                None => Event::Other,
            },
            Some("executing") => {
                Event::Executing { prompt_id: string("prompt_id"), node: string("node") }
            }
            Some("progress") => Event::Progress {
                prompt_id: string("prompt_id"),
                value: number("value"),
                max: number("max"),
            },
            Some("executed") => match (string("prompt_id"), string("node")) {
                (Some(prompt_id), Some(node)) => {
                    Event::Executed { prompt_id, node, images: images(data) }
                }
                _ => Event::Other,
            },
            Some("execution_error") => match string("prompt_id") {
                Some(prompt_id) => Event::ExecutionError {
                    prompt_id,
                    node_type: string("node_type").unwrap_or_default(),
                    // `exception_message` is the line a Python traceback ends
                    // with, which is the one a person can read. The traceback
                    // itself is in `traceback` and is left there.
                    message: string("exception_message").unwrap_or_default(),
                },
                None => Event::Other,
            },
            Some("execution_interrupted") => match string("prompt_id") {
                Some(prompt_id) => Event::ExecutionInterrupted { prompt_id },
                None => Event::Other,
            },
            Some("execution_success") => match string("prompt_id") {
                Some(prompt_id) => Event::ExecutionSuccess { prompt_id },
                None => Event::Other,
            },
            _ => Event::Other,
        }
    }

    /// The prompt this event is about, when it says.
    ///
    /// Load-bearing rather than tidy: ComfyUI broadcasts to every connected
    /// client, so a second window queueing its own render sends its progress
    /// down our socket too. Without this filter a wobu render would report
    /// somebody else's sampler steps and finish on somebody else's image.
    pub(crate) fn prompt_id(&self) -> Option<&str> {
        match self {
            Event::ExecutionStart { prompt_id }
            | Event::Executed { prompt_id, .. }
            | Event::ExecutionError { prompt_id, .. }
            | Event::ExecutionInterrupted { prompt_id }
            | Event::ExecutionSuccess { prompt_id } => Some(prompt_id),
            Event::Executing { prompt_id, .. } | Event::Progress { prompt_id, .. } => {
                prompt_id.as_deref()
            }
            Event::Status { .. } | Event::Other => None,
        }
    }
}

fn images(data: &Value) -> Vec<ImageRef> {
    let field = |image: &Value, key: &str| {
        image.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
    };
    data.pointer("/output/images")
        .and_then(Value::as_array)
        .map(|images| {
            images
                .iter()
                .filter(|image| !field(image, "filename").is_empty())
                .map(|image| ImageRef {
                    filename: field(image, "filename"),
                    subfolder: field(image, "subfolder"),
                    // Defaulted rather than skipped: `/view` treats a missing
                    // type as `output`, and so does a build that stops sending
                    // it.
                    kind: match field(image, "type") {
                        kind if kind.is_empty() => "output".to_owned(),
                        kind => kind,
                    },
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The binary event id ComfyUI puts in front of a preview payload.
///
/// `1` is `PREVIEW_IMAGE`. There are others — unencoded previews, text — and
/// they are skipped rather than guessed at, because a payload read as an image
/// when it is not becomes a `data:` URL the webview renders as a broken picture
/// in the middle of a render that is going fine.
const PREVIEW_IMAGE: u32 = 1;

/// A latent preview, as it arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Preview<'a> {
    pub(crate) mime: &'static str,
    pub(crate) bytes: &'a [u8],
}

/// Decode one binary frame, or `None` if it is not a preview.
///
/// The framing is two big-endian `u32`s and then the file: the event id, then
/// the image format. It is not JSON and it is not base64 — this is the part of
/// the protocol that a reader written against the documented *events* misses
/// entirely, and the symptom is a backend that declares
/// [`streaming_preview`](crate::Capabilities::streaming_preview) and never draws
/// one, which `backend.rs` calls out as an empty box for the whole of a render.
pub(crate) fn preview(frame: &[u8]) -> Option<Preview<'_>> {
    let header = |at: usize| -> u32 {
        u32::from_be_bytes([frame[at], frame[at + 1], frame[at + 2], frame[at + 3]])
    };
    if frame.len() <= 8 || header(0) != PREVIEW_IMAGE {
        return None;
    }
    let mime = match header(4) {
        1 => "image/jpeg",
        2 => "image/png",
        // A format id from a future release. Skipped rather than labelled
        // `image/jpeg` and hoped for.
        _ => return None,
    };
    Some(Preview { mime, bytes: &frame[8..] })
}

/// A `data:` URL, which is what [`ProgressSink::preview`] takes.
///
/// `backend.rs` leaves the encoding open — "whether previews reach the webview
/// as a `data:` URL or an `asset://` path is #40's decision and is not settled".
/// This is the one that needs no filesystem, which matters because a preview is
/// superseded three times a second and writing each one to disk would be a
/// render's worth of temporary files per generation.
///
/// [`ProgressSink::preview`]: crate::ProgressSink::preview
pub(crate) fn data_url(preview: &Preview<'_>) -> String {
    use base64::Engine;
    format!(
        "data:{};base64,{}",
        preview.mime,
        base64::engine::general_purpose::STANDARD.encode(preview.bytes),
    )
}

/// The body `/prompt` takes.
///
/// `client_id` is what ties the run to our websocket. Without it ComfyUI still
/// runs the graph and still broadcasts, but nothing arriving on the socket is
/// addressed to us, and a cancellation would have no prompt id to interrupt.
pub(crate) fn prompt_body(graph: serde_json::Map<String, Value>, client_id: &str) -> Value {
    serde_json::json!({ "prompt": graph, "client_id": client_id })
}

/// The id `/prompt` gives a graph it accepted.
pub(crate) fn queued(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body).ok()?.get("prompt_id")?.as_str().map(str::to_owned)
}

/// What a refused `/prompt` actually means.
///
/// Three answers, and they send the user to three different places — which is
/// the whole of #51's complaint about reporting the status code:
///
/// - **A node that is not installed.** The fix is a node pack, and the name of
///   the class is the thing to search for.
/// - **A model file the server cannot see.** The fix is a download, or picking a
///   different model from a dropdown that is drawn from
///   [`Installed`](super::Installed) and should not have offered this one.
/// - **Anything else**, which is ours: a graph we built and ComfyUI would not
///   take. It carries ComfyUI's own sentence, because that sentence is the only
///   description of the problem in existence.
///
/// All three are [`Error::Unavailable`], which `error.rs` chose for exactly this:
/// "the backend is not able to serve this right now, and waiting is a sensible
/// thing to do" — here the waiting is the user installing something, and "Try
/// again" is then the right button.
pub(crate) fn rejected(body: &[u8], status: u16) -> Error {
    let Ok(document) = serde_json::from_slice::<Value>(body) else {
        return Error::Unavailable {
            detail: format!("ComfyUI refused the workflow with HTTP {status} and no explanation"),
        };
    };
    let text = |at: &str| document.pointer(at).and_then(Value::as_str).unwrap_or_default();

    // `invalid_prompt` is what a graph naming a class this ComfyUI has never
    // loaded comes back as, and the message names the class.
    if text("/error/type") == "invalid_prompt" {
        return Error::Unavailable {
            detail: format!(
                "this workflow references a node this ComfyUI does not have installed — {}. \
                 Install the custom node pack that provides it, or pick a model that uses a \
                 different workflow",
                text("/error/message").trim_end_matches('.'),
            ),
        };
    }

    // Per-node validation. The first failing node is the one to name: the rest
    // are usually the same missing file reported again by everything downstream.
    if let Some(errors) = document.get("node_errors").and_then(Value::as_object) {
        for (id, node) in errors {
            let class = node.get("class_type").and_then(Value::as_str).unwrap_or("a node");
            let Some(first) = node.pointer("/errors/0") else {
                continue;
            };
            let detail = first.get("details").and_then(Value::as_str).unwrap_or_default();
            let kind = first.get("type").and_then(Value::as_str).unwrap_or_default();
            let message = first.get("message").and_then(Value::as_str).unwrap_or_default();
            return Error::Unavailable {
                detail: if kind == "value_not_in_list" {
                    format!(
                        "this ComfyUI has no such file for {class} — {detail}. Download it into \
                         the right models folder, or pick one this server already has"
                    )
                } else {
                    format!("ComfyUI refused node {id} ({class}): {message} — {detail}")
                },
            };
        }
    }

    Error::Unavailable {
        detail: match text("/error/message") {
            "" => format!("ComfyUI refused the workflow with HTTP {status}"),
            message => {
                format!("ComfyUI refused the workflow: {message} {}", text("/error/details"))
                    .trim_end()
                    .to_owned()
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame(value: Value) -> String {
        serde_json::to_string(&value).unwrap()
    }

    #[test]
    fn the_documented_events_of_one_whole_run_decode_in_order() {
        // The wire shapes ComfyUI publishes, in the order a render sends them.
        // Everything downstream — the progress bar, the image, the queue count —
        // is read off these, and a field renamed in a release shows up here.
        let events: Vec<Event> = [
            json!({"type": "status", "data": {"status": {"exec_info": {"queue_remaining": 2}},
                   "sid": "abc"}}),
            json!({"type": "execution_start", "data": {"prompt_id": "p1", "timestamp": 1}}),
            json!({"type": "execution_cached", "data": {"nodes": ["4"], "prompt_id": "p1"}}),
            json!({"type": "executing", "data": {"node": "3", "display_node": "3",
                   "prompt_id": "p1"}}),
            json!({"type": "progress", "data": {"value": 7, "max": 25, "prompt_id": "p1",
                   "node": "3"}}),
            json!({"type": "executed", "data": {"node": "9", "display_node": "9",
                   "prompt_id": "p1", "output": {"images": [{"filename": "wobu_00007_.png",
                   "subfolder": "", "type": "output"}]}}}),
            json!({"type": "executing", "data": {"node": null, "prompt_id": "p1"}}),
            json!({"type": "execution_success", "data": {"prompt_id": "p1", "timestamp": 2}}),
        ]
        .iter()
        .map(|value| Event::parse(&frame(value.clone())))
        .collect();

        assert_eq!(events[0], Event::Status { queue_remaining: 2 });
        assert_eq!(events[1], Event::ExecutionStart { prompt_id: "p1".into() });
        assert_eq!(events[2], Event::Other, "an event we do not read is not a failure");
        assert_eq!(events[3], Event::Executing {
            prompt_id: Some("p1".into()),
            node: Some("3".into())
        },);
        assert_eq!(events[4], Event::Progress { prompt_id: Some("p1".into()), value: 7, max: 25 });
        assert_eq!(events[5], Event::Executed {
            prompt_id: "p1".into(),
            node: "9".into(),
            images: vec![ImageRef {
                filename: "wobu_00007_.png".into(),
                subfolder: String::new(),
                kind: "output".into(),
            }],
        });
        assert_eq!(events[6], Event::Executing { prompt_id: Some("p1".into()), node: None });
        assert_eq!(events[7], Event::ExecutionSuccess { prompt_id: "p1".into() });
    }

    #[test]
    fn an_event_from_a_release_we_have_never_seen_is_ignored_rather_than_fatal() {
        // ComfyUI adds event types between releases — `progress_state` and
        // `execution_success` both arrived that way. An adapter that refused an
        // unknown one would fail a render because the server was upgraded.
        assert_eq!(
            Event::parse(&frame(json!({"type": "progress_state", "data": {}}))),
            Event::Other
        );
        assert_eq!(Event::parse("not json at all"), Event::Other);
        assert_eq!(Event::parse("{}"), Event::Other);
    }

    #[test]
    fn every_event_that_names_a_run_can_be_told_apart_from_another_clients() {
        // ComfyUI broadcasts to every connected client. A second window, or the
        // ComfyUI web UI itself, queueing a render sends its steps down our
        // socket; without the prompt id a wobu generation would report somebody
        // else's progress and finish on somebody else's image.
        let ours = Event::parse(&frame(json!({"type": "executed", "data": {"node": "9",
            "prompt_id": "ours", "output": {"images": [{"filename": "a.png", "type": "output"}]}}})));
        assert_eq!(ours.prompt_id(), Some("ours"));

        let theirs = Event::parse(&frame(json!({"type": "progress",
            "data": {"value": 1, "max": 20, "prompt_id": "theirs", "node": "3"}})));
        assert_eq!(theirs.prompt_id(), Some("theirs"));

        // Queue depth belongs to the server rather than to any run, which is why
        // the status bar can show it between generations.
        assert_eq!(Event::Status { queue_remaining: 0 }.prompt_id(), None);
    }

    #[test]
    fn a_latent_preview_is_a_binary_frame_and_not_a_json_one() {
        // The part of the protocol a reader written against the documented
        // events misses entirely: previews never arrive as text. Dropping them
        // gives a backend that declares `streaming_preview` and draws nothing,
        // which `backend.rs` calls an empty box for the whole of a render.
        let mut jpeg = vec![0, 0, 0, 1, 0, 0, 0, 1];
        jpeg.extend_from_slice(&[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10]);
        let decoded = preview(&jpeg).expect("event 1, format 1 is a JPEG preview");
        assert_eq!(decoded.mime, "image/jpeg");
        assert_eq!(decoded.bytes, [0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10]);
        assert!(data_url(&decoded).starts_with("data:image/jpeg;base64,/9j/"));

        let png = [&[0, 0, 0, 1, 0, 0, 0, 2][..], &[0x89, b'P', b'N', b'G'][..]].concat();
        assert_eq!(preview(&png).unwrap().mime, "image/png");
    }

    #[test]
    fn a_binary_frame_that_is_not_a_preview_is_never_shown_as_one() {
        // ComfyUI sends other binary event ids, and will send more. One of them
        // rendered as a `data:` URL is a broken picture in the middle of a render
        // that is going fine — worse than showing nothing, because it reads as
        // the generation having failed.
        assert_eq!(preview(&[0, 0, 0, 3, 0, 0, 0, 1, 0xff]), None, "event 3 is text, not an image");
        assert_eq!(preview(&[0, 0, 0, 1, 0, 0, 0, 9, 0xff]), None, "an unknown image format");
        assert_eq!(preview(&[0, 0, 0, 1, 0, 0, 0, 1]), None, "a header with no image after it");
        assert_eq!(preview(&[]), None);
    }

    #[test]
    fn a_missing_node_says_which_node_rather_than_which_status_code() {
        // The deliverable #51 asks for by name. A 400 tells the user a
        // validation failed; this tells them what to install.
        let body = serde_json::to_vec(&json!({
            "error": {"type": "invalid_prompt",
                      "message": "Cannot execute because node IPAdapterAdvanced does not exist.",
                      "details": "Node ID '#12'", "extra_info": {}},
            "node_errors": {},
        }))
        .unwrap();
        let error = rejected(&body, 400);
        let message = error.to_string();
        assert!(message.contains("IPAdapterAdvanced"), "{message}");
        assert!(message.contains("does not have installed"), "{message}");
        assert!(error.is_retryable(), "installing the pack and pressing Try again is the fix");
        assert_eq!(error.code(), "provider.unavailable");
    }

    #[test]
    fn a_checkpoint_this_server_has_never_downloaded_names_the_file() {
        // The other half of the same 400, and a completely different fix.
        // `error.rs` reserves `Unavailable` for this case in as many words: the
        // waiting is the user installing it.
        let body = serde_json::to_vec(&json!({
            "error": {"type": "prompt_outputs_failed_validation",
                      "message": "Prompt outputs failed validation", "details": "",
                      "extra_info": {}},
            "node_errors": {"4": {"errors": [{"type": "value_not_in_list",
                "message": "Value not in list",
                "details": "ckpt_name: 'ashfall_v3.safetensors' not in ['dreamshaper_8.safetensors']",
                "extra_info": {}}], "dependent_outputs": ["9"],
                "class_type": "CheckpointLoaderSimple"}},
        }))
        .unwrap();
        let message = rejected(&body, 400).to_string();
        assert!(message.contains("ashfall_v3.safetensors"), "{message}");
        assert!(message.contains("CheckpointLoaderSimple"), "{message}");
        assert!(message.contains("models folder"), "{message}");
    }

    #[test]
    fn a_refusal_we_cannot_classify_still_carries_comfyui_s_own_sentence() {
        // The fallback has to be better than the status code, because ComfyUI's
        // message is the only description of the problem that exists.
        let body = serde_json::to_vec(&json!({
            "error": {"type": "prompt_no_outputs", "message": "Prompt has no outputs",
                      "details": "", "extra_info": {}},
            "node_errors": {},
        }))
        .unwrap();
        assert!(rejected(&body, 400).to_string().contains("Prompt has no outputs"));

        // And a body that is not JSON at all — a proxy's error page, most likely
        // — still reports the status rather than panicking on it.
        let opaque = rejected(b"<html>502 Bad Gateway</html>", 502).to_string();
        assert!(opaque.contains("502"), "{opaque}");
    }

    #[test]
    fn a_filename_is_escaped_before_it_becomes_a_query() {
        // `filename_prefix` is ours, but subfolders are the user's and ComfyUI
        // allows spaces and `&` in both. An unescaped `&` truncates the query and
        // fetches the wrong file, or nothing.
        let image = ImageRef {
            filename: "wobu &c 00001_.png".into(),
            subfolder: "ash/fall".into(),
            kind: "output".into(),
        };
        assert_eq!(
            image.query(),
            "filename=wobu%20%26c%2000001_.png&subfolder=ash%2Ffall&type=output",
        );
    }

    #[test]
    fn the_prompt_body_ties_the_run_to_the_socket_that_will_report_it() {
        // Without `client_id` ComfyUI runs the graph and broadcasts to everyone,
        // and nothing on our socket is addressed to us — so there is no prompt id
        // to interrupt when the user presses Stop.
        let graph = match json!({"3": {"class_type": "KSampler", "inputs": {}}}) {
            Value::Object(map) => map,
            _ => unreachable!(),
        };
        let body = prompt_body(graph, "wobu-1");
        assert_eq!(body["client_id"], "wobu-1");
        assert_eq!(body["prompt"]["3"]["class_type"], "KSampler");
        assert_eq!(
            queued(br#"{"prompt_id": "p1", "number": 3, "node_errors": {}}"#),
            Some("p1".into())
        );
        assert_eq!(queued(b"{}"), None);
    }
}
