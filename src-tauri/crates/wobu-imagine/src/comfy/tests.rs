use super::*;

use std::sync::Arc;
use wobu_influence::Refs;

use crate::aspect::AspectRatio;
use serde_json::json;

use crate::testing::block_on;
fn object_info() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "CheckpointLoaderSimple": {
            "input": {"required": {"ckpt_name": [["sd_xl_base_1.0.safetensors"], {}]}},
        },
        "UNETLoader": {"input": {"required": {"unet_name": [["flux1-dev.safetensors"], {}]}}},
        "LoraLoader": {"input": {"required": {"lora_name": [["ashfall.safetensors"], {}]}}},
        "ControlNetLoader": {
            "input": {"required": {"control_net_name": [["openpose.pth"], {}]}},
        },
        "KSampler": {}, "EmptyLatentImage": {}, "CLIPTextEncode": {}, "VAEDecode": {},
        "SaveImage": {},
    }))
    .unwrap()
}

#[test]
fn health_checks_and_generation_backends_share_the_http_pool() {
    // A status check and a generation construct separate backends, but the
    // reqwest client beneath them must retain its connections and TLS state.
    let health = ComfyBackend::new(DEFAULT_URL).unwrap();
    let job = ComfyBackend::new("https://comfy.example").unwrap();
    assert!(Arc::ptr_eq(&health.client, &job.client));
    assert_ne!(health.client_id, job.client_id, "websocket routing stays per backend");
}

fn system_stats(vram: u64) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "system": {"os": "posix", "comfyui_version": "0.3.68"},
        "devices": [{"name": "cuda:0", "type": "cuda", "vram_total": vram}],
    }))
    .unwrap()
}

/// A backend with a probe already in it, which is what
/// [`ComfyBackend::connect`] leaves behind. Built by hand because the only
/// other way to get one is a server.
fn probed(vram: u64) -> ComfyBackend {
    let backend = ComfyBackend::new(DEFAULT_URL).unwrap();
    *backend.probed.write().unwrap() = Some(Probed {
        installed: Installed::parse(&object_info()).unwrap(),
        server: Server::parse(&system_stats(vram)).unwrap(),
    });
    backend
}

#[test]
fn the_status_bar_line_is_the_one_the_issue_asks_for() {
    // Verbatim from #51. It is the first place anybody looks when Generate
    // does nothing, so the disconnected form has to carry the diagnosis
    // rather than the word "error".
    let connected = Health::Connected { model: "flux-dev".into(), queue: 0 };
    assert_eq!(connected.to_string(), "ComfyUI connected · flux-dev · queue 0");
    assert!(connected.is_connected());

    let down = Health::Unreachable {
        detail: unreachable_detail("nothing is listening at http://127.0.0.1:8188"),
    };
    assert!(down.to_string().starts_with("ComfyUI unreachable — nothing is listening"));
    assert!(!down.is_connected());
}

fn unreachable_detail(detail: &str) -> String {
    detail.to_owned()
}

#[test]
fn the_three_failures_a_user_can_act_on_read_as_three_different_things() {
    // #51's deliverable: unreachable, wrong port, and a missing node are
    // three different sentences pointing at three different fixes. Reported
    // as a status code they are one sentence pointing at nothing.
    let not_running = not_listening();
    assert!(not_running.contains("nothing is listening"), "{not_running}");
    assert!(not_running.contains("--port"), "{not_running}");

    let wrong_port = not_comfyui("http://127.0.0.1:7860").to_string();
    assert!(wrong_port.contains("it is not ComfyUI"), "{wrong_port}");
    assert!(wrong_port.contains("8188"), "{wrong_port}");

    let missing = missing_nodes(&["IPAdapterAdvanced".to_string()]).to_string();
    assert!(missing.contains("`IPAdapterAdvanced`"), "{missing}");
    assert!(missing.contains("custom node pack"), "{missing}");

    // All three are worth another attempt once the user has done the thing,
    // which is what puts a "Try again" button on them.
    for error in [
        missing_nodes(&["A".into()]),
        not_comfyui("http://x"),
        no_such_model("a.safetensors", &Installed::parse(&object_info()).unwrap()),
    ] {
        assert!(error.is_retryable(), "{error}");
        assert_eq!(error.code(), "provider.unavailable");
    }

    // And none of them is the same sentence as another, which is the whole
    // claim.
    let all = [not_running, wrong_port, missing];
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        assert_ne!(all[a], all[b]);
    }
}

