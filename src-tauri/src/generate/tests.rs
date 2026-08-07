//! The planning chain end to end, and the spend ledger under contention.
//!
//! One harness for the whole of `generate` rather than one per submodule: what
//! these assert is that the four surfaces in [`super`] agree, which is a claim
//! about the seam between the modules and cannot be made from inside one.

use std::time::Duration;

use chrono::Utc;
use wobu_core::{
    AssetKind, AssetRef, AssetRole, Description, FragmentTarget, Generation, InfluenceSnapshot,
    SectionValue, SnapshotFragment, SnapshotLayer, VariationValue, new_id, preset,
};
use wobu_imagine::{LoraWeight, negotiate};

use super::batch::{BatchPlan, plan_batch};
use super::loras::{prompt_with_lora_triggers, scene_prompt_with_lora_triggers};
use super::plan::{
    GenerateShot, GenerateSlider, GenerationPlanRequest, MAX_GRID_CELLS, SeedSource, VariantGrid,
    fragments_for_cell, prepare_generation_plan, resolve_generation_stack, variant_cells,
};
use super::preview::{ImageReferenceReport, aspect_capability_view, reference_report_for_plan};
use super::replay::replay_plan;
use super::scene::{ScenePlan, normalize_scene_controls, plan_scene};
use super::spend::{
    Price, SPEND_AGGREGATE, SPEND_DIR, SpendReservation, cost_estimate, cost_estimate_prices,
    image_price, read_cached_spend_status_locked_with, receipt_cost, spend_status_for,
    spend_status_for_report,
};
use super::task::PlannedBatch;
use super::*;

struct TestProject {
    parent: PathBuf,
    root: PathBuf,
    node_id: Id,
}

impl TestProject {
    fn new(ceiling_usd_micros: u64) -> TestProject {
        let parent = std::env::temp_dir().join(format!("wobu-spend-test-{}", new_id()));
        std::fs::create_dir(&parent).unwrap();
        let mut project = Project::create(&parent, "Ledger").unwrap();
        project.set_spend_ceiling(Some(ceiling_usd_micros)).unwrap();
        let node_id = project.create_node(wobu_core::NodeKind::Character, "Kael", None).unwrap().id;
        let root = project.root().to_path_buf();
        drop(project);
        TestProject { parent, root, node_id }
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        // The parent is a unique path minted by this test, never a caller
        // supplied directory or a workspace root.
        let _ = std::fs::remove_dir_all(&self.parent);
    }
}

/* ── the planning fixture ─────────────────────────────────────────────── */

/// The model every planning test below prices against. Pinned rather than
/// defaulted: a default that moved would silently rewrite every expected
/// cost in this file.
const MODEL: &str = "gemini-3.1-flash-image";

/// A world with files in it: a house style, two characters, and one
/// reference image each on disk.
///
/// Planning reads reference bytes, so the images have to exist; everything
/// else is in memory, because the point of the seam under test is that
/// planning takes nodes and assets rather than an open project.
struct PlanWorld {
    parent: PathBuf,
    root: PathBuf,
    nodes: Vec<Node>,
    assets: Vec<Asset>,
    kael: Id,
    rell: Id,
    kael_costume: Id,
}

impl PlanWorld {
    fn new() -> PlanWorld {
        let parent = std::env::temp_dir().join(format!("wobu-plan-test-{}", new_id()));
        let root = parent.join("world");
        std::fs::create_dir_all(root.join("assets/img")).unwrap();

        let mut assets = Vec::new();
        let mut attach = |owner: &mut Node, name: &str, role: AssetRole| {
            let id = new_id();
            let rel_path = format!("assets/img/{name}.png");
            std::fs::write(root.join(&rel_path), name.as_bytes()).unwrap();
            owner.asset_links.push(AssetRef::new(id, role));
            assets.push(Asset {
                id,
                hash: format!("hash-{name}"),
                kind: AssetKind::Reference,
                rel_path,
                thumb_path: None,
                mime: "image/png".into(),
                width: 1_024,
                height: 1_024,
                bytes: name.len() as u64,
                created_at: Utc::now(),
            });
            id
        };

        let mut style = Node::new(wobu_core::NodeKind::StyleGuide, "Ashfall House Style")
            .expect("fixture names are sluggable");
        describe(&mut style, [("medium", prose("Oil on board"))]);

        let mut kael = Node::new(wobu_core::NodeKind::Character, "Kael Vantris")
            .expect("fixture names are sluggable");
        describe(
            &mut kael,
            [
                ("silhouette", prose("Tall, narrow, hooded")),
                ("costume", prose("Ash-grey longcoat")),
                ("never", list(&["modern firearms"])),
            ],
        );
        let kael_costume = attach(&mut kael, "kael-costume", AssetRole::Costume);

        let mut rell = Node::new(wobu_core::NodeKind::Character, "Rell Sarn")
            .expect("fixture names are sluggable");
        describe(&mut rell, [("silhouette", prose("Short, broad, plated"))]);
        attach(&mut rell, "rell-costume", AssetRole::Costume);

        let (kael_id, rell_id) = (kael.id, rell.id);
        PlanWorld {
            parent,
            root,
            nodes: vec![style, kael, rell],
            assets,
            kael: kael_id,
            rell: rell_id,
            kael_costume,
        }
    }

