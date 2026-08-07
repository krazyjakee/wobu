//! Watching one run over the websocket, and stopping when the user says stop.
//!
//! [`watch`] is generic over a stream of [`Frame`] rather than taking a
//! `WebSocketStream`, for the same reason `wobu-llm`'s `read_body` is generic
//! over a byte stream: this loop is where cancellation either works or silently
//! does not, and driving it from recorded frames is the only way to check that
//! without a GPU and a server. The concrete socket is [`connect`], which is four
//! lines and is the part nothing here can test.
//!
//! ## Cancellation is a race, not a poll
//!
//! Between two sampler steps ComfyUI can be silent for a long time — a
//! twenty-second model load, a VAE decode of a 2048px image, a queue with
//! somebody else's render in front of ours. Polling [`Cancel`] between frames
//! would leave a job that the user stopped parked until the server next spoke,
//! holding a queue slot the whole time. So every read is raced against
//! [`Cancel::cancelled`], exactly as the text adapters do.
//!
//! Losing that race returns immediately — but unlike a text provider, *returning
//! is not enough*. Dropping an HTTP response body closes the connection and
//! stops the model generating; dropping this websocket stops the reporting and
//! leaves the graph running on the GPU. That is the local equivalent of the
//! billing failure [#49](https://github.com/krazyjakee/wobu/issues/49) was
//! designed against, and the reason [`Watched::started`] is carried back out:
//! the caller has to tell ComfyUI to stop, and which call does that depends on
//! whether the run had left the queue. See `ComfyBackend::stop`.

use std::collections::BTreeMap;
use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use tokio_tungstenite::tungstenite::Message;

use crate::Cancel;
use crate::backend::ProgressSink;
use crate::comfy::wire::{Event, ImageRef, preview};
use crate::error::Error;

/// One websocket message, in the two forms ComfyUI uses.
///
/// Its own type rather than `tungstenite::Message` so that [`watch`] can be fed
/// recorded frames, and so that the ping, pong and close messages the library
/// deals with never reach the part of this file that decides what a run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Frame {
    Text(String),
    /// A latent preview. Never JSON — see `wire::preview`.
    Binary(Vec<u8>),
}

/// How the run ended.
#[derive(Debug)]
pub(crate) enum Ended {
    /// The output node reported files.
    Images(Vec<ImageRef>),
    Failed(Error),
    /// The user pressed Stop, or somebody pressed Interrupt in the ComfyUI web
    /// UI, which is the same event arriving from the other direction.
    Cancelled,
}

/// What one run did, plus the one fact stopping it needs.
#[derive(Debug)]
pub(crate) struct Watched {
    pub(crate) ended: Ended,
    /// Whether our graph had left the queue and reached the GPU.
    ///
    /// The difference between two entirely different ways to stop it, and
    /// getting it wrong is worse than not stopping: `/interrupt` takes no prompt
    /// id and kills whatever is executing, so interrupting a run of ours that
    /// was still queued would stop a *stranger's* render — the one in front of
    /// us — and leave ours to start straight afterwards.
    pub(crate) started: bool,
}