/// The message a refused connection produces, without a network. `reqwest`
/// errors cannot be constructed from outside the crate, so this is the same
/// branch reached through the one input that decides it.
fn not_listening() -> String {
    Error::Unavailable {
        detail: format!(
            "nothing is listening at {DEFAULT_URL}. Start {LABEL}, or point wobu at the \
             address it is running on — `--listen` and `--port` both move it"
        ),
    }
    .to_string()
}

#[test]
fn a_model_this_server_does_not_have_is_answered_with_the_ones_it_does() {
    // A dead end and a choice, from the same 400. The list is short on
    // purpose: a machine with sixty checkpoints would otherwise put all
    // sixty in a toast.
    let installed = Installed::parse(&object_info()).unwrap();
    let error = no_such_model("ashfall_v3.safetensors", &installed).to_string();
    assert!(error.contains("ashfall_v3.safetensors"), "{error}");
    assert!(error.contains("sd_xl_base_1.0.safetensors"), "{error}");
    assert!(error.contains("flux1-dev.safetensors"), "and the unet models too: {error}");

    // A fresh clone with nothing downloaded is its own message, because
    // "wobu cannot find your model" is misleading when there are none.
    let empty = no_such_model("anything", &Installed::default()).to_string();
    assert!(empty.contains("no models installed at all"), "{empty}");
}

#[test]
fn capabilities_are_read_off_the_probe_and_differ_per_model() {
    // The reason `capabilities` takes a model id. Flux's guidance chain has
    // no negative conditioning, so the same server answers differently for
    // two files it can both load — and `negotiate` compiles a `never:` list
    // for one of them and reports it withheld for the other.
    let backend = probed(24 * (1 << 30));
    assert!(backend.capabilities("sd_xl_base_1.0.safetensors").negative_prompt);
    assert!(!backend.capabilities("flux1-dev.safetensors").negative_prompt);

    // LoRAs are declared because this server has one. A UI drawing a picker
    // off this shows it here and hides it on a server with none.
    assert!(backend.capabilities("sd_xl_base_1.0.safetensors").loras);
}

#[test]
fn an_unprobed_backend_claims_nothing_it_has_not_checked() {
    // The asymmetry that makes probing worth doing. A capability declared
    // and absent is a 400 with a traceback in it; a capability present and
    // undeclared is a downgrade the user is told about and can fix by
    // reconnecting. So the unprobed answer is the conservative one.
    let backend = ComfyBackend::new("localhost:8188").unwrap();
    let caps = backend.capabilities("anything.safetensors");
    assert!(!caps.loras, "no probe means no LoRA picker");
    assert_eq!(caps.reference_mechanisms, ReferenceMechanisms::none());
    assert_eq!(caps.max_resolution, Resolution::new(1024, 1024), "the smallest band");
    assert!(!caps.requires_billing, "and this one is known without asking");
    assert!(backend.installed().is_none());
    assert_eq!(backend.suggested_model(), None);
}

#[test]
fn no_structure_mechanism_is_declared_even_where_the_models_are_installed() {
    // The discipline #51 asks for, applied against ourselves: this server
    // has `openpose.pth` and the loader to open it, and no workflow shipped
    // here can reach either. Declaring `true` would route a silhouette
    // reference into a graph with nowhere to put it, which is the
    // unactionable 400 all over again.
    let backend = probed(24 * (1 << 30));
    let installed = backend.installed().unwrap();
    assert!(installed.has_controlnet(), "the server really does have one");
    assert_eq!(
        backend.capabilities("sd_xl_base_1.0.safetensors").reference_mechanisms.structure,
        Refs::new(0),
        "wobu has no graph that uses it, so it must not claim an input",
    );
    // Which is why the probe result is public: the Inspector can say why the
    // reference was downgraded rather than leaving the user to guess.
    assert_eq!(installed.controlnets(), ["openpose.pth"]);
}

