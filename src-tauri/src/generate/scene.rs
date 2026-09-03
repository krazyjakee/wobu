//! Composing several subjects into one image.
//!
//! A scene has no subject node to take a preset, an aspect or a reference
//! budget from, so it substitutes its own: a wide establishing framing, a
//! prompt compiled from every participant, and a reference allowance shared out
//! so that no participant is squeezed to nothing by the ones before it.
//! Everything after that is the [`super::task::PlannedBatch`] a batch produces.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use serde_json::{Value, json};
use tauri::{AppHandle, State};
use wobu_core::{
    Asset, AssetRole, FragmentTarget, Generation, Id, Node, SceneComposition, kind_def, new_id,
};
use wobu_imagine::{AspectRatio, ImageBackend, ImageRequest, negotiate_scene};
use wobu_influence::{
    Fragment, FragmentBody, ResolvedScene, SceneScope, Shot, World, resolve_scene, scene_fragments,
};

use super::loras::{resolve_loras, scene_prompt_with_lora_triggers};
use super::plan::{
    FragmentKey, SeedIntent, SeedSource, normalize_aspect, normalize_prompt, normalize_weight,
    resolve_seed, snapshot,
};
use super::task::{GenerateTask, PlannedBatch, PlannedImage};
use super::{
    BackendPurpose, PlanningInputs, ReceiptPreparation, ReferenceLoader, ReferenceScope,
    image_backend, load_references, planning_inputs, prepare_blocking,
};
use crate::error::{Code, CommandResult, WobuError};
use crate::keys::Keys;
use crate::machine::MachineSettings;
use crate::state::{AppState, Jobs};

/// Composition has no preset of its own to take a default aspect from, and the
/// `environment_matte` framing its receipt records is a wide establishing shot.
const SCENE_ASPECT: &str = "16:9";

/// Queue one image containing two to four ordered world entities.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri exposes these as named bridge arguments.
pub async fn scene_generate_start(
    app: AppHandle,
    state: State<'_, AppState>,
    jobs: State<'_, Jobs>,
    keys: State<'_, Keys>,
    machine: State<'_, MachineSettings>,
    subject_ids: Vec<Id>,
    prompt: Option<String>,
    aspect: Option<String>,
    model: Option<String>,
    seed: Option<u64>,
) -> CommandResult<String> {
    if !(2..=4).contains(&subject_ids.len()) {
        return Err(WobuError::new(Code::Invalid, "A scene needs two to four entities."));
    }
    let distinct: HashSet<Id> = subject_ids.iter().copied().collect();
    if distinct.len() != subject_ids.len() {
        return Err(WobuError::new(Code::Invalid, "A scene cannot contain the same entity twice."));
    }

    let read_only = "This project is read-only, so a generated scene could not be saved.";
    let PlanningInputs { root, project_id, nodes, assets, selection } =
        planning_inputs(&state, read_only)?;

    for subject_id in &subject_ids {
        let subject = nodes.iter().find(|node| node.id == *subject_id).ok_or_else(|| {
            WobuError::new(Code::NoSuchNode, "A scene entity is not in this project any more.")
                .with_detail(subject_id.to_string())
        })?;
        if kind_def(subject.kind).singleton {
            return Err(WobuError::new(
                Code::Invalid,
                "Style Guides and World Bibles are scene context, not scene entities.",
            ));
        }
    }

    let backend =
        image_backend(&selection.provider, &machine, BackendPurpose::Generate(&keys)).await?;
    let model = selection.model(model, backend.default_model());
    // A composition has no locked seed of its own: its participants may each
    // have one and they may disagree, so there is nothing to inherit.
    let (seed, seed_source) = resolve_seed(seed, None, SeedIntent::Execute);
    let plan = prepare_blocking("Scene preparation stopped unexpectedly.", move || {
        prepare_scene(ScenePrepare {
            root,
            project_id,
            nodes,
            assets,
            subject_ids,
            prompt: prompt.unwrap_or_default(),
            aspect,
            model,
            seed,
            seed_source,
            backend,
            provider: selection.provider,
            app,
        })
    })
    .await?;
    let id = jobs.queue().submit(plan);
    Ok(id.to_string())
}

