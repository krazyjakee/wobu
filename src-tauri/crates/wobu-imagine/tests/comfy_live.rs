//! The ComfyUI adapter against a server that is actually there.
//!
//! Every one of these is `#[ignore]`d, because they need something listening.
//! Run them against a real ComfyUI with:
//!
//! ```text
//! cargo test -p wobu-imagine --test comfy_live -- --ignored
//! WOBU_COMFYUI_URL=http://192.168.1.4:8188 cargo test -p wobu-imagine \
//!     --test comfy_live -- --ignored
//! ```
//!
//! They exist because there is a seam the unit tests cannot cross. `wire.rs`,
//! `socket.rs` and `workflow.rs` are all driven from recorded payloads, which
//! checks what the adapter does with bytes it is given — and says nothing about
//! whether the bytes it *sends* are ones ComfyUI accepts, whether the websocket
//! handshake succeeds, or whether `/interrupt` reaches a render that is already
//! on the GPU. Those only fail in front of a user.
//!
//! Nothing here asserts on a checkpoint name or an image's contents: this
//! machine's ComfyUI is not the reader's. What is asserted is the shape of the
//! agreement — a model list that is not empty, a render that reports steps and
//! previews and comes back the size it was asked for, and a Stop that the server
//! agrees stopped something.

use std::env;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wobu_imagine::comfy::ComfyBackend;
use wobu_imagine::{
    AspectRatio, Cancel, Error, ImageBackend, ImageRequest, ProgressSink, negotiate,
};

/// Where to find it. The default is ComfyUI's own.
fn url() -> String {
    env::var("WOBU_COMFYUI_URL").unwrap_or_else(|_| wobu_imagine::comfy::DEFAULT_URL.to_owned())
}

/// Shared so the cancellation test can watch progress from another thread while
/// the render is in flight.
#[derive(Default)]
struct Seen {
    steps: Vec<(u32, u32, Option<String>)>,
    previews: usize,
}

#[derive(Clone, Default)]
struct Watch(Arc<Mutex<Seen>>);

impl Watch {
    fn seen(&self) -> std::sync::MutexGuard<'_, Seen> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl ProgressSink for Watch {
    fn step(&mut self, done: u32, total: u32, note: Option<&str>) {
        self.seen().steps.push((done, total, note.map(str::to_owned)));
    }

    fn preview(&mut self, _image: &str, _step: Option<u32>) {
        self.seen().previews += 1;
    }
}

async fn connected() -> ComfyBackend {
    ComfyBackend::connect(url()).await.unwrap_or_else(|e| panic!("no ComfyUI at {} — {e}", url()))
}

/// A request for the first model this server has, at the smallest shape a preset
/// asks for. Deliberately not the largest: these run on somebody's real card.
fn request(backend: &ComfyBackend, model: &str) -> ImageRequest {
    let caps = backend.capabilities(model);
    let negotiated = negotiate(&[], AspectRatio::parse("1:1").unwrap(), &caps);
    ImageRequest::new(model, "a plain grey sphere on a white background", 424242, &negotiated)
        .with_negative(if caps.negative_prompt { "text, watermark" } else { "" })
}

#[tokio::test]
#[ignore = "needs a ComfyUI"]
async fn a_live_server_reports_the_models_it_actually_has() {
    // The claim probing makes. A model list compiled into wobu would name
    // checkpoints this machine has never downloaded, and each one is a 400 after
    // the user picked it from a dropdown.
    let backend = connected().await;
    let installed = backend.installed().expect("connect leaves a probe behind");

    assert!(
        !installed.checkpoints().is_empty() || !installed.unets().is_empty(),
        "this ComfyUI has no models at all, so there is nothing to render with",
    );
    assert!(installed.has_class("KSampler"), "every ComfyUI has the core nodes");
    let model = backend.suggested_model().expect("a model list means a suggestion");
    assert!(
        installed.checkpoints().contains(&model) || installed.unets().contains(&model),
        "the suggestion has to be a file this server can actually load",
    );

    // And the machine underneath, which is where the resolution ceiling comes
    // from — there is no documented one for a local backend.
    let server = backend.server().expect("system_stats came back");
    println!(
        "ComfyUI {} · {} checkpoints · {} unets · {} loras · {} controlnets · vram {:?}",
        server.version.as_deref().unwrap_or("(no version)"),
        installed.checkpoints().len(),
        installed.unets().len(),
        installed.loras().len(),
        installed.controlnets().len(),
        server.vram_bytes,
    );
}

