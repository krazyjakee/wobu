//! What is actually installed in *this* ComfyUI, read from `/object_info`.
//!
//! Two ComfyUI installs on two machines share a name and nothing else. One has
//! ControlNet models and thirty custom node packs; the next is a fresh clone
//! with a single checkpoint. Declaring capabilities from the first and running
//! against the second produces a 400 with a Python traceback in it, which is the
//! failure [#51](https://github.com/krazyjakee/wobu/issues/51) singles out: the
//! user is told a node validation failed and is given no way to find out which
//! pack to install.
//!
//! So nothing here is assumed. `/object_info` is one request that answers all
//! three questions the capability declaration has to answer — which node classes
//! exist, which model files each loader can see, and therefore whether ControlNet
//! and LoRAs are usable rather than merely mentioned in a menu.
//!
//! ## Installed is not the same as usable
//!
//! `ControlNetLoader` is a core node and is present in every ComfyUI ever built.
//! Its `control_net_name` list is empty until somebody downloads a model. A probe
//! that read the class list alone would declare ControlNet on every install in
//! the world, and every structure reference would reach a loader with nothing to
//! load. Both halves are required, which is why [`Installed`] keeps the class
//! set and the file lists apart and [`Installed::has_controlnet`] asks for both.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::comfy::workflow::Family;

/// The `/object_info` path, relative to the server root.
pub(crate) const OBJECT_INFO: &str = "object_info";

/// The `/system_stats` path: what ComfyUI is and what it is running on.
pub(crate) const SYSTEM_STATS: &str = "system_stats";

/// The machine underneath, as far as it bears on what can be asked of it.
///
/// One field of it does real work. `max_resolution` has no documented answer for
/// a local backend — the ceiling is VRAM and the checkpoint's training
/// resolution, and ComfyUI reports one of those — so a compiled-in number would
/// be either a card nobody has or a limit that wastes the one they bought. The
/// alternative to reading it is asking the user for a number they would have to
/// guess at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Server {
    /// For the log and for a bug report. Never parsed: ComfyUI's version is a
    /// git describe on some builds and a release tag on others, and an adapter
    /// that branched on it would break on the first build that spelled it
    /// differently.
    pub version: Option<String>,
    /// The largest device's total VRAM. `None` on a CPU-only install, which is a
    /// real configuration and renders — slowly.
    pub vram_bytes: Option<u64>,
}

impl Server {
    pub(crate) fn parse(body: &[u8]) -> Option<Server> {
        let Ok(stats @ Value::Object(_)) = serde_json::from_slice::<Value>(body) else {
            return None;
        };
        // `/system_stats` is the cheapest endpoint that only ComfyUI serves, so
        // it is also the reachability probe. A body without `system` is
        // something else answering on the port.
        stats.get("system")?;
        Some(Server {
            version: stats
                .pointer("/system/comfyui_version")
                .and_then(Value::as_str)
                .map(str::to_owned),
            vram_bytes: stats.get("devices").and_then(Value::as_array).and_then(|devices| {
                devices
                    .iter()
                    .filter_map(|device| device.get("vram_total").and_then(Value::as_u64))
                    .max()
            }),
        })
    }
}

/// What one ComfyUI has, at the moment it was asked.
///
/// A snapshot rather than a live view. A user who installs a node pack while
/// wobu is open has to reconnect, which is the same deal the ComfyUI web UI
/// offers and is honest: the alternative is re-probing before every generation
/// and paying a multi-megabyte response for each image of a turnaround.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Installed {
    classes: BTreeSet<String>,
    checkpoints: Vec<String>,
    unets: Vec<String>,
    loras: Vec<String>,
    controlnets: Vec<String>,
    mesh_models: Vec<String>,
}

impl Installed {
    /// Read a `/object_info` response.
    ///
    /// Total, and deliberately forgiving of shapes it does not recognise: this
    /// document is several megabytes describing every node on the machine,
    /// including packs written by strangers, and one node with an input shape
    /// nobody anticipated must not cost the whole probe. Anything unreadable
    /// leaves a list empty, which reads downstream as "not installed" — the
    /// conservative answer, and the one that produces a downgrade the user is
    /// told about rather than a 400 they are not.
    pub(crate) fn parse(body: &[u8]) -> Option<Installed> {
        let Ok(Value::Object(object_info)) = serde_json::from_slice::<Value>(body) else {
            return None;
        };
        let combo = |class: &str, input: &str| -> Vec<String> {
            object_info.get(class).map(|node| options(node, input)).unwrap_or_default()
        };
        Some(Installed {
            classes: object_info.keys().cloned().collect(),
            checkpoints: combo("CheckpointLoaderSimple", "ckpt_name"),
            unets: combo("UNETLoader", "unet_name"),
            loras: combo("LoraLoader", "lora_name"),
            controlnets: combo("ControlNetLoader", "control_net_name"),
            mesh_models: combo("Hy3D_2_1SimpleMeshGen", "model"),
        })
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.classes.contains(class)
    }