pub(super) struct ScenePrepare {
    pub(super) root: PathBuf,
    pub(super) project_id: Id,
    pub(super) nodes: Vec<Node>,
    pub(super) assets: Vec<Asset>,
    /// The full ordered participant set is carried to every composition step.
    /// Participant LoRA collection belongs here too: #69 can collect compatible
    /// pins for every id and dedupe by hash without changing this request seam.
    pub(super) subject_ids: Vec<Id>,
    pub(super) prompt: String,
    pub(super) aspect: Option<String>,
    pub(super) model: String,
    pub(super) seed: u64,
    pub(super) seed_source: SeedSource,
    pub(super) backend: Arc<dyn ImageBackend>,
    pub(super) provider: String,
    pub(super) app: AppHandle,
}

/// Composition's half of the shared planning seam. Same shape and same rules as
/// [`BatchPlan`]: borrowed project state in, immutable [`PlannedBatch`] out, no
/// `AppHandle` and no queue.
pub(super) struct ScenePlan<'a> {
    pub(super) root: &'a Path,
    pub(super) nodes: &'a [Node],
    pub(super) assets: &'a [Asset],
    pub(super) subject_ids: &'a [Id],
    pub(super) prompt: &'a str,
    pub(super) aspect: Option<&'a str>,
    pub(super) model: &'a str,
    pub(super) provider: &'a str,
    pub(super) seed: u64,
    pub(super) seed_source: SeedSource,
    pub(super) backend: &'a dyn ImageBackend,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct NormalizedSceneControls {
    pub(super) prompt: String,
    pub(super) aspect: AspectRatio,
    pub(super) shot_weight: f32,
}

pub(super) fn normalize_scene_controls(
    prompt: &str,
    aspect: Option<&str>,
) -> CommandResult<NormalizedSceneControls> {
    Ok(NormalizedSceneControls {
        prompt: normalize_prompt(Some(prompt)),
        aspect: normalize_aspect(aspect, SCENE_ASPECT)?,
        shot_weight: normalize_weight(None),
    })
}

impl NormalizedSceneControls {
    /// `params.controls` for a composition.
    ///
    /// Deliberately a different shape from the single-subject one: a scene has
    /// no per-layer sliders and no Shot label to re-apply, and History reads the
    /// `scene` key to know which of the two it is looking at.
    fn receipt_controls(&self) -> Value {
        json!({
            "scene": {
                "prompt": self.prompt,
                "aspect": self.aspect.to_string(),
            },
        })
    }
}

fn prepare_scene(input: ScenePrepare) -> CommandResult<GenerateTask> {
    let planned = plan_scene(ScenePlan {
        root: &input.root,
        nodes: &input.nodes,
        assets: &input.assets,
        subject_ids: &input.subject_ids,
        prompt: &input.prompt,
        aspect: input.aspect.as_deref(),
        model: &input.model,
        provider: &input.provider,
        seed: input.seed,
        seed_source: input.seed_source,
        backend: input.backend.as_ref(),
    })?;
    Ok(planned.into_task(input.root, input.project_id, input.backend, input.app))
}

