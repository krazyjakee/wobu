//! Workflow templates, and patching one by node id.
//!
//! A ComfyUI workflow is a graph: a map from node id to `{class_type, inputs}`,
//! where an input is either a literal or a `[node_id, slot]` edge to another
//! node's output. `/prompt` takes that map. So a template is a graph with the
//! literals we are about to replace left at their defaults, and applying a
//! request means *setting fields in a parsed structure*.
//!
//! ## Why this is not find-and-replace on the serialised JSON
//!
//! The obvious implementation — ship the template as a string with `{{PROMPT}}`
//! in it and substitute — breaks on inputs nobody controls, and every one of
//! these is a real prompt someone will write:
//!
//! - A prompt containing `"` or `\` produces a document that is no longer JSON.
//!   The failure arrives as a 400 from a server that will not say which brace.
//! - A prompt containing the placeholder's own delimiters substitutes twice.
//! - `sd_checkpoint.json` has **two** `CLIPTextEncode` nodes whose `text` is the
//!   empty string, one positive and one negative. Any replace keyed on the value
//!   rather than on the node hits both, and the user gets a picture of the
//!   things they asked never to see. That is the regression
//!   `the_positive_and_negative_prompts_land_in_different_nodes` guards.
//! - A checkpoint filename that happens to contain a node id — and they are
//!   short strings like `"3"` — corrupts an edge.
//!
//! Patching by id has none of those failure modes, because the prompt never
//! passes through a serialiser that could confuse it with syntax: it is a
//! `Value::String` set at a key, and `serde_json` escapes it on the way out.
//!
//! ## Why a `Value` and not a typed graph
//!
//! Every node in a template is one this file knows about, so a struct would fit.
//! But a template is data a user may eventually supply — the ComfyUI UI exports
//! exactly this format — and a graph containing a node from a custom pack would
//! then lose whatever fields the struct could not name. Round-tripping through
//! `Value` keeps unknown nodes byte-identical, and the only fields this file
//! touches are the ones a [`Binding`] points at.

use serde_json::{Map, Value};

use crate::backend::ImageRequest;
use crate::error::{Error, Result};

/// One value a request has to reach inside a graph.
///
/// Not a string key: a typo in a slot name would be a template that silently
/// renders at the default resolution, which is a plausible-looking wrong picture
/// rather than a failure. The set is closed and the compiler checks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    /// The checkpoint or diffusion-model filename. This is `ImageRequest::model`,
    /// which `backend.rs` documents as backend-specific and opaque.
    Model,
    Positive,
    /// Absent from templates for models that have no negative prompt, which is
    /// what makes [`Capabilities::negative_prompt`](crate::Capabilities) a
    /// per-model answer rather than a per-backend one — Flux's guidance chain
    /// has nowhere to put one.
    Negative,
    Width,
    Height,
    Seed,
}

/// Where one [`Slot`] lives: a node id and an input name on that node.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Binding {
    pub(crate) slot: Slot,
    /// The key in the graph, exactly as the template spells it. A binding whose
    /// node is not in the graph is a broken template and fails loudly, because a
    /// binding that quietly did nothing is the silent-default failure above.
    pub(crate) node: &'static str,
    pub(crate) input: &'static str,
}

const fn bind(slot: Slot, node: &'static str, input: &'static str) -> Binding {
    Binding { slot, node, input }
}

/// A shipped graph plus the map from our vocabulary onto its node ids.
///
/// The two are declared together and next to each other on purpose: the JSON on
/// its own says nothing about which of two identical `CLIPTextEncode` nodes is
/// the negative one, and a binding on its own points at nothing. Splitting them
/// across files would let one be edited without the other, and the way that
/// fails is a prompt in the wrong node.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Workflow {
    /// Stable, and recorded in `Generation.params` so an old generation can say
    /// which graph produced it.
    pub(crate) id: &'static str,
    /// Which shape of model this graph drives. Selection is by what the server
    /// says it has — see [`Workflow::for_family`] — and never by parsing the
    /// filename, because `flux1-dev.safetensors` is a full checkpoint on one
    /// machine and a bare diffusion model on the next.
    pub(crate) family: Family,
    graph: &'static str,
    bindings: &'static [Binding],
    /// The `SaveImage` node whose `executed` message carries the filename. Named
    /// rather than discovered, because a graph may legitimately have several
    /// output nodes and the one we want is the one this template put there.
    pub(crate) output: &'static str,
}

