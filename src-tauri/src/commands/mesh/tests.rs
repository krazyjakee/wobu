//! Sheet layout, and the orchestration around one reconstruction.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;
use wobu_core::{Asset, Generation, Id, InfluenceSnapshot, new_id};
use wobu_imagine::{
    Error as ImageError, FACE_COUNT, GenerateType, MeshBackend, MeshCapabilities, MeshFormat,
    MeshRequest, MeshUsage, ProgressSink, View,
};
use wobu_jobs::Billed;
use wobu_store::Project;

use crate::error::Code;

use super::task::{PersistMesh, mesh_failure, persist_failed_receipt, persist_mesh};
use super::turnaround::sheet;
use super::{build_request, parse_generate_type, resolve_views};

#[cfg(test)]
mod sheet_layout {
    use super::*;

    fn receipt(id: Id, view: Option<&str>, seed: u64, at: &str, asset: Option<Id>) -> Generation {
        Generation {
            id,
            node_id: Id::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
            created_at: at.parse::<DateTime<Utc>>().unwrap(),
            preset: "turnaround".into(),
            view_type: view.map(str::to_owned),
            user_prompt: String::new(),
            compiled_prompt: "kael".into(),
            negative_prompt: String::new(),
            backend: "comfyui".into(),
            model: "flux-dev".into(),
            seed,
            params: Default::default(),
            output_asset_ids: asset.into_iter().collect(),
            influence_snapshot: InfluenceSnapshot { layers: Vec::new() },
        }
    }

    fn full_run(seed: u64, at: &str) -> Vec<Generation> {
        View::ALL
            .into_iter()
            .map(|view| receipt(new_id(), Some(view.as_str()), seed, at, Some(new_id())))
            .collect()
    }

    #[test]
    fn an_empty_history_still_answers_with_the_eight_slots_it_is_waiting_for() {
        // The 3D tab has to be able to say *what* is missing before anything has
        // been generated; "no turnaround" and "seven of eight" are different
        // sentences and only one of them is a reroll away from reconstruction.
        let sheet = sheet(&[]);
        assert_eq!(sheet.views.len(), 8);
        assert_eq!(sheet.missing.len(), 8);
        assert_eq!(sheet.missing[0], "front");
        assert!(sheet.batches.is_empty());
    }

    #[test]
    fn a_receipt_with_no_image_is_not_a_take() {
        // A billed failure is still a receipt, and it is tagged with its view.
        // Offering it as a reconstruction input would send a paid request with
        // an asset id that resolves to nothing.
        let sheet = sheet(&[receipt(new_id(), Some("front"), 7, "2026-08-01T12:00:00Z", None)]);
        assert!(sheet.views[0].takes.is_empty());
        assert_eq!(sheet.missing.len(), 8);
    }

    #[test]
    fn only_a_complete_run_of_one_seed_is_offered_as_a_batch() {
        let mut history = full_run(11, "2026-08-01T12:00:00Z");
        history.truncate(7);
        assert!(sheet(&history).batches.is_empty(), "seven views is not a turnaround");

        let history = full_run(11, "2026-08-01T12:00:00Z");
        let sheet = sheet(&history);
        assert_eq!(sheet.batches.len(), 1);
        assert_eq!(sheet.batches[0].seed, 11);
        assert_eq!(sheet.batches[0].generation_ids.len(), 8);
        assert!(sheet.missing.is_empty());
    }

    #[test]
    fn batches_are_newest_first_and_takes_within_a_view_are_too() {
        // The sheet is what the review step reads. Newest first is what makes
        // "the batch I just generated" the default rather than the first one
        // this entity ever had.
        let mut history = full_run(11, "2026-08-01T12:00:00Z");
        history.extend(full_run(22, "2026-08-02T12:00:00Z"));
        let sheet = sheet(&history);
        assert_eq!(sheet.batches.iter().map(|b| b.seed).collect::<Vec<_>>(), [22, 11]);
        assert_eq!(sheet.views[0].takes.len(), 2);
        assert_eq!(sheet.views[0].takes[0].seed, 22);
    }