#[test]
fn unreachable_inputs_are_not_misrepresented_as_vendor_counting_caps() {
    let caps = probed(24 * (1 << 30)).capabilities("sd_xl_base_1.0.safetensors");
    assert_eq!(caps.reference_mechanisms, ReferenceMechanisms::none());
    assert_eq!(caps.image_refs, ImageBudget::unlimited());
}

#[test]
fn the_ceiling_follows_the_card_and_can_be_overridden_by_someone_who_knows_better() {
    // There is no documented ceiling for a local backend, and a compiled-in
    // number is either a card nobody has or a limit that wastes the one they
    // bought. Bands rather than a formula, because the real answer depends on
    // the architecture and on whether the VAE decode is tiled.
    assert_eq!(ceiling(Some(24 * (1 << 30))), Resolution::new(2048, 2048));
    assert_eq!(ceiling(Some(12 * (1 << 30))), Resolution::new(1536, 1536));
    assert_eq!(ceiling(Some(6 * (1 << 30))), Resolution::new(1024, 1024));
    assert_eq!(ceiling(None), Resolution::new(1024, 1024), "a CPU-only install renders too");

    let backend = probed(6 * (1 << 30)).with_max_resolution(Resolution::new(3072, 3072));
    assert_eq!(
        backend.capabilities("sd_xl_base_1.0.safetensors").max_resolution,
        Resolution::new(3072, 3072),
    );
}

#[test]
fn a_shape_asked_for_is_the_shape_sent_because_comfyui_takes_pixels() {
    // `capability.rs`: an empty aspect list means the parameter is not taken,
    // not that every value is refused. Every preset's aspect has to survive
    // to the graph, or an environment matte comes back square.
    let backend = probed(24 * (1 << 30));
    let caps = backend.capabilities("sd_xl_base_1.0.safetensors");
    for aspect in AspectRatio::ALL {
        assert!(caps.supports_aspect(aspect), "{aspect}");
        assert_eq!(caps.nearest_aspect(aspect), aspect);
    }
    assert_eq!(
        caps.resolution_for(AspectRatio::parse("21:9").unwrap()),
        Resolution::new(2048, 877),
    );
}

#[test]
fn a_server_address_is_read_the_way_people_write_it() {
    // This is a field somebody types into Settings. A missing scheme or a
    // trailing slash producing "could not reach ComfyUI at
    // localhost:8188//system_stats" is a failure with nothing to act on.
    assert_eq!(normalise("localhost:8188"), "http://localhost:8188");
    assert_eq!(normalise("http://127.0.0.1:8188/"), "http://127.0.0.1:8188");
    assert_eq!(normalise("  http://192.168.1.4:8188  "), "http://192.168.1.4:8188");
    assert_eq!(normalise("https://comfy.example/proxy"), "https://comfy.example/proxy");
    assert_eq!(normalise(""), DEFAULT_URL);
}

#[test]
fn the_websocket_url_follows_the_server_and_carries_the_client_id() {
    // Without `clientId` ComfyUI runs the graph and broadcasts to everyone,
    // and no preview is addressed to us — which reads as a backend that
    // declares `streaming_preview` and never draws one.
    let backend = ComfyBackend::new("localhost:8188").unwrap();
    let url = backend.ws_url();
    assert!(url.starts_with("ws://localhost:8188/ws?clientId=wobu-"), "{url}");

    let secure = ComfyBackend::new("https://comfy.example").unwrap();
    assert!(secure.ws_url().starts_with("wss://comfy.example/ws?clientId="));

    // And two backends in one process do not share one, or they would each
    // receive the other's previews.
    assert_ne!(backend.client_id, secure.client_id);
}