/// Which loader chain a model needs.
///
/// Not a guess about quality or a marketing name — it is the answer to "which
/// node loads this file", which is the only thing the graph cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    /// Anything `CheckpointLoaderSimple` can open: SD 1.x, SD 2.x, SDXL, and the
    /// all-in-one Flux checkpoints.
    Checkpoint,
    /// A bare diffusion model under `models/unet`, with CLIP and VAE loaded
    /// separately. The usual shape of a Flux install.
    Unet,
}

impl Family {
    /// Every family, so `every_family_has_a_workflow_of_its_own` can check the
    /// table covers all of them. That test is what makes the fallback in
    /// [`Workflow::for_family`] unreachable rather than a guess.
    #[cfg(test)]
    const ALL: [Family; 2] = [Family::Checkpoint, Family::Unet];
}

/// txt2img through `CheckpointLoaderSimple`, with a negative prompt.
const SD_CHECKPOINT: Workflow = Workflow {
    id: "sd_checkpoint",
    family: Family::Checkpoint,
    graph: include_str!("templates/sd_checkpoint.json"),
    bindings: &[
        bind(Slot::Model, "4", "ckpt_name"),
        // Nodes 6 and 7 are byte-identical in the template. Only these two lines
        // say which is which.
        bind(Slot::Positive, "6", "text"),
        bind(Slot::Negative, "7", "text"),
        bind(Slot::Width, "5", "width"),
        bind(Slot::Height, "5", "height"),
        bind(Slot::Seed, "3", "seed"),
    ],
    output: "9",
};

/// Flux through the custom-sampler chain: no `KSampler`, no negative prompt, and
/// the seed on a node of its own.
///
/// It is here as much for what it proves as for what it renders. Every slot it
/// shares with [`SD_CHECKPOINT`] is at a different node id under a different
/// input name — the seed is `RandomNoise.noise_seed` rather than `KSampler.seed`
/// — so one `ImageRequest` filling both graphs is only possible because the
/// binding table, and not the request, knows where things go.
const FLUX_UNET: Workflow = Workflow {
    id: "flux_unet",
    family: Family::Unet,
    graph: include_str!("templates/flux_unet.json"),
    bindings: &[
        bind(Slot::Model, "10", "unet_name"),
        bind(Slot::Positive, "13", "text"),
        bind(Slot::Width, "15", "width"),
        bind(Slot::Height, "15", "height"),
        bind(Slot::Seed, "16", "noise_seed"),
    ],
    output: "22",
};

pub(crate) const WORKFLOWS: &[Workflow] = &[SD_CHECKPOINT, FLUX_UNET];

impl Workflow {
    /// The graph for a model the server has told us about.
    ///
    /// `family` comes from which list the filename was in — `CheckpointLoader`'s
    /// or `UNETLoader`'s — so this is a fact read off that ComfyUI rather than a
    /// substring match on a name.
    pub(crate) fn for_family(family: Family) -> Workflow {
        // Read out of the table rather than matched on, so a workflow added to
        // `WORKFLOWS` is reachable without a second edit somewhere else. Total
        // rather than an `Option`, because a family with no workflow would be a
        // case every caller had to invent an answer for and the answer would be
        // one of these anyway — and the table is checked to cover every family,
        // which is what makes the fallback unreachable rather than a guess.
        WORKFLOWS
            .iter()
            .copied()
            .find(|workflow| workflow.family == family)
            .unwrap_or(SD_CHECKPOINT)
    }

    /// Whether a request naming this slot can be honoured at all.
    pub(crate) fn has(&self, slot: Slot) -> bool {
        self.bindings.iter().any(|b| b.slot == slot)
    }

    /// Every distinct `class_type` this graph needs, in graph order.
    ///
    /// What the "you do not have that node installed" diagnosis is checked
    /// against, before anything is sent. A 400 from `/prompt` says the same
    /// thing, eventually, in a form nobody can act on.
    pub(crate) fn classes(&self) -> Vec<String> {
        let mut classes: Vec<String> = Vec::new();
        for (_, node) in self.parse().unwrap_or_default() {
            if let Some(class) = node.get("class_type").and_then(Value::as_str)
                && !classes.iter().any(|seen| seen == class)
            {
                classes.push(class.to_owned());
            }
        }
        classes
    }