    #[test]
    fn a_rerolled_view_is_a_take_on_its_own_seed_and_not_a_batch() {
        // The whole reason takes exist. The Turnaround preset locks one seed
        // across eight views, so re-rolling the back view *must* use a
        // different one — and a design that only knew about batches would
        // either lose the reroll or invent an eight-view run out of one image.
        let mut history = full_run(11, "2026-08-01T12:00:00Z");
        history.push(receipt(new_id(), Some("back"), 99, "2026-08-03T12:00:00Z", Some(new_id())));
        let sheet = sheet(&history);
        assert_eq!(sheet.batches.len(), 1, "the reroll did not create a second run");
        let back = sheet.views.iter().find(|slot| slot.view_type == "back").unwrap();
        assert_eq!(back.takes.len(), 2);
        assert_eq!(back.takes[0].seed, 99, "the reroll is the default take");
    }

    #[test]
    fn a_view_name_from_outside_the_eight_is_ignored_rather_than_placed() {
        let ninth =
            receipt(new_id(), Some("three_quarter"), 1, "2026-08-01T12:00:00Z", Some(new_id()));
        let sheet = sheet(&[ninth]);
        assert!(sheet.views.iter().all(|slot| slot.takes.is_empty()));
    }

    #[test]
    fn the_generate_mode_names_are_the_providers_and_nothing_else_parses() {
        assert_eq!(parse_generate_type(None).unwrap(), GenerateType::Normal);
        assert_eq!(parse_generate_type(Some("  ")).unwrap(), GenerateType::Normal);
        assert_eq!(parse_generate_type(Some("Geometry")).unwrap(), GenerateType::Geometry);
        assert_eq!(parse_generate_type(Some("geometry")).unwrap(), GenerateType::Geometry);
        assert!(parse_generate_type(Some("Blockout")).is_err());
    }

    #[test]
    fn a_billed_failure_is_reported_as_charged_with_what_it_cost() {
        // The queue decides whether to retry from `Billed`. Reporting a paid
        // Hunyuan job as free would make the queue retry it on the user's card.
        let failure = mesh_failure(&ImageError::NoMesh, MeshUsage::billed(1), true);
        assert_eq!(failure.billed, Billed::Charged);
        assert_eq!(failure.cost_note.as_deref(), Some("1 billed job"));

        // Unbilled but past the point of no return on a paid backend is
        // "nobody can tell", which the queue also treats as charged.
        let unknown = mesh_failure(&ImageError::NoMesh, MeshUsage::free(), true);
        assert_eq!(unknown.billed, Billed::Unknown);

        // The local tier costs nothing, so the same error is retryable-free.
        let local = mesh_failure(&ImageError::NoMesh, MeshUsage::free(), false);
        assert_eq!(local.billed, Billed::Nothing);
        assert!(local.cost_note.is_none());
    }
}

/// The half of #110 that is a *chain* rather than a function: reviewed
/// receipts, a provider that answers, and a mesh the 3D gallery can find.
///
/// Driven against a fake [`MeshBackend`] on a real temporary project. There are
/// no Tencent credentials in this tree and a live job costs money, so what is
/// proved here is everything either side of the network: which views reach the
/// adapter and in what order, what the options do to the request, and that the
/// bytes coming back become a GLB plus the one receipt field the gallery reads.
#[cfg(test)]
mod orchestration {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use wobu_core::{AssetKind, Node, NodeKind};
    use wobu_imagine::{GeneratedMesh, MeshFile, MeshOutcome};

    use super::*;