#[test]
fn the_running_entry_is_matched_on_the_prompt_id_and_not_on_position() {
    // `/interrupt` takes no prompt id, so this is the whole of the check that
    // decides whether it is safe to call. Reading the wrong field stops a
    // render that is not ours — and the user, who pressed Stop, is told it
    // worked.
    let queue = json!({
        "queue_running": [[7, "theirs", {}, {}, []]],
        "queue_pending": [[8, "ours", {}, {}, []]],
    });
    assert!(running(&queue, "theirs"));
    assert!(!running(&queue, "ours"), "queued is not running, and must not be interrupted");
    assert!(!running(&json!({"queue_running": []}), "ours"));
    assert!(!running(&json!({}), "ours"));
}

#[test]
fn a_backend_works_through_a_box_dyn_and_needs_no_server_to_build() {
    // `project.json` names the backend, so the generate path holds a
    // `Box<dyn ImageBackend>` — and the Inspector draws a backend dropdown
    // on a machine where nothing is running, so constructing one must not
    // touch the network.
    let boxed: Box<dyn ImageBackend> = Box::new(ComfyBackend::new(DEFAULT_URL).unwrap());
    assert_eq!(boxed.id(), ID);
    assert_eq!(boxed.label(), LABEL);
    assert_eq!(boxed.default_model(), DEFAULT_MODEL);
    assert!(!boxed.capabilities(DEFAULT_MODEL).requires_billing);
}

#[test]
fn a_cancelled_job_never_queues_a_graph() {
    // The queue can cancel a job between queueing it and starting it.
    // Locally the cost is not money — it is the next job waiting behind a
    // render nobody wants, on a GPU that will not be free for ten minutes.
    let backend = ComfyBackend::new("http://127.0.0.1:1")
        .unwrap()
        .with_max_resolution(Resolution::new(1024, 1024));
    let caps = backend.capabilities(DEFAULT_MODEL);
    let negotiated = crate::negotiate::negotiate(&[], AspectRatio::parse("1:1").unwrap(), &caps);
    let request = ImageRequest::new(DEFAULT_MODEL, "a hooded figure", 1, &negotiated);

    let cancel = Cancel::new();
    cancel.cancel();
    let outcome = block_on(backend.generate(&request, &mut crate::backend::Discard, &cancel));

    assert!(matches!(outcome.result, Err(Error::Cancelled)));
    assert_eq!(outcome.usage, ImageUsage::free());
    assert!(!outcome.usage.is_billed(), "and a local render never is");
}

#[test]
fn a_generation_against_a_model_this_server_lacks_never_opens_a_socket() {
    // The pre-flight order matters: probing, then the model, then the nodes,
    // then the graph — all before anything is sent. A backend that opened a
    // socket first would leave one hanging on every mistyped model name.
    let backend = probed(24 * (1 << 30));
    let caps = backend.capabilities("sd_xl_base_1.0.safetensors");
    let negotiated = crate::negotiate::negotiate(&[], AspectRatio::parse("1:1").unwrap(), &caps);
    let request = ImageRequest::new("never_downloaded.safetensors", "p", 1, &negotiated);

    let outcome =
        block_on(backend.generate(&request, &mut crate::backend::Discard, &Cancel::new()));
    let error = outcome.result.unwrap_err();
    assert!(error.to_string().contains("never_downloaded.safetensors"), "{error}");
    assert_eq!(outcome.usage, ImageUsage::free());
}

#[test]
fn debug_output_does_not_list_every_model_on_the_users_disk() {
    // There is no key to leak here, but `Installed` is a list of every file
    // in their models folders, and a `{backend:?}` in a log line is not
    // somewhere to put it.
    let printed = format!("{:?}", probed(24 * (1 << 30)));
    assert!(!printed.contains("sd_xl_base_1.0.safetensors"), "{printed}");
    assert!(printed.contains(DEFAULT_URL), "{printed}");
}