/// Read frames until this run ends, reporting progress as it goes.
///
/// `output` is the node whose `executed` message carries the image; anything
/// another node saves — a preview node somebody left in a template — is not what
/// was asked for. `nodes` maps node id to class so a step can say what is
/// happening rather than only how far through it is.
pub(crate) async fn watch<S, E>(
    frames: S,
    prompt_id: &str,
    output: &str,
    nodes: &BTreeMap<String, String>,
    progress: &mut dyn ProgressSink,
    cancel: &Cancel,
) -> Watched
where
    S: Stream<Item = std::result::Result<Frame, E>>,
    E: fmt::Display,
{
    let mut frames = std::pin::pin!(frames);
    let mut started = false;
    let mut running: Option<String> = None;
    let ended = loop {
        let frame = match next(frames.as_mut(), cancel).await {
            Next::Frame(Ok(frame)) => frame,
            Next::Frame(Err(e)) => {
                break Ended::Failed(Error::Unavailable {
                    detail: format!("the connection to ComfyUI dropped mid-render: {e}"),
                });
            }
            // ComfyUI closed the socket. It does that on shutdown, and a proxy
            // does it on an idle timeout; either way the render is unaccounted
            // for, which is not the same as no image having been made.
            Next::End => {
                break Ended::Failed(Error::Unavailable {
                    detail: "ComfyUI closed the connection before the render finished".into(),
                });
            }
            Next::Cancelled => break Ended::Cancelled,
        };

        let text = match frame {
            Frame::Binary(bytes) => {
                // Binary frames carry no prompt id. ComfyUI addresses previews to
                // the client whose graph is executing, so one arriving before our
                // run started belongs to whoever is ahead of us in the queue —
                // and drawing it would show the user a picture of somebody else's
                // work as their own.
                if started && let Some(image) = preview(&bytes) {
                    progress.preview(&crate::comfy::wire::data_url(&image), None);
                }
                continue;
            }
            Frame::Text(text) => text,
        };

        let event = Event::parse(&text);
        // ComfyUI broadcasts every event to every connected client. Anything
        // naming a different run is somebody else's.
        if event.prompt_id().is_some_and(|id| id != prompt_id) {
            continue;
        }
        match event {
            Event::Status { queue_remaining } if !started => {
                // Not a step of ours — there is nothing to be a fraction of yet —
                // but a queue three deep is the difference between "wobu is
                // broken" and "you are third in line".
                progress.step(0, 1, Some(&queued(queue_remaining)));
            }
            Event::ExecutionStart { .. } => {
                started = true;
                progress.step(0, 1, Some("starting"));
            }
            Event::Executing { node, .. } => match node {
                Some(node) => running = Some(node),
                // Older builds end a run with `executing` and a null node rather
                // than with `execution_success`. Reaching it without an image
                // means the graph ran and saved nothing.
                None => break Ended::Failed(Error::NoImage),
            },
            Event::Progress { value, max, .. } => {
                let note =
                    running.as_ref().and_then(|node| nodes.get(node)).map(|class| doing(class));
                progress.step(value, max.max(1), note);
            }
            Event::Executed { node, images, .. } => {
                if node == output && !images.is_empty() {
                    break Ended::Images(images);
                }
            }
            Event::ExecutionError { node_type, message, .. } => {
                break Ended::Failed(node_failed(&node_type, &message));
            }
            // Somebody pressed Interrupt in the ComfyUI web UI, or this is the
            // echo of our own `/interrupt`. Both are the user stopping it, and
            // `error.rs` is explicit that a cancellation is never retried.
            Event::ExecutionInterrupted { .. } => break Ended::Cancelled,
            Event::ExecutionSuccess { .. } => break Ended::Failed(Error::NoImage),
            Event::Status { .. } | Event::Other => {}
        }
    };
    Watched { ended, started }
}

fn queued(remaining: u32) -> String {
    match remaining {
        0 | 1 => "queued".to_owned(),
        ahead => format!("queued, {} ahead", ahead - 1),
    }
}

/// What a node class is doing, in words a status bar can show.
///
/// `backend.rs` asks for this by name: "`note` is what makes 'sampling 12/30'
/// possible where '40%' is not". Unrecognised classes fall back to their own
/// name, which is right for a custom node — the user installed it and knows what
/// it is called.
fn doing(class: &str) -> &str {
    match class {
        "KSampler" | "SamplerCustomAdvanced" | "KSamplerAdvanced" => "sampling",
        "VAEDecode" | "VAEDecodeTiled" => "decoding",
        "CheckpointLoaderSimple" | "UNETLoader" | "DualCLIPLoader" | "VAELoader" => {
            "loading the model"
        }
        "CLIPTextEncode" => "reading the prompt",
        "SaveImage" | "PreviewImage" => "saving",
        other => other,
    }
}

/// A node threw a Python exception.
///
/// The message is ComfyUI's own, because it is the only description of the
/// failure that exists — and an out-of-memory, which is the common one on a
/// local GPU, says so in a sentence the user can act on.
fn node_failed(node_type: &str, message: &str) -> Error {
    let detail = match (node_type, message) {
        ("", "") => "a node in the workflow failed and ComfyUI said nothing about why".to_owned(),
        ("", message) => format!("a node in the workflow failed: {message}"),
        (node_type, "") => {
            format!("the {node_type} node failed and ComfyUI said nothing about why")
        }
        (node_type, message) => format!("the {node_type} node failed: {message}"),
    };
    Error::Unavailable { detail }
}