    fn backend() -> GeminiBackend {
        GeminiBackend::new("test-key").expect("the placeholder key is well formed")
    }

    fn request(&self, preset_id: &str, seed: u64) -> GenerationPlanRequest {
        GenerationPlanRequest {
            subject_id: self.kael,
            preset_id: Some(preset_id.to_owned()),
            sliders: Vec::new(),
            shot: GenerateShot::default(),
            aspect: None,
            seed,
            seed_source: SeedSource::Random,
            locked_seed: None,
            grid: None,
        }
    }

    fn plan(&self, request: GenerationPlanRequest) -> CommandResult<PlannedBatch> {
        let backend = PlanWorld::backend();
        plan_batch(BatchPlan {
            root: &self.root,
            nodes: &self.nodes,
            assets: &self.assets,
            request,
            model: MODEL,
            provider: gemini::ID,
            backend: &backend,
        })
    }

    fn compose(&self, prompt: &str, aspect: Option<&str>) -> CommandResult<PlannedBatch> {
        let backend = PlanWorld::backend();
        plan_scene(ScenePlan {
            root: &self.root,
            nodes: &self.nodes,
            assets: &self.assets,
            subject_ids: &[self.kael, self.rell],
            prompt,
            aspect,
            model: MODEL,
            provider: gemini::ID,
            seed: 7,
            seed_source: SeedSource::Random,
            backend: &backend,
        })
    }

    fn preview(&self, request: GenerationPlanRequest) -> CommandResult<ImageReferenceReport> {
        let backend = PlanWorld::backend();
        reference_report_for_plan(&self.nodes, gemini::ID, MODEL, &backend, request)
    }
}

impl Drop for PlanWorld {
    fn drop(&mut self) {
        // The parent is a unique path minted by this test, never a caller
        // supplied directory or a workspace root.
        let _ = std::fs::remove_dir_all(&self.parent);
    }
}