    pub fn checkpoints(&self) -> &[String] {
        &self.checkpoints
    }

    pub fn unets(&self) -> &[String] {
        &self.unets
    }

    /// The LoRA files this install can load, which is what the UI's LoRA picker
    /// is drawn from. Empty is a picker that is not shown, not a picker that
    /// silently does nothing.
    pub fn loras(&self) -> &[String] {
        &self.loras
    }

    /// The ControlNet models this install can load.
    ///
    /// Public even though no shipped workflow uses one yet, so the Inspector can
    /// say "you have three ControlNet models and wobu has no graph for them"
    /// rather than leaving the user to guess why a silhouette reference was
    /// downgraded.
    pub fn controlnets(&self) -> &[String] {
        &self.controlnets
    }

    /// Hunyuan3D 2.1 checkpoints offered by the installed custom node.
    pub fn mesh_models(&self) -> &[String] {
        &self.mesh_models
    }

    /// Whether a structure reference could be honoured: the loader exists *and*
    /// there is something for it to load. See the module note.
    pub fn has_controlnet(&self) -> bool {
        self.has_class("ControlNetLoader") && !self.controlnets.is_empty()
    }

    /// Whether a LoRA picker would have anything in it.
    pub fn has_loras(&self) -> bool {
        self.has_class("LoraLoader") && !self.loras.is_empty()
    }

    /// Which loader chain this model needs, or `None` when this ComfyUI has
    /// never heard of the file.
    ///
    /// Checkpoints are checked first: an all-in-one Flux checkpoint is in both
    /// lists on some installs, and `CheckpointLoaderSimple` is the loader that
    /// can open it.
    pub(crate) fn family_of(&self, model: &str) -> Option<Family> {
        if self.checkpoints.iter().any(|name| name == model) {
            Some(Family::Checkpoint)
        } else if self.unets.iter().any(|name| name == model) {
            Some(Family::Unet)
        } else {
            None
        }
    }

    /// The classes a workflow needs and this install does not have, in the order
    /// the graph declares them.
    ///
    /// The input to the one diagnosis a status code cannot give.
    pub(crate) fn missing(&self, classes: &[String]) -> Vec<String> {
        classes.iter().filter(|class| !self.has_class(class)).cloned().collect()
    }
}