#[tokio::test]
#[ignore = "needs a ComfyUI"]
async fn the_status_bar_line_comes_back_from_a_live_server() {
    // #51's line, with the queue depth read off the server rather than invented.
    let backend = connected().await;
    let health = backend.health("flux-dev").await;
    assert!(health.is_connected(), "{health}");
    assert!(health.to_string().starts_with("ComfyUI connected · flux-dev · queue "), "{health}");
    println!("{health}");

    // And the other half: a port with nothing on it is a diagnosis, not a
    // spinner. 1 is reserved and never listened on.
    let nothing = ComfyBackend::new("http://127.0.0.1:1").unwrap().health("m").await;
    assert!(!nothing.is_connected());
    println!("{nothing}");
}

#[tokio::test]
#[ignore = "needs a ComfyUI"]
async fn a_whole_render_arrives_with_previews_and_the_size_it_actually_is() {
    // The seam nothing offline can cross: that the graph we build is one ComfyUI
    // accepts, that the websocket handshake works, and that the file named in
    // `executed` can be fetched back through `/view`.
    let backend = connected().await;
    let model = backend.suggested_model().expect("a model to render with");
    let request = request(&backend, &model);
    let mut watch = Watch::default();

    let outcome = backend.generate(&request, &mut watch, &Cancel::new()).await;
    let image = match outcome.result {
        Ok(image) => image,
        Err(e) => panic!("render failed: {e}"),
    };

    assert_eq!(outcome.usage.billed_images, 0, "a local render is not metered");
    assert_eq!(image.resolution(), request.resolution, "the latent got the dimensions");
    assert_eq!(image.seed, Some(request.seed), "and the seed we can reproduce it with");
    assert!(image.bytes.len() > 1000, "an image, not an error page");
    assert!(image.mime.starts_with("image/"), "{}", image.mime);

    let seen = watch.seen();
    assert!(!seen.steps.is_empty(), "a render with no progress is an indeterminate spinner");
    assert!(
        seen.steps.iter().any(|(_, _, note)| note.as_deref() == Some("sampling")),
        "the sampler node was named, so the bar can say what it is doing: {:?}",
        seen.steps,
    );
    assert!(
        seen.previews > 0,
        "this backend declares streaming_preview, so a render that draws none is an empty \
         box for its whole duration",
    );
    println!("{} steps, {} previews, {}", seen.steps.len(), seen.previews, image.resolution());
}

#[tokio::test]
#[ignore = "needs a ComfyUI"]
async fn stopping_a_render_interrupts_it_rather_than_leaving_it_on_the_gpu() {
    // The failure this adapter is most at risk of. Dropping the websocket stops
    // the reporting and returns immediately, which *looks* exactly like
    // cancelling — and the graph runs to completion on a card the next job in
    // the queue is waiting for.
    //
    // Checked from the server's side, which is the only place the difference is
    // visible: after the Stop, `/queue` must have nothing of ours running.
    let backend = connected().await;
    let model = backend.suggested_model().expect("a model to render with");
    let request = request(&backend, &model);
    let watch = Watch::default();

    let cancel = Cancel::new();
    let stop = cancel.clone();
    let watching = watch.clone();
    std::thread::spawn(move || {
        // Long enough that the render is on the GPU rather than still queued,
        // which is the case where `/interrupt` is the right call and dropping
        // the socket is not.
        for _ in 0..200 {
            if watching.seen().steps.iter().any(|(done, _, _)| *done > 0) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        stop.cancel();
    });

    let mut sink = watch.clone();
    let outcome = backend.generate(&request, &mut sink, &cancel).await;
    let error = outcome.result.expect_err("a stopped render has no image");
    assert!(matches!(error, Error::Cancelled), "{error}");
    assert!(!error.is_retryable(), "a queue that retries a cancellation ignores the Stop button");

    // The server's own account. `queue_remaining` counts running and pending
    // together, so a graph still on the GPU shows up here — and it is allowed a
    // moment, because `/interrupt` is asynchronous on ComfyUI's side too.
    for _ in 0..40 {
        if let wobu_imagine::comfy::Health::Connected { queue, .. } = backend.health(&model).await
            && queue == 0
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("the render was still on the GPU ten seconds after Stop");
}