/// `None` when the cancellation won, in which case `future` is dropped.
///
/// For the requests either side of the render — opening the socket, posting the
/// graph, fetching the finished image. Each is short, and a `Cancel` set while
/// one is in flight should not have to wait it out: a `/prompt` that completes
/// after the user pressed Stop has queued a graph nobody wants.
pub(crate) async fn until_cancelled<F: std::future::Future>(
    future: F,
    cancel: &Cancel,
) -> Option<F::Output> {
    let mut future = std::pin::pin!(future);
    let mut cancelled = std::pin::pin!(cancel.cancelled());
    std::future::poll_fn(move |cx: &mut Context<'_>| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        future.as_mut().poll(cx).map(Some)
    })
    .await
}

pub(crate) enum Next<T, E> {
    Frame(std::result::Result<T, E>),
    End,
    Cancelled,
}

/// The next frame, or the cancellation that beat it.
///
/// Cancellation is polled first, so a token set while a frame was already
/// waiting still wins. A local render is the case that makes this matter most:
/// the frames arrive several a second and there is always one ready, so a loop
/// that checked the flag second would keep the GPU for the rest of the render.
pub(crate) async fn next<S, T, E>(mut frames: Pin<&mut S>, cancel: &Cancel) -> Next<T, E>
where
    S: Stream<Item = std::result::Result<T, E>>,
{
    let mut cancelled = std::pin::pin!(cancel.cancelled());
    std::future::poll_fn(move |cx: &mut Context<'_>| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Next::Cancelled);
        }
        match frames.as_mut().poll_next(cx) {
            Poll::Ready(Some(frame)) => Poll::Ready(Next::Frame(frame)),
            Poll::Ready(None) => Poll::Ready(Next::End),
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

/// Open the progress socket.
///
/// `clientId` is what makes the run ours: ComfyUI keys previews and the
/// `execution_*` events to the client that queued the graph, and a socket opened
/// without it receives the broadcast events and no previews at all.
///
/// **Opened before `/prompt` is posted, never after.** A graph that starts
/// executing before the socket is up loses its first events, and on a cached
/// graph — the second image of a batch, where every node but the sampler is
/// unchanged — it can lose all of them and the run appears to hang.
pub(crate) async fn connect(
    url: &str,
) -> crate::error::Result<
    // `use<>`: the socket outlives the URL that opened it, and under the 2024
    // capture rules an `impl Trait` would otherwise borrow it for as long as the
    // render runs.
    impl Stream<Item = std::result::Result<Frame, tokio_tungstenite::tungstenite::Error>> + use<>,
> {
    // `impl Stream` rather than the concrete `WebSocketStream<MaybeTlsStream<
    // TcpStream>>`, which would mean naming `tokio` in this crate. `lib.rs` is
    // explicit that nothing here names a runtime — Tauri's is the one it runs on
    // — and a dependency added only to spell a type in a signature would read as
    // a decision that was never made.
    match tokio_tungstenite::connect_async(url).await {
        Ok((stream, _)) => Ok(Socket { inner: stream }),
        Err(e) => Err(Error::Unavailable {
            detail: format!("ComfyUI's progress socket at {url} would not open: {e}"),
        }),
    }
}

/// A websocket, as a stream of [`Frame`].
///
/// Generic over the transport so it can be written without naming one. The only
/// thing it does is drop the frames that are the library's business rather than
/// the run's.
struct Socket<S> {
    inner: S,
}

impl<S, E> Stream for Socket<S>
where
    S: Stream<Item = std::result::Result<Message, E>> + Unpin,
{
    type Item = std::result::Result<Frame, E>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            return match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(Message::Text(text)))) => {
                    Poll::Ready(Some(Ok(Frame::Text(text.as_str().to_owned()))))
                }
                Poll::Ready(Some(Ok(Message::Binary(bytes)))) => {
                    Poll::Ready(Some(Ok(Frame::Binary(bytes.to_vec()))))
                }
                Poll::Ready(Some(Ok(Message::Close(_)))) => Poll::Ready(None),
                // Ping, pong and continuation frames are the library's business.
                // Passing them on would make every consumer of this stream
                // re-learn that they are not events.
                Poll::Ready(Some(Ok(_))) => continue,
                Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use crate::testing::block_on;
    /// Stands in for the websocket. Counts its own polls, because "the render
    /// stopped" and "the reporting stopped" are the difference between
    /// cancellation working and cancellation being a lie.
    struct Frames {
        frames: VecDeque<std::result::Result<Frame, String>>,
        polls: Arc<AtomicUsize>,
        /// What happens once they run out. `true` models a server that has gone
        /// quiet mid-render — a model load, a VAE decode, a queue with somebody
        /// else's job in front — which is the case a poll-only cancellation
        /// leaves parked.
        quiet: bool,
    }

    impl Frames {
        fn new(frames: Vec<std::result::Result<Frame, String>>) -> Frames {
            Frames { frames: frames.into(), polls: Arc::new(AtomicUsize::new(0)), quiet: false }
        }

        fn quiet_after(mut self) -> Frames {
            self.quiet = true;
            self
        }
    }

    impl Stream for Frames {
        type Item = std::result::Result<Frame, String>;

        fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let this = self.get_mut();
            this.polls.fetch_add(1, Ordering::SeqCst);
            match this.frames.pop_front() {
                Some(frame) => Poll::Ready(Some(frame)),
                // No waker registered on purpose: only the cancellation can wake
                // this task, which is the situation being modelled.
                None if this.quiet => Poll::Pending,
                None => Poll::Ready(None),
            }
        }
    }

    /// Records what it was told, so both promises about progress can be checked
    /// — that it arrives, and that a preview never arrives from a run that is
    /// not ours.
    #[derive(Default)]
    struct Recorder {
        steps: Vec<(u32, u32, Option<String>)>,
        previews: Vec<String>,
    }

    impl ProgressSink for Recorder {
        fn step(&mut self, done: u32, total: u32, note: Option<&str>) {
            self.steps.push((done, total, note.map(str::to_owned)));
        }

        fn preview(&mut self, image: &str, _step: Option<u32>) {
            self.previews.push(image.to_owned());
        }
    }

    fn text(value: serde_json::Value) -> std::result::Result<Frame, String> {
        Ok(Frame::Text(serde_json::to_string(&value).unwrap()))
    }

    /// A binary preview frame: event 1, format 1, then a JPEG.
    fn latent(byte: u8) -> std::result::Result<Frame, String> {
        Ok(Frame::Binary([&[0, 0, 0, 1, 0, 0, 0, 1][..], &[0xff, 0xd8, byte][..]].concat()))
    }

    fn nodes() -> BTreeMap<String, String> {
        [("3", "KSampler"), ("8", "VAEDecode"), ("9", "SaveImage")]
            .into_iter()
            .map(|(id, class)| (id.to_owned(), class.to_owned()))
            .collect()
    }

    /// One whole render, as ComfyUI reports it.
    fn a_run(prompt: &str) -> Vec<std::result::Result<Frame, String>> {
        let mut frames = vec![
            text(json!({"type": "status", "data": {"status": {"exec_info":
                 {"queue_remaining": 1}}}})),
            text(json!({"type": "execution_start", "data": {"prompt_id": prompt}})),
            text(json!({"type": "executing", "data": {"node": "3", "prompt_id": prompt}})),
        ];
        for step in 1..=4u32 {
            frames.push(text(json!({"type": "progress", "data": {"value": step, "max": 4,
                 "prompt_id": prompt, "node": "3"}})));
            frames.push(latent(step as u8));
        }
        frames.push(text(json!({"type": "executing", "data": {"node": "9", "prompt_id": prompt}})));
        frames.push(text(json!({"type": "executed", "data": {"node": "9", "prompt_id": prompt,
             "output": {"images": [{"filename": "wobu_00001_.png", "subfolder": "",
             "type": "output"}]}}})));
        frames
    }

    fn run(frames: Frames, cancel: &Cancel, sink: &mut Recorder) -> Watched {
        block_on(watch(frames, "ours", "9", &nodes(), sink, cancel))
    }

    #[test]
    fn a_whole_render_reports_its_steps_its_previews_and_the_file_it_wrote() {
        // End to end over the documented frames. The three things the Generate
        // panel draws — a bar, a picture of the work in flight, and the result —
        // all come off this one loop.
        let mut sink = Recorder::default();
        let watched = run(Frames::new(a_run("ours")), &Cancel::new(), &mut sink);

        let images = match watched.ended {
            Ended::Images(images) => images,
            other => panic!("a finished render is an image, not {other:?}"),
        };
        assert_eq!(images[0].filename, "wobu_00001_.png");
        assert!(watched.started);
        assert_eq!(sink.previews.len(), 4, "one latent preview per sampler step");
        assert!(sink.previews[0].starts_with("data:image/jpeg;base64,"));
        assert_eq!(sink.steps.last().unwrap(), &(4, 4, Some("sampling".to_owned())));
        assert_eq!(sink.steps[0], (0, 1, Some("queued".to_owned())));
    }

    #[test]
    fn another_clients_render_is_never_reported_as_ours() {
        // ComfyUI broadcasts to every connected client, so the ComfyUI web UI
        // left open in a browser sends its whole render down our socket. Without
        // the prompt-id filter a wobu generation reports a stranger's sampler
        // steps and finishes on their image — which is then written into this
        // project as the user's own concept art.
        let mut frames = a_run("theirs");
        frames.extend(a_run("ours"));
        let mut sink = Recorder::default();
        let watched = run(Frames::new(frames), &Cancel::new(), &mut sink);

        assert!(matches!(watched.ended, Ended::Images(_)));
        // Four steps from our run, and none from theirs. The queue notice is
        // shared, because queue depth belongs to the server.
        let sampling = sink.steps.iter().filter(|(_, _, note)| note.as_deref() == Some("sampling"));
        assert_eq!(sampling.count(), 4);
        assert_eq!(sink.previews.len(), 4, "and their previews were not drawn as ours either");
    }

    #[test]
    fn cancelling_mid_render_stops_reading_and_says_the_gpu_still_has_it() {
        // The expensive local failure. A loop that read to the end and discarded
        // the answer would leave the render running, holding a GPU the next job
        // in the queue is waiting for — so `started` has to come back out, or
        // the caller has nothing to interrupt.
        let frames = Frames::new(a_run("ours"));
        let polls = Arc::clone(&frames.polls);
        let total = frames.frames.len();
        let cancel = Cancel::new();

        struct Stopper<'a>(&'a Cancel, usize);
        impl ProgressSink for Stopper<'_> {
            fn step(&mut self, _done: u32, _total: u32, _note: Option<&str>) {
                self.1 += 1;
                if self.1 == 3 {
                    self.0.cancel();
                }
            }
        }

        let watched =
            block_on(watch(frames, "ours", "9", &nodes(), &mut Stopper(&cancel, 0), &cancel));

        assert!(matches!(watched.ended, Ended::Cancelled));
        assert!(watched.started, "it had left the queue, so stopping it means /interrupt");
        assert!(
            polls.load(Ordering::SeqCst) < total,
            "the loop read {} of {total} frames after Stop",
            polls.load(Ordering::SeqCst),
        );
    }

    #[test]
    fn a_run_cancelled_while_it_was_still_queued_says_so() {
        // `/interrupt` takes no prompt id and kills whatever is executing. A run
        // of ours that never started is behind somebody else's in the queue, so
        // interrupting on its behalf would stop a stranger's render and leave
        // ours to start immediately afterwards.
        let cancel = Cancel::new();
        cancel.cancel();
        let watched = run(Frames::new(a_run("ours")), &cancel, &mut Recorder::default());
        assert!(matches!(watched.ended, Ended::Cancelled));
        assert!(!watched.started, "nothing of ours is on the GPU, so nothing may be interrupted");
    }

    #[test]
    fn a_render_waiting_on_a_quiet_server_is_woken_by_the_cancellation() {
        // A model load or a VAE decode is twenty seconds of silence with the GPU
        // fully occupied. Polling a flag between frames would leave a stopped job
        // parked until the server next spoke, holding a queue slot the whole
        // time — which is the case worth paying a race for.
        let mut frames = a_run("ours");
        frames.truncate(3);
        let cancel = Cancel::new();
        let stop = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            stop.cancel();
        });

        let watched = run(Frames::new(frames).quiet_after(), &cancel, &mut Recorder::default());
        assert!(matches!(watched.ended, Ended::Cancelled));
        assert!(watched.started);
    }

    #[test]
    fn interrupting_from_the_comfyui_web_ui_ends_the_run_the_same_way_stop_does() {
        // The button exists on the other end and people press it. Reported as an
        // error it would offer a "Try again" for something the user deliberately
        // stopped, which `error.rs` says a cancellation must never do.
        let mut frames = a_run("ours");
        frames.truncate(5);
        frames.push(text(json!({"type": "execution_interrupted", "data": {"prompt_id": "ours",
             "node_id": "3", "node_type": "KSampler"}})));
        let watched = run(Frames::new(frames), &Cancel::new(), &mut Recorder::default());
        assert!(matches!(watched.ended, Ended::Cancelled));
    }

    #[test]
    fn a_node_that_throws_reports_the_exception_and_not_the_node_number() {
        // The common local failure is an out-of-memory in the sampler, and
        // ComfyUI's own sentence says so. "Node 3 failed" sends the user to read
        // a graph they did not write.
        let mut frames = a_run("ours");
        frames.truncate(4);
        frames.push(text(json!({"type": "execution_error", "data": {"prompt_id": "ours",
             "node_id": "3", "node_type": "KSampler",
             "exception_type": "torch.OutOfMemoryError",
             "exception_message": "Allocation on device: 23.44 GiB requested",
             "traceback": ["..."]}})));
        let watched = run(Frames::new(frames), &Cancel::new(), &mut Recorder::default());
        match watched.ended {
            Ended::Failed(error) => {
                let message = error.to_string();
                assert!(message.contains("KSampler"), "{message}");
                assert!(message.contains("23.44 GiB"), "{message}");
                assert!(error.is_retryable(), "a smaller image would fit, so waiting helps");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_graph_that_ran_and_saved_nothing_is_told_apart_from_one_that_broke() {
        // `error.rs`: a silent empty result and a stated failure send the user to
        // two different places, and the first one is a bug report against us.
        let mut frames = a_run("ours");
        frames.truncate(4);
        frames.push(text(json!({"type": "executing", "data": {"node": null,
             "prompt_id": "ours"}})));
        let watched = run(Frames::new(frames), &Cancel::new(), &mut Recorder::default());
        assert!(matches!(watched.ended, Ended::Failed(Error::NoImage)));
    }

    #[test]
    fn a_socket_that_dies_mid_render_is_reported_as_a_dropped_connection() {
        // ComfyUI restarting, or a proxy's idle timeout. The render is
        // unaccounted for, which is not the same as no image having been made —
        // and "Try again" is the right button for both.
        let mut frames = a_run("ours");
        frames.truncate(4);
        frames.push(Err("connection reset by peer".to_string()));
        let watched = run(Frames::new(frames), &Cancel::new(), &mut Recorder::default());
        match watched.ended {
            Ended::Failed(error) => {
                assert!(error.to_string().contains("dropped mid-render"), "{error}");
                assert!(error.is_retryable());
            }
            other => panic!("{other:?}"),
        }

        // And a clean close with no result is the same kind of answer.
        let mut frames = a_run("ours");
        frames.truncate(4);
        let watched = run(Frames::new(frames), &Cancel::new(), &mut Recorder::default());
        assert!(matches!(watched.ended, Ended::Failed(Error::Unavailable { .. })));
    }

    #[test]
    fn only_the_output_node_s_files_are_the_answer() {
        // A template may legitimately save more than one thing — somebody's
        // preview node left in a graph, a debug save. Taking the first `executed`
        // with images would return the wrong picture, and it would look right.
        let mut frames = a_run("ours");
        frames.truncate(4);
        frames.push(text(json!({"type": "executed", "data": {"node": "99", "prompt_id": "ours",
             "output": {"images": [{"filename": "debug.png", "type": "temp"}]}}})));
        frames.push(text(json!({"type": "executed", "data": {"node": "9", "prompt_id": "ours",
             "output": {"images": [{"filename": "wobu_00001_.png", "type": "output"}]}}})));
        match run(Frames::new(frames), &Cancel::new(), &mut Recorder::default()).ended {
            Ended::Images(images) => assert_eq!(images[0].filename, "wobu_00001_.png"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_queue_with_work_in_front_of_us_says_how_much() {
        // "wobu is broken" and "you are third in line" look identical behind a
        // spinner, and only one of them is worth waiting out.
        assert_eq!(queued(0), "queued");
        assert_eq!(queued(1), "queued", "one remaining is our own graph");
        assert_eq!(queued(4), "queued, 3 ahead");
    }
}