/// The values a combo input offers, across the two shapes ComfyUI has used.
///
/// Historically an input's type slot is a list of the choices themselves —
/// `["ckpt_name", [["a.safetensors", "b.safetensors"], {}]]`. Newer builds send
/// the literal `"COMBO"` with the choices under `options` in the trailing
/// metadata object. Reading only the first would make wobu declare no
/// checkpoints at all against a current ComfyUI, and reading only the second
/// would do the same against every install older than the change; neither
/// failure says which it was.
fn options(node: &Value, input: &str) -> Vec<String> {
    let strings = |value: &Value| -> Option<Vec<String>> {
        Some(value.as_array()?.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
    };
    for section in ["required", "optional"] {
        let Some(spec) = node.pointer(&format!("/input/{section}/{input}")) else {
            continue;
        };
        if let Some(names) = spec.get(0).and_then(strings) {
            return names;
        }
        if let Some(names) = spec.get(1).and_then(|meta| meta.get("options")).and_then(strings) {
            return names;
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A `/object_info` response in the shape ComfyUI documents, cut down to the
    /// classes this crate reads. The real document describes several hundred
    /// nodes and is megabytes long; nothing here depends on its size.
    fn object_info() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "CheckpointLoaderSimple": {
                "input": {"required": {"ckpt_name": [["sd_xl_base_1.0.safetensors",
                                                     "dreamshaper_8.safetensors"], {}]}},
                "output": ["MODEL", "CLIP", "VAE"],
                "name": "CheckpointLoaderSimple",
                "category": "loaders",
            },
            "UNETLoader": {
                "input": {"required": {"unet_name": [["flux1-dev.safetensors"], {}],
                                       "weight_dtype": [["default", "fp8_e4m3fn"], {}]}},
                "output": ["MODEL"],
            },
            "LoraLoader": {
                "input": {"required": {"lora_name": [["ashfall_style.safetensors"], {}]}},
            },
            "ControlNetLoader": {
                "input": {"required": {"control_net_name": [[], {}]}},
            },
            "KSampler": {"input": {"required": {}}},
            "EmptyLatentImage": {"input": {"required": {}}},
            "CLIPTextEncode": {"input": {"required": {}}},
            "VAEDecode": {"input": {"required": {}}},
            "SaveImage": {"input": {"required": {}}},
        }))
        .unwrap()
    }

    #[test]
    fn the_models_a_backend_offers_are_the_files_this_server_can_actually_see() {
        // The whole point of probing. A model list compiled into wobu would name
        // checkpoints this machine has never downloaded, and every one of them
        // is a 400 after the user has picked it from a dropdown.
        let installed = Installed::parse(&object_info()).unwrap();
        assert_eq!(
            installed.checkpoints(),
            ["sd_xl_base_1.0.safetensors", "dreamshaper_8.safetensors",]
        );
        assert_eq!(installed.unets(), ["flux1-dev.safetensors"]);
        assert_eq!(installed.family_of("flux1-dev.safetensors"), Some(Family::Unet));
        assert_eq!(installed.family_of("sd_xl_base_1.0.safetensors"), Some(Family::Checkpoint),);
        assert_eq!(installed.family_of("something_never_downloaded.safetensors"), None);
    }

    #[test]
    fn a_loader_with_no_models_behind_it_is_not_a_capability() {
        // `ControlNetLoader` ships with ComfyUI and is present on every install
        // in the world; its list is empty until somebody downloads a model. A
        // probe that checked the class alone would declare ControlNet
        // everywhere, and #51's whole complaint is a backend claiming an adapter
        // it does not have.
        let installed = Installed::parse(&object_info()).unwrap();
        assert!(installed.has_class("ControlNetLoader"));
        assert!(installed.controlnets().is_empty());
        assert!(!installed.has_controlnet(), "the loader is there and there is nothing to load");

        // And the mirror: a LoRA the user really does have.
        assert!(installed.has_loras());
        assert_eq!(installed.loras(), ["ashfall_style.safetensors"]);
    }

    #[test]
    fn a_class_the_workflow_needs_and_this_machine_lacks_is_named() {
        // The one diagnosis a status code cannot give. ComfyUI answers a graph
        // with an unknown node with a validation failure; the deliverable is the
        // name of the node, because that is what the user searches for.
        let installed = Installed::parse(&object_info()).unwrap();
        let sd = crate::comfy::workflow::Workflow::for_family(Family::Checkpoint);
        assert!(installed.missing(&sd.classes()).is_empty(), "a stock ComfyUI runs this graph");

        let flux = crate::comfy::workflow::Workflow::for_family(Family::Unet);
        assert_eq!(
            installed.missing(&flux.classes()),
            [
                "DualCLIPLoader",
                "VAELoader",
                "FluxGuidance",
                "EmptySD3LatentImage",
                "RandomNoise",
                "KSamplerSelect",
                "BasicScheduler",
                "BasicGuider",
                "SamplerCustomAdvanced",
            ],
            "and in graph order, so the first name is the first thing to install",
        );
    }

    #[test]
    fn a_combo_written_the_new_way_reads_the_same_as_the_old_way() {
        // ComfyUI moved combo inputs from "the type slot is the list of choices"
        // to `"COMBO"` with the choices under `options`. Reading one shape only
        // means wobu declares no checkpoints against half the installs in
        // existence, and the failure is a dropdown that is simply empty.
        let new_shape = serde_json::to_vec(&json!({
            "CheckpointLoaderSimple": {
                "input": {"required": {"ckpt_name": ["COMBO", {"options": ["a.safetensors"],
                                                               "tooltip": "the checkpoint"}]}},
            },
        }))
        .unwrap();
        let installed = Installed::parse(&new_shape).unwrap();
        assert_eq!(installed.checkpoints(), ["a.safetensors"]);
    }

    #[test]
    fn the_hunyuan_node_advertises_the_shape_models_it_can_open() {
        let object_info = serde_json::to_vec(&json!({
            "Hy3D_2_1SimpleMeshGen": {
                "input": {"required": {
                    "model": [["hunyuan3d-dit-v2-1.ckpt"], {}],
                    "image": ["IMAGE", {}]
                }}
            },
            "Hy3DPostprocessMesh": {"input": {"required": {}}},
            "Hy3DExportMesh": {"input": {"required": {}}}
        }))
        .unwrap();
        let installed = Installed::parse(&object_info).unwrap();
        assert_eq!(installed.mesh_models(), ["hunyuan3d-dit-v2-1.ckpt"]);
        assert!(installed.has_class("Hy3D_2_1SimpleMeshGen"));
        assert!(installed.has_class("Hy3DPostprocessMesh"));
        assert!(installed.has_class("Hy3DExportMesh"));
    }

    #[test]
    fn an_input_declared_optional_is_still_an_input() {
        // Custom packs move fields between `required` and `optional` between
        // releases, and a probe that only read one would report a model list
        // that shrank for no reason the user could see.
        let optional = serde_json::to_vec(&json!({
            "LoraLoader": {"input": {"optional": {"lora_name": [["x.safetensors"], {}]}}},
        }))
        .unwrap();
        assert_eq!(Installed::parse(&optional).unwrap().loras(), ["x.safetensors"]);
    }

    #[test]
    fn one_unreadable_node_does_not_cost_the_whole_probe() {
        // This document describes every pack on the machine, several of them
        // written by strangers. A single node with a shape nobody anticipated
        // must leave the rest of the answer intact, or one bad plugin means
        // wobu reports an empty ComfyUI.
        let hostile = serde_json::to_vec(&json!({
            "CheckpointLoaderSimple": {"input": {"required": {"ckpt_name": [["a.ckpt"], {}]}}},
            "SomeoneElsesNode": {"input": "not an object at all"},
            "AnotherOne": {"input": {"required": {"thing": 7}}},
            "SaveImage": {},
        }))
        .unwrap();
        let installed = Installed::parse(&hostile).unwrap();
        assert_eq!(installed.checkpoints(), ["a.ckpt"]);
        assert!(installed.has_class("SomeoneElsesNode"));
        assert!(installed.has_class("SaveImage"));
    }

    #[test]
    fn the_ceiling_is_read_off_the_card_rather_than_compiled_in() {
        // The one number a local backend cannot declare in advance. A figure
        // baked into wobu is either a card nobody has or a limit that wastes the
        // one they bought, and `/system_stats` is where the real answer lives.
        let stats = serde_json::to_vec(&json!({
            "system": {"os": "posix", "comfyui_version": "0.3.68", "ram_total": 67_000_000_000u64,
                       "python_version": "3.12.3", "pytorch_version": "2.6.0",
                       "embedded_python": false, "argv": []},
            "devices": [{"name": "cuda:0 NVIDIA GeForce RTX 4090", "type": "cuda", "index": 0,
                         "vram_total": 25_390_809_088u64, "vram_free": 24_000_000_000u64,
                         "torch_vram_total": 0, "torch_vram_free": 0}],
        }))
        .unwrap();
        let server = Server::parse(&stats).unwrap();
        assert_eq!(server.version.as_deref(), Some("0.3.68"));
        assert_eq!(server.vram_bytes, Some(25_390_809_088));

        // A CPU-only install is a real configuration; it renders, slowly.
        let cpu = serde_json::to_vec(&json!({"system": {"os": "nt"}, "devices": []})).unwrap();
        assert_eq!(Server::parse(&cpu).unwrap().vram_bytes, None);
    }

    #[test]
    fn something_else_answering_on_the_port_is_not_a_server() {
        // 8188 is a plain HTTP port and plenty of things sit on one. A body that
        // parses but has no `system` is the wrong service, which is a different
        // message from "ComfyUI is not running" and sends the user to a
        // different place.
        assert_eq!(Server::parse(br#"{"status": "ok"}"#), None);
        assert_eq!(Server::parse(b"<html>Stable Diffusion web UI</html>"), None);
    }

    #[test]
    fn a_body_that_is_not_object_info_is_no_probe_at_all() {
        // What comes back when the port belongs to something else. `None` is
        // what lets the caller say "that is not ComfyUI" rather than "this
        // ComfyUI has no models", which send the user to two different places.
        assert_eq!(Installed::parse(b"<!DOCTYPE html>"), None);
        assert_eq!(Installed::parse(b"[1, 2, 3]"), None);
        assert_eq!(Installed::parse(b""), None);
    }
}