pub(super) fn plan_scene(input: ScenePlan<'_>) -> CommandResult<PlannedBatch> {
    let controls = normalize_scene_controls(input.prompt, input.aspect)?;
    let world = World::new(input.nodes.iter());
    let names: Vec<String> = input
        .subject_ids
        .iter()
        .map(|id| {
            world
                .get(*id)
                .map(|node| node.name.clone())
                .ok_or_else(|| WobuError::new(Code::NoSuchNode, "A scene entity disappeared."))
        })
        .collect::<Result<_, _>>()?;
    let scene_label = format!("Scene · {}", names.join(" + "));
    let scene = resolve_scene(
        &world,
        input.subject_ids,
        Shot { label: &scene_label, weight: controls.shot_weight },
    )
    .map_err(|error| {
        WobuError::new(Code::Invalid, "The scene influence stacks could not be composed.")
            .with_detail(format!("{error:?}"))
    })?;
    let mut extracted = scene_fragments(&world, &scene);
    append_scene_prompt(&scene, &mut extracted, &controls.prompt);
    let requested_aspect = controls.aspect;
    let caps = input.backend.capabilities(input.model);
    let negotiated = negotiate_scene(&extracted, requested_aspect, &caps, input.subject_ids);
    ensure_scene_reference_fairness(input.subject_ids, &extracted, &negotiated)?;
    let (prompt, negative) = compile_scene_prompt(&scene, negotiated.fragments(), &names);
    let loras = resolve_loras(
        input.root,
        input.nodes,
        scene.stack().sources().iter().filter_map(|source| source.node_id()),
        input.model,
        input.backend,
    );
    let prompt = scene_prompt_with_lora_triggers(&prompt, &loras.weights);
    if prompt.trim().is_empty() {
        return Err(WobuError::new(
            Code::Invalid,
            "Describe at least one scene entity before generating.",
        ));
    }

    let assets: HashMap<Id, &Asset> = input.assets.iter().map(|asset| (asset.id, asset)).collect();
    let mut reference_loader = ReferenceLoader::new();
    let references = load_references(
        &negotiated,
        &assets,
        input.root,
        &mut reference_loader,
        ReferenceScope::Scene,
    )?;
    let dropped: Vec<FragmentKey> = negotiated
        .images()
        .dropped()
        .map(|drop| FragmentKey::of(drop.fragment))
        .chain(negotiated.downgrades().iter().map(|drop| FragmentKey::of(drop.fragment)))
        .collect();
    let resolution = negotiated.resolution();
    let reference_asset_ids =
        references.iter().map(|reference| reference.asset_id).collect::<Vec<_>>();
    let mut params = ReceiptPreparation {
        batch_index: 0,
        batch_size: 1,
        requested_aspect,
        actual_aspect: negotiated.aspect(),
        resolution,
        negative_prompt_supported: caps.negative_prompt,
        seed_source: input.seed_source,
        reference_asset_ids: &reference_asset_ids,
        loras: &loras.receipts,
        lora_downgrades: &loras.downgrades,
        controls: controls.receipt_controls(),
    }
    .params();
    params.insert(
        "sceneComposition".into(),
        serde_json::to_value(SceneComposition {
            version: 1,
            subject_ids: input.subject_ids.to_vec(),
            subject_names: names.clone(),
        })
        .map_err(|error| {
            WobuError::new(Code::Internal, "The scene receipt could not be encoded.")
                .with_detail(error.to_string())
        })?,
    );
    let request = ImageRequest::new(input.model.to_owned(), &prompt, input.seed, &negotiated)
        .with_negative(&negative)
        .with_references(references)
        .with_loras(loras.weights);
    let primary = input.subject_ids[0];
    let generation = Generation {
        id: new_id(),
        node_id: primary,
        created_at: Utc::now(),
        // Scene composition deliberately uses the registry's wide establishing
        // preset for framing/aspect. `params.sceneComposition` is the mode and
        // participant record; inventing a preset id no registry knows would
        // make history and replay disagree about which framing was chosen.
        preset: "environment_matte".into(),
        view_type: None,
        user_prompt: controls.prompt.clone(),
        compiled_prompt: prompt,
        negative_prompt: negative,
        backend: input.provider.to_owned(),
        model: input.model.to_owned(),
        seed: input.seed,
        params,
        output_asset_ids: Vec::new(),
        influence_snapshot: snapshot(scene.stack(), &extracted, &[], &HashSet::new(), &dropped),
    };
    Ok(PlannedBatch {
        label: format!("Compose scene · {}", names.join(" + ")),
        subject_id: primary,
        plans: vec![PlannedImage { request, generation }],
        requires_billing: caps.requires_billing,
        archival_replay: false,
    })
}