    /// The template as a fresh graph. Parsed per call rather than cached,
    /// because [`patch`](Self::patch) mutates it and a shared copy would carry
    /// one generation's prompt into the next.
    fn parse(&self) -> Result<Map<String, Value>> {
        match serde_json::from_str::<Value>(self.graph) {
            Ok(Value::Object(map)) => Ok(map),
            // Only reachable if a shipped template was edited into something
            // that is not a graph, which is our bug in exactly the sense
            // `error.rs` reserves `Unsupported` for.
            _ => Err(Error::Unsupported {
                detail: format!("workflow template {} is not a JSON object of nodes", self.id),
            }),
        }
    }

    /// The graph this request will be sent as.
    ///
    /// Fails rather than substituting, per the trait's first contract point: a
    /// request carrying something no binding can reach has been through a
    /// negotiation that disagrees with [`ComfyBackend::capabilities`], and that
    /// is our bug and never the user's.
    ///
    /// [`ComfyBackend::capabilities`]: super::ComfyBackend
    pub(crate) fn patch(&self, request: &ImageRequest) -> Result<Map<String, Value>> {
        if !request.negative.is_empty() && !self.has(Slot::Negative) {
            return Err(Error::Unsupported {
                detail: format!(
                    "workflow {} has no negative prompt, and the request carries one",
                    self.id,
                ),
            });
        }
        if let Some(reference) = request.references.first() {
            // The shipped templates are text-to-image. A reference reaching here
            // means the declared image budget let one through, and sending the
            // request without it would be the silent drop the whole crate exists
            // to prevent.
            return Err(Error::Unsupported {
                detail: format!(
                    "workflow {} takes no reference images, and the request carries {} \
                     (first is a {} reference)",
                    self.id,
                    request.references.len(),
                    reference.role.as_str(),
                ),
            });
        }

        let mut graph = self.parse()?;
        for binding in self.bindings {
            let value = match binding.slot {
                Slot::Model => Value::from(request.model.as_str()),
                Slot::Positive => Value::from(request.prompt.as_str()),
                Slot::Negative => Value::from(request.negative.as_str()),
                Slot::Width => Value::from(request.resolution.width),
                Slot::Height => Value::from(request.resolution.height),
                Slot::Seed => Value::from(request.seed),
            };
            set(&mut graph, self.id, *binding, value)?;
        }
        Ok(graph)
    }
}