fn describe(node: &mut Node, sections: impl IntoIterator<Item = (&'static str, SectionValue)>) {
    node.description = Some(Description::from_sections(
        sections.into_iter().map(|(key, value)| (key.to_owned(), value)),
    ));
}

fn prose(value: &str) -> SectionValue {
    SectionValue::Text(value.to_owned())
}

fn list(items: &[&str]) -> SectionValue {
    SectionValue::List(items.iter().map(|item| (*item).to_owned()).collect())
}

fn keys(params: &Map<String, Value>) -> Vec<&str> {
    params.keys().map(String::as_str).collect()
}

fn receipt(node_id: Id, backend: &str, model: &str, width: u32, height: u32) -> Generation {
    Generation {
        id: new_id(),
        node_id,
        created_at: Utc::now(),
        preset: "portrait".into(),
        view_type: None,
        user_prompt: String::new(),
        compiled_prompt: "portrait".into(),
        negative_prompt: String::new(),
        backend: backend.into(),
        model: model.into(),
        seed: 1,
        params: Map::from_iter([
            ("aspect".into(), json!("1:1")),
            ("width".into(), json!(width)),
            ("height".into(), json!(height)),
        ]),
        output_asset_ids: Vec::new(),
        influence_snapshot: InfluenceSnapshot { layers: Vec::new() },
    }
}

#[test]
fn sixteen_cell_batch_reads_each_reference_once_and_shares_its_buffer() {
    let reads = std::cell::Cell::new(0);
    let mut loader = ReferenceLoader::with_reader(|path| {
        reads.set(reads.get() + 1);
        Ok(path.to_string_lossy().as_bytes().to_vec())
    });
    let costume = new_id();
    let palette = new_id();

    let cells: Vec<_> = (0..MAX_GRID_CELLS)
        .map(|_| {
            [
                loader.load(costume, Path::new("costume.png")).unwrap(),
                loader.load(palette, Path::new("palette.png")).unwrap(),
            ]
        })
        .collect();

    assert_eq!(reads.get(), 2, "one filesystem read per unique asset");
    for cell in &cells[1..] {
        assert!(Arc::ptr_eq(&cells[0][0], &cell[0]));
        assert!(Arc::ptr_eq(&cells[0][1], &cell[1]));
    }
    assert!(!Arc::ptr_eq(&cells[0][0], &cells[0][1]));
    assert_eq!(Arc::strong_count(&cells[0][0]), MAX_GRID_CELLS + 1);
    assert_eq!(Arc::strong_count(&cells[0][1]), MAX_GRID_CELLS + 1);
}

#[test]
fn google_standard_output_prices_are_exact_usd_micros() {
    let cases = [
        ("gemini-3.1-flash-lite-image", 1_024, 33_600),
        ("gemini-3.1-flash-image", 512, 45_000),
        ("gemini-3.1-flash-image", 1_024, 67_000),
        ("gemini-3.1-flash-image", 2_048, 101_000),
        ("gemini-3.1-flash-image", 4_096, 151_000),
        ("gemini-3-pro-image", 1_024, 134_000),
        ("gemini-3-pro-image", 4_096, 240_000),
        ("gemini-2.5-flash-image", 1_024, 39_000),
    ];
    for (model, side, expected) in cases {
        assert_eq!(
            image_price(gemini::ID, model, Resolution::new(side, side))
                .unwrap()
                .per_image_usd_micros,
            expected,
            "{model} at {side}px"
        );
    }
}

#[test]
fn local_is_free_and_unknown_paid_models_fail_high() {
    assert!(image_price(comfy::ID, "anything", Resolution::new(4_096, 4_096)).is_none());
    let unknown =
        image_price(gemini::ID, "gemini-future-image", Resolution::new(1_024, 1_024)).unwrap();
    assert_eq!(unknown.per_image_usd_micros, 240_000);
    assert!(unknown.conservative_fallback);
}

#[test]
fn aspect_preview_exposes_ordered_choices_and_the_negotiated_substitution() {
    let mut caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
    caps.max_resolution = Resolution::new(1_024, 1_024);
    caps.aspect_ratios =
        ["1:1", "2:3"].into_iter().map(|value| AspectRatio::parse(value).unwrap()).collect();

    let preview = aspect_capability_view(gemini::ID.into(), "restricted".into(), caps);

    assert_eq!(
        preview.aspect_ratios,
        [AspectRatio::parse("1:1").unwrap(), AspectRatio::parse("2:3").unwrap()]
    );
    let portrait = preview
        .previews
        .iter()
        .find(|candidate| candidate.requested_aspect == AspectRatio::parse("3:4").unwrap())
        .unwrap();
    assert!(portrait.substituted);
    assert_eq!(portrait.actual_aspect, AspectRatio::parse("2:3").unwrap());
    assert_eq!((portrait.width, portrait.height), (682, 1_023));
}

#[test]
fn flexible_aspect_preview_uses_the_curated_validated_vocabulary() {
    let mut caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
    caps.max_resolution = Resolution::new(2_048, 2_048);
    caps.aspect_ratios.clear();

    let preview = aspect_capability_view(comfy::ID.into(), "local".into(), caps);

    assert!(preview.flexible_aspect);
    assert_eq!(preview.aspect_ratios, AspectRatio::ALL);
    assert!(preview.previews.iter().all(|candidate| !candidate.substituted));
    let square = &preview.previews[0];
    assert_eq!(square.actual_aspect, AspectRatio::parse("1:1").unwrap());
    assert_eq!((square.width, square.height), (2_048, 2_048));
}

#[test]
fn lora_triggers_are_deduplicated_and_scene_identity_stays_last() {
    let loras = vec![
        LoraWeight {
            content_hash: "a".repeat(64),
            provider_name: "first.safetensors".into(),
            trigger_token: "wobu_kael".into(),
            strength: 0.8,
        },
        LoraWeight {
            content_hash: "b".repeat(64),
            provider_name: "second.safetensors".into(),
            trigger_token: "wobu_kael".into(),
            strength: 0.7,
        },
    ];
    assert_eq!(prompt_with_lora_triggers("portrait", &loras), "portrait, wobu_kael");
    assert_eq!(prompt_with_lora_triggers("portrait of wobu_kael", &loras), "portrait of wobu_kael",);
    assert_eq!(
        scene_prompt_with_lora_triggers(
            "Shared world; wide framing; preserve every named identity",
            &loras,
        ),
        "Shared world; wide framing; wobu_kael; preserve every named identity",
    );
}

#[test]
fn batch_estimate_and_old_receipts_use_recorded_model_and_size() {
    let estimate =
        cost_estimate(gemini::ID, "gemini-3.1-flash-image", Resolution::new(2_048, 2_048), 8)
            .unwrap();
    assert_eq!(estimate.batch_usd_micros, 808_000);

    let old = receipt(new_id(), gemini::ID, "gemini-3-pro-image", 4_096, 4_096);
    assert_eq!(receipt_cost(&old), 240_000);
    let local = receipt(new_id(), comfy::ID, "local", 4_096, 4_096);
    assert_eq!(receipt_cost(&local), 0);
    let mut explicit = old;
    explicit.params.insert("estimatedCostUsdMicros".into(), json!(123_456));
    assert_eq!(receipt_cost(&explicit), 123_456);
}

#[test]
fn replay_plan_uses_recorded_request_and_current_price_without_compiling() {
    let mut original = receipt(new_id(), gemini::ID, "gemini-3.1-flash-image", 1_024, 1_024);
    original.compiled_prompt = "the immutable positive".into();
    original.negative_prompt = "the immutable negative".into();
    original.seed = 77;
    original.params.insert("estimatedCostUsdMicros".into(), json!(12_345));
    let original_id = original.id;
    let original_snapshot = original.influence_snapshot.clone();

    let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
    let backend = GeminiBackend::new("test-key").unwrap();
    let plan = replay_plan(Path::new("."), &[], original, &caps, &backend).unwrap();
    assert_eq!(plan.request.prompt, "the immutable positive");
    assert_eq!(plan.request.negative, "the immutable negative");
    assert_eq!(plan.request.seed, 77);
    assert_eq!(plan.request.resolution, Resolution::new(1_024, 1_024));
    assert_eq!(plan.generation.influence_snapshot, original_snapshot);
    assert_eq!(plan.generation.params.get("replayOf"), Some(&json!(original_id)));
    assert_eq!(
        plan.generation.params.get("replayOriginalEstimatedCostUsdMicros"),
        Some(&json!(12_345))
    );
    assert_eq!(plan.cost_usd_micros, 67_000);
    assert_eq!(plan.generation.params.get("estimatedCostUsdMicros"), Some(&json!(67_000)));
}

#[test]
fn replay_refuses_missing_snapshot_reference_instead_of_using_current_links() {
    let missing = new_id();
    let mut original = receipt(new_id(), comfy::ID, "local", 1_024, 1_024);
    original.influence_snapshot.layers.push(SnapshotLayer {
        layer: wobu_core::Layer::Subject,
        node_id: Some(original.node_id),
        node_name: "Kael".into(),
        weight: 1.0,
        muted: false,
        fragments: vec![SnapshotFragment {
            section: "pose".into(),
            text: None,
            asset_id: Some(missing),
            asset_role: Some(AssetRole::Pose),
            weight: 0.8,
            target: FragmentTarget::StructureRef,
            dropped: false,
        }],
    });
    let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
    let backend = GeminiBackend::new("test-key").unwrap();
    let Err(error) = replay_plan(Path::new("."), &[], original, &caps, &backend) else {
        panic!("a missing immutable reference must refuse replay")
    };
    assert_eq!(error.code, Code::NoSuchAsset);
    assert!(error.detail.is_some_and(|detail| detail.contains(&missing.to_string())));
}

#[test]
fn variant_cells_change_one_axis_and_report_the_real_output_count() {
    let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
    let chosen = *preset("character_sheet").unwrap();
    let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
    let available = HashSet::from([subject.id]);
    let weight_grid =
        VariantGrid::FragmentWeight { node_id: subject.id, values: vec![0.4, 0.7, 1.0] };
    let weights = variant_cells(
        &subject,
        chosen,
        AspectRatio::parse("3:4").unwrap(),
        42,
        SeedSource::Locked,
        &[],
        &available,
        Some(&weight_grid),
        &caps,
    )
    .unwrap();
    assert_eq!(weights.len(), 3);
    assert!(weights.iter().all(|cell| cell.item.seed == 42));
    assert_eq!(
        weights.iter().map(|cell| cell.slider_values[0].1).collect::<Vec<_>>(),
        [0.4, 0.7, 1.0]
    );
    assert!(weights.iter().all(|cell| {
        matches!(
            cell.variation.as_ref().map(|variation| &variation.value),
            Some(VariationValue::FragmentWeight { .. })
        )
    }));

    let seed_grid = VariantGrid::Seed { values: vec![11, 22, 33, 44, 55] };
    let seeds = variant_cells(
        &subject,
        chosen,
        AspectRatio::parse("3:4").unwrap(),
        42,
        SeedSource::Locked,
        &[],
        &available,
        Some(&seed_grid),
        &caps,
    )
    .unwrap();
    assert_eq!(seeds.iter().map(|cell| cell.item.seed).collect::<Vec<_>>(), [11, 22, 33, 44, 55]);

    let estimate = cost_estimate_prices(
        seeds
            .iter()
            .map(|_| Price { per_image_usd_micros: 67_000, conservative_fallback: false })
            .collect(),
        seeds.len(),
    )
    .unwrap();
    assert_eq!(estimate.images, 5);
    assert_eq!(estimate.batch_usd_micros, 335_000);
}

#[test]
fn preview_and_execution_prepare_the_same_normalized_plan() {
    let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
    let subject_id = subject.id;
    let nodes = vec![subject];
    let backend = GeminiBackend::new("test-key").unwrap();
    let model = "gemini-3.1-flash-image";
    let caps = backend.capabilities(model);
    let request = GenerationPlanRequest {
        subject_id,
        preset_id: Some("character_sheet".into()),
        sliders: vec![GenerateSlider { node_id: subject_id, value: 0.75, muted: false }],
        shot: GenerateShot {
            label: Some("  low angle  ".into()),
            weight: Some(1.5),
            prompt: Some("  wind catches the cloak  ".into()),
        },
        aspect: Some(" 3:4 ".into()),
        seed: 42,
        seed_source: SeedSource::Locked,
        locked_seed: Some(42),
        grid: None,
    };

    let preview =
        reference_report_for_plan(&nodes, gemini::ID, model, &backend, request.clone()).unwrap();
    let execution = prepare_generation_plan(&nodes, request, &caps).unwrap();
    assert_eq!(execution.controls.shot_label, "low angle");
    assert_eq!(execution.controls.shot_weight, 1.0);
    assert_eq!(execution.controls.user_prompt, "wind catches the cloak");
    // The per-cell copies are the authoritative ones, and for a request
    // with no grid every cell agrees with the request.
    assert_eq!(execution.cells[0].slider_values, [(subject_id, 0.75)]);
    assert_eq!(execution.cells[0].aspect, AspectRatio::parse("3:4").unwrap());

    let execution_stack = resolve_generation_stack(&nodes, &execution).unwrap();
    let execution_fragments =
        fragments_for_cell(&execution_stack, &execution.cells[0], &execution.controls.user_prompt);
    let negotiated = negotiate(&execution_fragments, execution.cells[0].aspect, &caps);
    let execution_price = image_price(gemini::ID, model, negotiated.resolution()).unwrap();
    let preview_cost = preview.cost.unwrap();
    assert_eq!(preview_cost.images, execution.cells.len());
    assert_eq!(preview_cost.per_image_usd_micros, execution_price.per_image_usd_micros);
}

#[test]
fn variant_grids_and_scene_composition_share_control_normalization() {
    let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
    let subject_id = subject.id;
    let nodes = vec![subject];
    let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
    let plan = prepare_generation_plan(
        &nodes,
        GenerationPlanRequest {
            subject_id,
            preset_id: Some("character_sheet".into()),
            sliders: Vec::new(),
            shot: GenerateShot {
                label: None,
                weight: None,
                prompt: Some("  hold the horizon  ".into()),
            },
            aspect: Some(" 16:9 ".into()),
            seed: 9,
            seed_source: SeedSource::Random,
            locked_seed: None,
            grid: Some(VariantGrid::Aspect { values: vec![" 16:9 ".into(), " 1:1 ".into()] }),
        },
        &caps,
    )
    .unwrap();
    let scene = normalize_scene_controls("  hold the horizon  ", Some(" 16:9 ")).unwrap();

    assert_eq!(plan.controls.user_prompt, scene.prompt);
    assert_eq!(plan.controls.shot_weight, scene.shot_weight);
    assert_eq!(plan.cells[0].aspect, scene.aspect);
    assert_eq!(plan.cells[1].aspect, AspectRatio::parse("1:1").unwrap());
}

/* ── what the three callers plan ──────────────────────────────────────── */

#[test]
fn a_batch_plans_one_receipt_and_one_request_per_cell() {
    let world = PlanWorld::new();
    let mut request = world.request("character_sheet", 100);
    request.locked_seed = Some(100);
    request.seed_source = SeedSource::Locked;
    request.shot = GenerateShot {
        label: Some("  low angle  ".into()),
        weight: Some(0.5),
        prompt: Some("  wind catches the cloak  ".into()),
    };
    let planned = world.plan(request).unwrap();

    assert_eq!(planned.label, "Generate Kael Vantris ×4");
    assert_eq!(planned.subject_id, world.kael);
    assert!(planned.requires_billing);
    assert!(!planned.archival_replay);
    // `character_sheet` emits four images on the adjacent-seed family, and
    // only the first of them is the lock itself.
    assert_eq!(
        planned.plans.iter().map(|plan| plan.generation.seed).collect::<Vec<_>>(),
        [100, 101, 102, 103]
    );
    assert_eq!(
        planned.plans.iter().map(|plan| plan.request.seed).collect::<Vec<_>>(),
        [100, 101, 102, 103]
    );

    let first = &planned.plans[0].generation;
    assert_eq!(
        keys(&first.params),
        [
            "aspect",
            "batchIndex",
            "batchSize",
            "controls",
            "estimatedCostUsdMicros",
            "height",
            "lockedSeed",
            "loraDowngrades",
            "loras",
            "negativePromptSupported",
            "pricingCheckedAt",
            "pricingConservativeFallback",
            "pricingIndicative",
            "pricingSource",
            "referenceAssetIds",
            "requestedAspect",
            "seedSource",
            "usedLockedSeed",
            "width",
        ]
    );
    assert_eq!(first.preset, "character_sheet");
    assert_eq!(first.view_type, None);
    assert_eq!(first.user_prompt, "wind catches the cloak");
    assert_eq!(first.backend, gemini::ID);
    assert_eq!(first.model, MODEL);
    assert_eq!(first.params["batchIndex"], json!(0));
    assert_eq!(first.params["batchSize"], json!(4));
    assert_eq!(first.params["requestedAspect"], json!("3:4"));
    assert_eq!(first.params["aspect"], json!("3:4"));
    assert_eq!(first.params["seedSource"], json!("locked"));
    assert_eq!(first.params["lockedSeed"], json!(100));
    assert_eq!(first.params["usedLockedSeed"], json!(true));
    assert_eq!(first.params["referenceAssetIds"], json!([world.kael_costume]));
    assert_eq!(
        first.params["controls"],
        json!({
            "sliders": [],
            "shot": { "label": "low angle", "weight": 0.5, "prompt": "wind catches the cloak" },
        })
    );
    assert!(first.compiled_prompt.contains("Ash-grey longcoat"));
    assert!(first.compiled_prompt.contains("wind catches the cloak"));
    // Gemini has no negative prompt, so negotiation withholds the `never`
    // section rather than the receipt claiming one was sent.
    assert_eq!(first.params["negativePromptSupported"], json!(false));
    assert_eq!(first.negative_prompt, "");
    assert_eq!(planned.plans[0].request.references.len(), 1);
    assert_eq!(planned.plans[0].request.references[0].asset_id, world.kael_costume);

    // Only the first cell is the lock, and every cell is priced.
    assert_eq!(planned.plans[1].generation.params["usedLockedSeed"], json!(false));
    assert_eq!(planned.plans[1].generation.params["seedSource"], json!("locked_derived"));
    let cost = planned.plans[0].cost_usd_micros;
    assert!(cost > 0);
    assert!(planned.plans.iter().all(|plan| plan.cost_usd_micros == cost));
}

#[test]
fn a_named_view_batch_plans_eight_tagged_views_on_one_seed() {
    let world = PlanWorld::new();
    let planned = world.plan(world.request("turnaround", 55)).unwrap();
    assert_eq!(
        planned
            .plans
            .iter()
            .map(|plan| (
                plan.generation.view_type.clone().unwrap(),
                plan.generation.seed,
                plan.generation.params["batchSize"].clone()
            ))
            .collect::<Vec<_>>(),
        [
            ("front".to_owned(), 55, json!(8)),
            ("left".to_owned(), 55, json!(8)),
            ("right".to_owned(), 55, json!(8)),
            ("back".to_owned(), 55, json!(8)),
            ("top".to_owned(), 55, json!(8)),
            ("bottom".to_owned(), 55, json!(8)),
            ("left_front".to_owned(), 55, json!(8)),
            ("right_front".to_owned(), 55, json!(8)),
        ]
    );
}

#[test]
fn a_variant_grid_varies_one_axis_and_records_it_per_cell() {
    let world = PlanWorld::new();
    let mut request = world.request("character_sheet", 12);
    request.grid =
        Some(VariantGrid::FragmentWeight { node_id: world.kael, values: vec![0.25, 0.75] });
    let planned = world.plan(request).unwrap();

    assert_eq!(planned.plans.len(), 2);
    for (index, weight) in [0.25_f32, 0.75].into_iter().enumerate() {
        let params = &planned.plans[index].generation.params;
        // One seed across the grid: the axis is the weight, nothing else.
        assert_eq!(planned.plans[index].generation.seed, 12);
        assert_eq!(params["controls"]["sliders"][0]["nodeId"], json!(world.kael));
        assert_eq!(params["controls"]["sliders"][0]["value"], json!(weight));
        assert_eq!(params["variation"]["index"], json!(index));
        assert_eq!(params["variation"]["total"], json!(2));
        assert_eq!(params["variation"]["axis"], json!("fragment_weight"));
        assert_eq!(params["variation"]["weight"], json!(weight));
    }
    assert_eq!(
        planned.plans[0].generation.params["variation"]["gridId"],
        planned.plans[1].generation.params["variation"]["gridId"]
    );
}

#[test]
fn a_composition_plans_one_receipt_naming_every_participant() {
    let world = PlanWorld::new();
    let planned = world.compose("  they meet on the bridge  ", None).unwrap();

    assert_eq!(planned.label, "Compose scene · Kael Vantris + Rell Sarn");
    assert_eq!(planned.subject_id, world.kael);
    assert_eq!(planned.plans.len(), 1);
    let generation = &planned.plans[0].generation;
    assert_eq!(generation.preset, "environment_matte");
    assert_eq!(generation.node_id, world.kael);
    assert_eq!(generation.user_prompt, "they meet on the bridge");
    assert_eq!(generation.params["batchSize"], json!(1));
    assert_eq!(generation.params["requestedAspect"], json!("16:9"));
    assert_eq!(
        generation.params["controls"],
        json!({ "scene": { "prompt": "they meet on the bridge", "aspect": "16:9" } })
    );
    assert_eq!(generation.params["sceneComposition"]["version"], json!(1));
    assert_eq!(
        generation.params["sceneComposition"]["subjectIds"],
        json!([world.kael, world.rell])
    );
    assert_eq!(
        generation.params["sceneComposition"]["subjectNames"],
        json!(["Kael Vantris", "Rell Sarn"])
    );
    // Composition has no per-layer sliders and no `variation`, and it never
    // acquires a `lockedSeed`: the participants may disagree about theirs.
    assert!(!generation.params.contains_key("variation"));
    assert!(!generation.params.contains_key("lockedSeed"));
    assert!(generation.compiled_prompt.contains("Kael Vantris"));
    assert!(generation.compiled_prompt.contains("Rell Sarn"));
    assert_eq!(planned.plans[0].request.references.len(), 2);
}

#[test]
fn the_preview_prices_and_budgets_exactly_what_the_batch_would_send() {
    let world = PlanWorld::new();
    let request = world.request("character_sheet", 3);
    let planned = world.plan(request.clone()).unwrap();
    let preview = world.preview(request).unwrap();

    let cost = preview.cost.unwrap();
    assert_eq!(cost.images, planned.plans.len());
    assert_eq!(cost.per_image_usd_micros, planned.plans[0].cost_usd_micros);
    assert_eq!(
        cost.batch_usd_micros,
        planned.plans.iter().map(|plan| plan.cost_usd_micros).sum::<u64>()
    );
    assert!(!cost.varies_by_cell);

    let kept: usize = preview.buckets.iter().map(|bucket| bucket.kept).sum();
    assert_eq!(kept, planned.plans[0].request.references.len());
    assert_eq!(preview.buckets.iter().map(|bucket| bucket.dropped).sum::<usize>(), 0);
    assert_eq!(preview.layers.iter().map(|layer| layer.kept).sum::<usize>(), kept);
}

#[test]
fn the_preview_budgets_the_grids_first_cell_and_not_the_ungridded_request() {
    // The report used to negotiate the request's own fragment set, which
    // for a grid is a set no cell sends. Silencing the subject in cell one
    // is the case where that shows: the reference the report counted was
    // one the first image would not have carried.
    let world = PlanWorld::new();
    let mut request = world.request("character_sheet", 4);
    request.grid =
        Some(VariantGrid::FragmentWeight { node_id: world.kael, values: vec![0.0, 1.0] });
    let planned = world.plan(request.clone()).unwrap();
    let preview = world.preview(request).unwrap();

    assert_eq!(planned.plans[0].request.references.len(), 0);
    assert_eq!(planned.plans[1].request.references.len(), 1);
    assert_eq!(preview.buckets.iter().map(|bucket| bucket.kept).sum::<usize>(), 0);
}

#[test]
fn the_preview_budgets_the_first_view_of_a_named_view_preset() {
    // A Turnaround's cells are per-view, so the report has to be a report
    // about a view: the viewless fragment set is one this preset never
    // sends. First rather than an average, because it is the cell execution
    // renders first and the only one a single report can be true about.
    let world = PlanWorld::new();
    let request = world.request("turnaround", 5);
    let planned = world.plan(request.clone()).unwrap();
    let preview = world.preview(request).unwrap();

    let kept: usize = preview.buckets.iter().map(|bucket| bucket.kept).sum();
    assert_eq!(kept, planned.plans[0].request.references.len());
    assert_eq!(preview.cost.unwrap().images, 8);
}

#[test]
fn named_view_presets_are_refused_as_variant_grids() {
    let subject = Node::new(wobu_core::NodeKind::Character, "Kael").unwrap();
    let caps = GeminiBackend::new("test-key").unwrap().capabilities("gemini-3.1-flash-image");
    let grid = VariantGrid::Seed { values: vec![1, 2] };
    let error = variant_cells(
        &subject,
        *preset("turnaround").unwrap(),
        AspectRatio::parse("1:1").unwrap(),
        1,
        SeedSource::Locked,
        &[],
        &HashSet::from([subject.id]),
        Some(&grid),
        &caps,
    )
    .unwrap_err();
    assert_eq!(error.code, Code::Invalid);
}

#[test]
fn competing_reservations_cannot_consume_the_same_remaining_ceiling() {
    let project = TestProject::new(100_000);
    let first = SpendReservation::create(&project.root, 60_000).unwrap();
    let second = SpendReservation::create(&project.root, 50_000).unwrap_err();
    assert_eq!(second.code, Code::SpendCeilingExceeded);

    let held = spend_status_for(&project.root).unwrap();
    assert_eq!(held.spent_usd_micros, 0);
    assert_eq!(held.reserved_usd_micros, 60_000);
    assert_eq!(held.remaining_usd_micros, Some(40_000));
    drop(first);
    assert_eq!(spend_status_for(&project.root).unwrap().reserved_usd_micros, 0);
}

#[test]
fn committed_receipt_and_reduced_reservation_are_not_double_counted() {
    let project = TestProject::new(200_000);
    let mut reservation = SpendReservation::create(&project.root, 134_000).unwrap();
    let mut generation =
        receipt(project.node_id, gemini::ID, "gemini-3.1-flash-image", 1_024, 1_024);
    generation.params.insert("estimatedCostUsdMicros".into(), json!(67_000));
    let mut store = Project::open(&project.root).unwrap();
    store.record_generation(generation).unwrap();
    drop(store);
    reservation.commit(67_000).unwrap();

    let status = spend_status_for(&project.root).unwrap();
    assert_eq!(status.spent_usd_micros, 67_000);
    assert_eq!(status.reserved_usd_micros, 67_000);
    assert_eq!(status.remaining_usd_micros, Some(66_000));
}

#[test]
fn unchanged_poll_skips_four_thousand_artificially_slow_receipts() {
    let project = TestProject::new(500_000_000);
    std::fs::create_dir_all(project.root.join(SPEND_DIR).join("reservations")).unwrap();
    let receipts: Vec<_> = (0..4_000)
        .map(|_| {
            let mut generation =
                receipt(project.node_id, gemini::ID, "gemini-3.1-flash-image", 1_024, 1_024);
            generation.params.insert("estimatedCostUsdMicros".into(), json!(67_000));
            generation
        })
        .collect();
    let opened = std::cell::Cell::new(0_usize);
    let rebuild_started = std::time::Instant::now();
    let rebuilt = read_cached_spend_status_locked_with(&project.root, || {
        for _ in &receipts {
            opened.set(opened.get() + 1);
            // Models the fixed per-file cost of opening a receipt over a
            // shared mount without making the test depend on one.
            std::thread::sleep(Duration::from_micros(25));
        }
        Ok((Some(500_000_000), receipts))
    })
    .unwrap();
    let rebuild_elapsed = rebuild_started.elapsed();
    assert_eq!(opened.get(), 4_000);
    assert_eq!(rebuilt.spent_usd_micros, 268_000_000);

    opened.set(0);
    let poll_started = std::time::Instant::now();
    let unchanged = read_cached_spend_status_locked_with(&project.root, || {
        opened.set(usize::MAX);
        panic!("an unchanged poll reopened the canonical receipt ledger")
    })
    .unwrap();
    let poll_elapsed = poll_started.elapsed();

    assert_eq!(unchanged.spent_usd_micros, rebuilt.spent_usd_micros);
    assert_eq!(opened.get(), 0, "the cached poll opened no receipt files");
    assert!(
        poll_elapsed < rebuild_elapsed,
        "cached poll {poll_elapsed:?} did not beat artificial receipt latency {rebuild_elapsed:?}",
    );
}

#[test]
fn cache_loss_reconstructs_from_canonical_receipts() {
    let project = TestProject::new(200_000);
    let mut generation =
        receipt(project.node_id, gemini::ID, "gemini-3.1-flash-image", 1_024, 1_024);
    generation.params.insert("estimatedCostUsdMicros".into(), json!(67_000));
    let mut store = Project::open(&project.root).unwrap();
    store.record_generation(generation).unwrap();
    drop(store);

    let aggregate = project.root.join(SPEND_DIR).join(SPEND_AGGREGATE);
    let _ = std::fs::remove_file(&aggregate);
    let rebuilt = spend_status_for_report(&project.root).unwrap();
    assert_eq!(rebuilt.spent_usd_micros, 67_000);
    assert!(aggregate.is_file(), "the disposable aggregate was reconstructed");
}

#[test]
fn malformed_canonical_receipt_fails_closed() {
    let project = TestProject::new(200_000);
    // Establish a plausible cache first. Admission must ignore it rather
    // than letting it hide a malformed receipt that arrives afterwards.
    assert_eq!(spend_status_for_report(&project.root).unwrap().spent_usd_micros, 0);
    let month = project.root.join("generations/2026-08");
    std::fs::create_dir_all(&month).unwrap();
    std::fs::write(month.join(format!("{}.json", new_id())), b"{not-json").unwrap();

    let error = spend_status_for(&project.root).unwrap_err();
    assert_eq!(error.code, Code::Malformed);
    assert!(SpendReservation::create(&project.root, 1).is_err());
}