fn append_scene_prompt<'a>(
    scene: &ResolvedScene<'a>,
    fragments: &mut Vec<Fragment<'a>>,
    prompt: &'a str,
) {
    if prompt.is_empty() {
        return;
    }
    if let Some(source) =
        scene.stack().sources().iter().find(|source| source.layer == wobu_core::Layer::Shot)
    {
        let fragment = Fragment::new(
            source,
            "user_prompt",
            FragmentBody::Text(prompt),
            source.weight,
            FragmentTarget::Prompt,
        );
        let identity = fragments
            .iter()
            .position(|fragment| fragment.section() == "scene_identity")
            .unwrap_or(fragments.len());
        fragments.insert(identity, fragment);
    }
}

fn compile_scene_prompt(
    scene: &ResolvedScene<'_>,
    fragments: &[Fragment<'_>],
    names: &[String],
) -> (String, String) {
    let mut shared = Vec::new();
    let mut by_subject: HashMap<Id, Vec<&str>> =
        scene.subjects().iter().copied().map(|id| (id, Vec::new())).collect();
    let mut shot = Vec::new();
    let mut negatives = Vec::new();
    for fragment in fragments.iter().copied().filter(|fragment| fragment.contributes()) {
        let Some(text) = fragment.text() else { continue };
        match fragment.target() {
            FragmentTarget::Negative => push_unique(&mut negatives, text),
            FragmentTarget::Prompt => match fragment.node_id() {
                None => push_unique(&mut shot, text),
                Some(id) => match scene.scope_for_node(id) {
                    Some(SceneScope::Subject(subject)) => {
                        if let Some(values) = by_subject.get_mut(&subject) {
                            push_unique(values, text);
                        }
                    }
                    Some(SceneScope::Shared) | None => push_unique(&mut shared, text),
                },
            },
            FragmentTarget::StyleRef
            | FragmentTarget::StructureRef
            | FragmentTarget::Palette
            | FragmentTarget::MoodboardOnly => {}
        }
    }

    let mut clauses = Vec::new();
    if !shared.is_empty() {
        clauses.push(format!("Shared world and style: {}", shared.join(", ")));
    }
    for (subject, name) in scene.subjects().iter().zip(names) {
        if let Some(values) = by_subject.get(subject)
            && !values.is_empty()
        {
            clauses.push(format!("{name}: {}", values.join(", ")));
        }
    }
    clauses.extend(shot.into_iter().map(str::to_owned));
    (clauses.join("; "), negatives.join(", "))
}

fn push_unique<'a>(values: &mut Vec<&'a str>, value: &'a str) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn ensure_scene_reference_fairness(
    subjects: &[Id],
    offered: &[Fragment<'_>],
    negotiated: &wobu_imagine::Negotiated<'_>,
) -> CommandResult<()> {
    let kept: HashSet<(Id, AssetRole)> = negotiated
        .images()
        .kept()
        .filter_map(|fragment| Some((fragment.asset_id()?, fragment.asset_role()?)))
        .collect();
    for subject in subjects {
        let direct: Vec<(Id, AssetRole)> = offered
            .iter()
            .copied()
            .filter(|fragment| {
                fragment.node_id() == Some(*subject)
                    && fragment.contributes()
                    && fragment.is_sendable()
            })
            .filter_map(|fragment| Some((fragment.asset_id()?, fragment.asset_role()?)))
            .collect();
        if !direct.is_empty() && !direct.iter().any(|reference| kept.contains(reference)) {
            return Err(WobuError::new(
                Code::Invalid,
                "The selected image model cannot keep one identity reference for every scene entity.",
            )
            .with_detail(format!("no reference slot remained for {subject}")));
        }
    }
    Ok(())
}