    /// A private directory per test. `tempfile` is not a dependency of this
    /// crate, and `sync.rs`'s tests make the same call for the same reason.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wobu-mesh-{name}-{}", new_id()));
        std::fs::create_dir_all(&dir).expect("a temp directory");
        dir
    }

    /// A PNG header, which is all `image::probe` and `dimensions::read` need.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0]);
        out
    }

    /// A GLB whose header is self-consistent, which is exactly what
    /// `wobu_store::assets::validate_mesh` insists on before it writes one.
    fn glb(payload: &[u8]) -> Vec<u8> {
        let mut out = b"glTF".to_vec();
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
        while out.len() < 20 {
            out.push(0);
        }
        let len = out.len() as u32;
        out[8..12].copy_from_slice(&len.to_le_bytes());
        out
    }

    struct Scene {
        dir: PathBuf,
        project: Project,
        node: Node,
        /// View name → the generation that rendered it.
        views: HashMap<String, Id>,
    }

    /// A node with a complete, rendered turnaround behind it.
    fn scene(name: &str, size: u32) -> Scene {
        let dir = scratch(name);
        let mut project = Project::create(&dir, "Ashfall").expect("a project");
        let node = project.create_node(NodeKind::Character, "Kael", None).expect("a node");
        let mut views = HashMap::new();
        for (index, view) in View::ALL.into_iter().enumerate() {
            let asset = project
                .import_asset(&png(size, size + index as u32), AssetKind::Generated)
                .expect("an imported view");
            let generation = project
                .record_generation(turnaround_receipt(node.id, view, asset.asset.id))
                .expect("a recorded view");
            views.insert(view.to_string(), generation.id);
        }
        Scene { dir, project, node, views }
    }

    fn turnaround_receipt(node_id: Id, view: View, asset_id: Id) -> Generation {
        Generation {
            id: new_id(),
            node_id,
            created_at: Utc::now(),
            preset: "turnaround".into(),
            view_type: Some(view.to_string()),
            user_prompt: String::new(),
            compiled_prompt: "kael, ash-grey coat".into(),
            negative_prompt: String::new(),
            backend: "comfyui".into(),
            model: "flux-dev".into(),
            seed: 4_242,
            params: Default::default(),
            output_asset_ids: vec![asset_id],
            influence_snapshot: InfluenceSnapshot { layers: Vec::new() },
        }
    }

    fn assets_of(project: &Project) -> HashMap<Id, Asset> {
        project.list_assets().expect("assets").into_iter().map(|a| (a.id, a)).collect()
    }

    fn ordered(scene: &Scene) -> Vec<Id> {
        View::ALL.into_iter().map(|view| scene.views[view.as_str()]).collect()
    }

    /// Records what it was asked for and answers with whatever it was told to.
    struct FakeBackend {
        seen: Mutex<Option<MeshRequest>>,
        answer: Mutex<Option<GeneratedMesh>>,
    }

    impl FakeBackend {
        fn returning(mesh: GeneratedMesh) -> FakeBackend {
            FakeBackend { seen: Mutex::new(None), answer: Mutex::new(Some(mesh)) }
        }
    }

    #[async_trait]
    impl MeshBackend for FakeBackend {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn label(&self) -> &'static str {
            "Fake mesh backend"
        }
        fn default_model(&self) -> &'static str {
            "3.1"
        }
        fn capabilities(&self, _model: &str) -> MeshCapabilities {
            MeshCapabilities {
                max_views: 8,
                face_count: FACE_COUNT,
                pbr: true,
                generate_types: vec![GenerateType::Normal, GenerateType::Geometry],
                text_to_mesh: true,
                requires_billing: true,
            }
        }
        async fn generate(
            &self,
            request: &MeshRequest,
            _progress: &mut dyn ProgressSink,
            _cancel: &wobu_imagine::Cancel,
        ) -> MeshOutcome {
            *self.seen.lock().unwrap() = Some(request.clone());
            match self.answer.lock().unwrap().take() {
                Some(mesh) => MeshOutcome::new(MeshUsage::billed(1), Ok(mesh)),
                None => MeshOutcome::new(MeshUsage::billed(1), Err(ImageError::NoMesh)),
            }
        }
    }

    #[test]
    fn a_reviewed_turnaround_becomes_a_mesh_the_3d_gallery_can_find() {
        // The whole of #110 in one test. Before this file existed the last two
        // assertions were unreachable: nothing in the tree wrote `meshOutput`,
        // so `mesh_concepts` could never pair a stored GLB with a receipt.
        let scene = scene("full-chain", 1024);
        let assets = assets_of(&scene.project);
        let chosen = resolve_views(&scene.project, scene.node.id, &ordered(&scene), &assets)
            .expect("eight reviewed views resolve");
        let request = build_request(scene.dir.join("ashfall.wobu").as_path(), chosen, "3.1")
            .expect("a provider-ready request")
            .with_face_count(250_000)
            .with_generate_type(GenerateType::Geometry);

        let backend = FakeBackend::returning(GeneratedMesh {
            format: MeshFormat::Glb,
            mesh: MeshFile::new("model.glb", glb(b"kael")),
            extras: vec![],
            preview: None,
        });
        let outcome = tauri::async_runtime::block_on(backend.generate(
            &request,
            &mut wobu_imagine::Discard,
            &wobu_imagine::Cancel::new(),
        ));

        // What the provider was actually handed: eight images, front first, in
        // the order it names them, with the options the user chose.
        let sent = backend.seen.lock().unwrap().clone().expect("the adapter was called");
        let names: Vec<String> = sent.views().iter().map(|view| view.view.to_string()).collect();
        assert_eq!(names, View::ALL.map(|view| view.to_string()).to_vec());
        assert_eq!(sent.face_count, 250_000);
        assert_eq!(sent.generate_type, GenerateType::Geometry);
        assert!(!sent.enable_pbr, "a request nobody edited costs the provider's default");

        let mesh = outcome.result.expect("the fake answered");
        let root = scene.project.root().to_path_buf();
        let ready = persist_mesh(PersistMesh {
            root: root.clone(),
            project_id: scene.project.id(),
            subject_id: scene.node.id,
            generation: mesh_receipt(scene.node.id),
            bytes: &mesh.mesh.bytes,
            turnaround_generation_ids: ordered(&scene),
            usage: outcome.usage,
        })
        .expect("the mesh is stored and the receipt written");

        let reopened = Project::open(&root).expect("the project reopens");
        let stored = reopened.list_meshes();
        assert_eq!(stored.len(), 1, "one content-addressed GLB landed");
        assert_eq!(stored[0].id, ready.asset_id);

        // `mesh_concepts` joins exactly these two facts and nothing else.
        let output = ready.generation.mesh_output().expect("the receipt names its mesh");
        assert_eq!(output.asset_id, ready.asset_id);
        assert_eq!(output.turnaround_generation_ids, ordered(&scene));
        assert_eq!(ready.generation.params["outcome"], json!("done"));
        assert_eq!(ready.generation.params["billedJobs"], json!(1));

        std::fs::remove_dir_all(&scene.dir).ok();
    }

    fn mesh_receipt(node_id: Id) -> Generation {
        Generation {
            id: new_id(),
            node_id,
            created_at: Utc::now(),
            preset: "turnaround".into(),
            view_type: None,
            user_prompt: String::new(),
            compiled_prompt: String::new(),
            negative_prompt: String::new(),
            backend: "fake".into(),
            model: "3.1".into(),
            seed: 4_242,
            params: Default::default(),
            output_asset_ids: Vec::new(),
            influence_snapshot: InfluenceSnapshot { layers: Vec::new() },
        }
    }

    #[test]
    fn a_billed_failure_leaves_a_receipt_that_claims_no_mesh() {
        // The common case on a paid backend, because a mesh job is minutes long.
        // A receipt with a `meshOutput` here would be a permanent empty tile in
        // the 3D gallery pointing at an asset that was never written.
        let scene = scene("billed-failure", 1024);
        let recorded = persist_failed_receipt(
            scene.project.root(),
            scene.project.id(),
            mesh_receipt(scene.node.id),
            "provider.bad_response",
            MeshUsage::billed(1),
        )
        .expect("the receipt is written");

        assert_eq!(recorded.params["outcome"], json!("failed"));
        assert_eq!(recorded.params["errorCode"], json!("provider.bad_response"));
        assert_eq!(recorded.params["billedJobs"], json!(1));
        assert_eq!(recorded.mesh_output(), None);
        assert!(Project::open(scene.project.root()).unwrap().list_meshes().is_empty());

        std::fs::remove_dir_all(&scene.dir).ok();
    }

    #[test]
    fn the_reviewed_views_are_ordered_deduplicated_and_must_include_the_front() {
        let scene = scene("resolve-views", 1024);
        let assets = assets_of(&scene.project);
        let node_id = scene.node.id;

        // Chosen in any order, sent in the provider's.
        let mut shuffled = ordered(&scene);
        shuffled.reverse();
        let resolved =
            resolve_views(&scene.project, node_id, &shuffled, &assets).expect("all eight resolve");
        let names: Vec<String> = resolved.iter().map(|view| view.view.to_string()).collect();
        assert_eq!(names, View::ALL.map(|view| view.to_string()).to_vec());

        // Two takes of one view is a duplicate the provider refuses *after* the
        // upload, so it is refused here instead.
        let front = scene.views["front"];
        let duplicate = vec![front, front];
        let error = resolve_views(&scene.project, node_id, &duplicate, &assets).unwrap_err();
        assert!(error.message.contains("front"), "{}", error.message);

        // A single-image reconstruction *is* the front view.
        let no_front: Vec<Id> = ordered(&scene).into_iter().filter(|id| *id != front).collect();
        let error = resolve_views(&scene.project, node_id, &no_front, &assets).unwrap_err();
        assert!(error.message.contains("front view is required"), "{}", error.message);

        std::fs::remove_dir_all(&scene.dir).ok();
    }

    #[test]
    fn a_generation_that_is_not_a_tagged_view_cannot_be_sent_as_one() {
        // `mesh_concepts` shows an ordinary portrait beside a mesh happily
        // enough. Sending one as the `top` view would be a paid reconstruction
        // of the wrong pictures.
        let mut scene = scene("untagged", 1024);
        let asset = scene
            .project
            .import_asset(&png(900, 900), AssetKind::Generated)
            .expect("an imported portrait");
        let mut portrait = turnaround_receipt(scene.node.id, View::Front, asset.asset.id);
        portrait.preset = "character_sheet".into();
        portrait.view_type = None;
        let portrait = scene.project.record_generation(portrait).expect("a recorded portrait");

        let error = resolve_views(
            &scene.project,
            scene.node.id,
            &[portrait.id],
            &assets_of(&scene.project),
        )
        .unwrap_err();
        assert!(error.message.contains("eight tagged turnaround views"), "{}", error.message);

        std::fs::remove_dir_all(&scene.dir).ok();
    }

    #[test]
    fn the_provider_envelope_is_enforced_before_anything_is_signed_and_sent() {
        // `Turnaround::new` proves format, dimensions and the combined payload
        // cap — the last of which no single view can break and all eight can.
        // Failing here is free; failing at the provider is a signed, billed
        // call that comes back `InvalidParameterValue`.
        let scene = scene("too-small", 100);
        let assets = assets_of(&scene.project);
        let chosen = resolve_views(&scene.project, scene.node.id, &ordered(&scene), &assets)
            .expect("the receipts themselves are fine");
        let error =
            build_request(scene.project.root(), chosen, "3.1").expect_err("the images are not");
        assert_eq!(error.code, Code::Invalid, "this is the pictures, not a bug in the adapter");
        assert!(error.message.contains("128"), "{}", error.message);

        std::fs::remove_dir_all(&scene.dir).ok();
    }

    #[test]
    fn a_provider_answer_that_is_not_a_glb_is_refused_rather_than_written_as_one() {
        // `wobu_core::asset::mesh_path` names every stored mesh `.glb`. An OBJ
        // archive under that name is a file whose extension lies and a viewer
        // that reports a corrupt mesh.
        let scene = scene("not-a-glb", 1024);
        let error = persist_mesh(PersistMesh {
            root: scene.project.root().to_path_buf(),
            project_id: scene.project.id(),
            subject_id: scene.node.id,
            generation: mesh_receipt(scene.node.id),
            bytes: b"v 0 0 0\n",
            turnaround_generation_ids: ordered(&scene),
            usage: MeshUsage::billed(1),
        })
        .unwrap_err();
        assert!(error.message.to_lowercase().contains("mesh"), "{}", error.message);
        assert!(Project::open(scene.project.root()).unwrap().list_meshes().is_empty());

        std::fs::remove_dir_all(&scene.dir).ok();
    }
}