/// Write one input, or say which node and field were not there.
///
/// The `Err` arm is the whole reason this is a function. A binding that pointed
/// at a node the template does not have would otherwise insert one — `Value` is
/// a map and maps take new keys — and the graph would go out with an orphan node
/// carrying the prompt, and render at the template's defaults. That is a
/// plausible picture nothing on screen explains, which is worse than a failure.
fn set(
    graph: &mut Map<String, Value>,
    workflow: &str,
    binding: Binding,
    value: Value,
) -> Result<()> {
    let inputs = graph
        .get_mut(binding.node)
        .and_then(|node| node.get_mut("inputs"))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| Error::Unsupported {
            detail: format!(
                "workflow {workflow} binds {:?} to node {}, which has no inputs",
                binding.slot, binding.node,
            ),
        })?;
    match inputs.get_mut(binding.input) {
        Some(slot) => {
            *slot = value;
            Ok(())
        }
        None => Err(Error::Unsupported {
            detail: format!(
                "workflow {workflow} binds {:?} to {}.{}, which the template does not declare",
                binding.slot, binding.node, binding.input,
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aspect::{AspectRatio, Resolution};
    use crate::capability::Capabilities;
    use crate::negotiate::negotiate;
    use wobu_influence::{ImageBudget, Refs};

    fn caps() -> Capabilities {
        Capabilities {
            max_resolution: Resolution::new(2048, 2048),
            aspect_ratios: vec![],
            image_refs: ImageBudget {
                objects: Refs::new(0),
                characters: Some(Refs::new(0)),
                style_refs: Some(Refs::new(0)),
            },
            controlnet: false,
            loras: false,
            negative_prompt: true,
            requires_billing: false,
            streaming_preview: true,
        }
    }

    fn request(prompt: &str, negative: &str) -> ImageRequest {
        let negotiated = negotiate(&[], AspectRatio::parse("16:9").unwrap(), &caps());
        ImageRequest::new("sd_xl_base_1.0.safetensors", prompt, 424242, &negotiated)
            .with_negative(negative)
    }

    fn text_at(graph: &Map<String, Value>, node: &str, input: &str) -> String {
        graph[node]["inputs"][input].as_str().unwrap().to_owned()
    }

    #[test]
    fn every_shipped_template_parses_and_every_binding_points_at_a_real_input() {
        // The failure this guards is a template edited without its bindings: a
        // renamed node id, or an input a ComfyUI release spelled differently.
        // Both produce a graph that renders at the template's defaults, which is
        // a plausible wrong picture rather than an error, so it has to fail at
        // build time instead.
        for workflow in WORKFLOWS {
            let graph = workflow.patch(&request("a hooded figure", "")).unwrap_or_else(|e| {
                panic!("{} could not be patched: {e}", workflow.id);
            });
            assert!(graph.contains_key(workflow.output), "{} has no output node", workflow.id);
            assert!(workflow.has(Slot::Model), "{} cannot be told which model", workflow.id);
            assert!(workflow.has(Slot::Seed), "{} cannot be seeded", workflow.id);
            assert!(!workflow.classes().is_empty());
        }
    }

    #[test]
    fn the_positive_and_negative_prompts_land_in_different_nodes() {
        // `sd_checkpoint.json` has two `CLIPTextEncode` nodes whose `text` is the
        // empty string. Any substitution keyed on the value rather than on the
        // node id fills both, and the user gets a picture of the things they
        // listed under `never:`.
        let graph = SD_CHECKPOINT
            .patch(&request("a hooded figure in ash-glazed plate", "modern firearms"))
            .unwrap();
        assert_eq!(text_at(&graph, "6", "text"), "a hooded figure in ash-glazed plate");
        assert_eq!(text_at(&graph, "7", "text"), "modern firearms");
    }

    #[test]
    fn a_prompt_full_of_json_syntax_survives_being_patched_in() {
        // The regression that makes this patch-by-id rather than replace-in-text.
        // Braces, quotes and backslashes are ordinary things to write about a
        // costume, and through a string substitution each of them produces a
        // document ComfyUI answers with a 400 that names no field.
        let hostile = r#"a sigil like {"3"} — "ash" \ ember, 100% {{PROMPT}}"#;
        let graph = SD_CHECKPOINT.patch(&request(hostile, r#"{"7": null}"#)).unwrap();
        assert_eq!(text_at(&graph, "6", "text"), hostile);
        assert_eq!(text_at(&graph, "7", "text"), r#"{"7": null}"#);

        // And the graph is still a graph: the edges the prompt impersonates are
        // untouched, and it round-trips through a serialiser.
        assert_eq!(graph["3"]["inputs"]["positive"], serde_json::json!(["6", 0]));
        let round_tripped: Map<String, Value> =
            serde_json::from_slice(&serde_json::to_vec(&graph).unwrap()).unwrap();
        assert_eq!(round_tripped, graph);
    }

    #[test]
    fn a_model_filename_that_looks_like_a_node_id_does_not_corrupt_an_edge() {
        // Node ids are short strings like "3", and checkpoint filenames are
        // whatever somebody named a download. A substitution over serialised
        // JSON cannot tell the two apart.
        let mut request = request("p", "");
        request.model = "3".into();
        let graph = SD_CHECKPOINT.patch(&request).unwrap();
        assert_eq!(text_at(&graph, "4", "ckpt_name"), "3");
        assert_eq!(graph["3"]["inputs"]["latent_image"], serde_json::json!(["5", 0]));
        assert_eq!(graph["8"]["inputs"]["vae"], serde_json::json!(["4", 2]));
    }

    #[test]
    fn one_request_fills_two_graphs_that_keep_the_same_values_in_different_places() {
        // The claim the second template exists to check. Flux's seed is
        // `RandomNoise.noise_seed` and SD's is `KSampler.seed`; the dimensions
        // are on `EmptySD3LatentImage` rather than `EmptyLatentImage`. Nothing in
        // `ImageRequest` knows any of that, which is the point.
        let request = request("a hooded figure", "");
        let sd = SD_CHECKPOINT.patch(&request).unwrap();
        let flux = FLUX_UNET.patch(&request).unwrap();

        assert_eq!(sd["3"]["inputs"]["seed"], 424242);
        assert_eq!(flux["16"]["inputs"]["noise_seed"], 424242);
        assert_eq!(sd["5"]["inputs"]["width"], 2048);
        assert_eq!(flux["15"]["inputs"]["width"], 2048);
        assert_eq!(text_at(&sd, "6", "text"), "a hooded figure");
        assert_eq!(text_at(&flux, "13", "text"), "a hooded figure");
        assert_eq!(text_at(&sd, "4", "ckpt_name"), request.model);
        assert_eq!(text_at(&flux, "10", "unet_name"), request.model);
    }

    #[test]
    fn a_negative_prompt_sent_to_a_graph_that_has_none_is_reported_as_our_bug() {
        // Flux's guidance chain has no negative conditioning, so the adapter
        // declares `negative_prompt: false` for a Unet model and `negotiate`
        // never compiles one. Reaching here means those two disagree, which
        // `error.rs` says is `internal` rather than something to retry.
        assert!(!FLUX_UNET.has(Slot::Negative));
        let error = FLUX_UNET.patch(&request("p", "modern firearms")).unwrap_err();
        assert_eq!(error.code(), "internal");
        assert!(!error.is_retryable());

        // And an empty negative is not a negative, so the same graph takes the
        // same request once the negotiation has done its job.
        assert!(FLUX_UNET.patch(&request("p", "")).is_ok());
    }

    #[test]
    fn a_binding_that_points_at_nothing_fails_instead_of_inserting_a_node() {
        // `Value::Object` takes new keys, so the natural implementation of a
        // patch adds an orphan node carrying the prompt and renders at the
        // template's defaults — a wrong picture with nothing to explain it.
        let mut graph = SD_CHECKPOINT.parse().unwrap();
        let before = graph.len();
        let error = set(&mut graph, "t", bind(Slot::Positive, "404", "text"), Value::from("p"))
            .unwrap_err();
        assert!(error.to_string().contains("404"), "{error}");
        assert_eq!(graph.len(), before, "no node was invented");

        // Same for an input the node does not declare, which is how a ComfyUI
        // release that renames a field shows up.
        assert!(
            set(&mut graph, "t", bind(Slot::Width, "5", "wdith"), Value::from(1)).is_err(),
            "a misspelled input must not be added alongside the real one",
        );
    }

    #[test]
    fn patching_never_reuses_a_graph_between_generations() {
        // The templates are `&'static str` and the bindings are shared. A cached
        // parsed graph would carry one node's prompt into the next node's render
        // — and a turnaround is eight of these in a row.
        let first = SD_CHECKPOINT.patch(&request("first", "")).unwrap();
        let second = SD_CHECKPOINT.patch(&request("second", "")).unwrap();
        assert_eq!(text_at(&first, "6", "text"), "first");
        assert_eq!(text_at(&second, "6", "text"), "second");
    }

    #[test]
    fn a_reference_image_is_refused_rather_than_quietly_left_out() {
        // The shipped graphs are text-to-image, so the adapter declares a
        // reference budget of zero and `negotiate` reports every attached
        // picture as dropped. If one arrives anyway the honest answer is a bug
        // report, not a render that ignored it.
        let mut request = request("p", "");
        request.references = vec![crate::backend::Reference {
            asset_id: wobu_core::Id::nil(),
            role: wobu_core::AssetRole::Costume,
            bucket: wobu_influence::RefBucket::StyleRefs,
            weight: 1.0,
            bytes: vec![],
            mime: "image/png".into(),
        }];
        let error = SD_CHECKPOINT.patch(&request).unwrap_err();
        assert_eq!(error.code(), "internal");
        assert!(error.to_string().contains("costume"), "{error}");
    }

    #[test]
    fn a_workflow_lists_the_classes_its_diagnosis_will_be_checked_against() {
        // `classes` is what turns "ComfyUI returned 400" into "you do not have
        // that node installed", so a template whose classes could not be read
        // would silently lose the better message.
        let classes = SD_CHECKPOINT.classes();
        assert!(classes.contains(&"CheckpointLoaderSimple".to_string()));
        assert!(classes.contains(&"SaveImage".to_string()));
        assert_eq!(
            classes.iter().filter(|c| *c == "CLIPTextEncode").count(),
            1,
            "the two text nodes are one class, and the check is per class",
        );
        assert!(FLUX_UNET.classes().contains(&"SamplerCustomAdvanced".to_string()));
    }

    #[test]
    fn every_family_has_a_workflow_of_its_own() {
        // `for_family` falls back rather than returning an `Option`, so this is
        // what stops the fallback from being a guess: a family added without a
        // graph would otherwise render through the checkpoint loader, which
        // cannot open the file, and the failure would arrive from ComfyUI.
        for family in Family::ALL {
            assert_eq!(Workflow::for_family(family).family, family, "{family:?}");
        }
        assert_eq!(Workflow::for_family(Family::Checkpoint).id, "sd_checkpoint");
        assert_eq!(Workflow::for_family(Family::Unet).id, "flux_unet");
    }
}
