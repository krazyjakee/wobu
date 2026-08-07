//! The bridge contract, pinned.
//!
//! `src/lib/api.ts` hand-writes the TypeScript for every payload here. Nothing
//! generates one from the other, so the only thing stopping them drifting is a
//! test that feeds the Rust side exactly the JSON the webview sends. These use
//! literal JSON rather than round-tripping a Rust value on purpose — a
//! round-trip would agree with itself no matter what the frontend believes.

use std::fs::{self, OpenOptions};
use std::path::PathBuf;

use wobu_core::{Asset, AssetKind, AssetRole, Generation, Id, LinkEdge, LinkRole, Node, NodeKind};
use wobu_imagine::{comfy, gemini as image_gemini};
use wobu_llm::{Cancel, EnhanceOutcome, EnhanceRequest, TextProvider, Usage, anthropic, gemini};
use wobu_store::{AssetUsage, Conflict, ImportedAsset, Keep, Peer, Project};

use super::assets::{
    ASSET_TRANSFER_CHUNK_BYTES, AssetTransfer, AssetTransfers, append_asset_transfer_chunk,
    ensure_thumb_unlocked, import_file_unlocked_with,
};
use super::credentials::{ProbeResult, verdict};
use super::diagnostics::log_info;
use super::project::kind_registry;
use super::settings::{
    ActiveModel, BackendHealth, Capability, StatusBarBackend, context_window, image_default,
    provider_region, text_default, write_providers,
};
use super::thumbs::{node_thumb_asset, node_thumb_assets};
use crate::diag;
use crate::error::Code;
use crate::state::ProjectTicket;
use std::sync::atomic::{AtomicUsize, Ordering};
use wobu_store::{AssetUsageRole, ImportWarning};

fn staged_asset_transfer(total_bytes: u64) -> (AssetTransfers, String, PathBuf) {
    let transfers = AssetTransfers::default();
    let transfer_id = wobu_core::new_id().to_string();
    let path = std::env::temp_dir().join(format!("wobu-transfer-test-{transfer_id}.part"));
    let file = OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
    transfers.0.lock().insert(
        transfer_id.clone(),
        AssetTransfer {
            project: ProjectTicket {
                project: wobu_core::new_id(),
                root: PathBuf::new(),
                generation: 0,
            },
            path: path.clone(),
            file,
            kind: AssetKind::Reference,
            received_bytes: 0,
            total_bytes,
        },
    );
    (transfers, transfer_id, path)
}

#[test]
fn raw_asset_chunks_are_bounded_ordered_and_never_retained_in_memory() {
    let total = ASSET_TRANSFER_CHUNK_BYTES as u64 + 17;
    let (transfers, transfer_id, path) = staged_asset_transfer(total);
    let chunk = vec![7; ASSET_TRANSFER_CHUNK_BYTES];

    let first = append_asset_transfer_chunk(&transfers, &transfer_id, 0, &chunk).unwrap();
    assert_eq!(first.received_bytes, ASSET_TRANSFER_CHUNK_BYTES as u64);
    assert_eq!(fs::metadata(&path).unwrap().len(), ASSET_TRANSFER_CHUNK_BYTES as u64);
    assert!(
        std::mem::size_of::<AssetTransfer>() < 256,
        "a transfer session must hold file metadata, never a byte Vec"
    );

    let done = append_asset_transfer_chunk(
        &transfers,
        &transfer_id,
        ASSET_TRANSFER_CHUNK_BYTES as u64,
        &[9; 17],
    )
    .unwrap();
    assert_eq!(done.received_bytes, total);
    assert_eq!(fs::metadata(&path).unwrap().len(), total);
    assert!(transfers.cancel(&transfer_id));
    assert!(!path.exists(), "Cancel must remove the staged file");
}

#[test]
fn the_chunked_transfer_import_reads_its_staged_source_once_before_the_index_commit() {
    let dir = std::env::temp_dir().join(format!("wobu-transfer-import-{}", wobu_core::new_id()));
    fs::create_dir_all(&dir).unwrap();
    let mut project = Project::create(&dir, "Transfer target").unwrap();
    let root = project.asset_import_root().unwrap();
    let staged = dir.join("completed-transfer.part");
    let reads = AtomicUsize::new(0);

    let imported =
        import_file_unlocked_with(&root, &staged, AssetKind::Reference, |path, _cancel| {
            assert_eq!(path, staged);
            reads.fetch_add(1, Ordering::SeqCst);
            // Header-complete so import succeeds; deliberately pixel-
            // incomplete so the best-effort immediate thumbnail is null.
            let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
            png.extend_from_slice(&13u32.to_be_bytes());
            png.extend_from_slice(b"IHDR");
            png.extend_from_slice(&64u32.to_be_bytes());
            png.extend_from_slice(&64u32.to_be_bytes());
            png.extend_from_slice(&[8, 6, 0, 0, 0]);
            Ok(png)
        })
        .unwrap();

    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert!(project.list_assets().unwrap().is_empty(), "slow work must not mutate the index");
    project.record_import(&imported).unwrap();
    assert_eq!(project.list_assets().unwrap(), [imported.asset]);
    drop(project);
    let _ = fs::remove_dir_all(dir);
}

/// A blob record without the blob: enough for every index question here.
fn staged_asset(project: &Project, seed: &str) -> Id {
    let hash = seed.repeat(32);
    let asset = Asset {
        id: wobu_core::new_id(),
        hash: hash.clone(),
        kind: AssetKind::Reference,
        rel_path: format!("assets/originals/{}/{hash}.png", &hash[..2]),
        thumb_path: None,
        mime: "image/png".into(),
        width: 8,
        height: 8,
        bytes: 12,
        created_at: chrono::Utc::now(),
    };
    project.index().upsert_asset(&asset).unwrap();
    asset.id
}

fn concept_receipt(node_id: Id, output: Id) -> Generation {
    Generation {
        id: wobu_core::new_id(),
        node_id,
        created_at: chrono::Utc::now(),
        preset: "portrait".into(),
        view_type: None,
        user_prompt: String::new(),
        compiled_prompt: "an ashwalker".into(),
        negative_prompt: String::new(),
        backend: "comfyui".into(),
        model: "flux-dev".into(),
        seed: 7,
        params: serde_json::Map::new(),
        output_asset_ids: vec![output],
        influence_snapshot: wobu_core::InfluenceSnapshot { layers: Vec::new() },
    }
}

#[test]
fn a_row_picture_prefers_the_cover_then_a_reference_then_the_newest_concept() {
    let dir = std::env::temp_dir().join(format!("wobu-node-thumb-{}", wobu_core::new_id()));
    fs::create_dir_all(&dir).unwrap();
    let mut project = Project::create(&dir, "Row pictures").unwrap();
    let node = project.create_node(NodeKind::Character, "Kael", None).unwrap();

    // A text-only entity is absent from the answer rather than present with
    // a null: the row draws its kind icon and asks for nothing.
    assert_eq!(node_thumb_asset(project.index(), node.id).unwrap(), None);
    assert!(node_thumb_assets(&project, &[node.id]).unwrap().is_empty());

    let concept = staged_asset(&project, "c0");
    let reference = staged_asset(&project, "b1");
    let cover = staged_asset(&project, "a2");

    // A generated concept stands in when nothing has been chosen, which is
    // the case the issue is really about: entities whose only picture was
    // produced by Forge and never pinned anywhere.
    project.record_generation(concept_receipt(node.id, concept)).unwrap();
    assert_eq!(node_thumb_asset(project.index(), node.id).unwrap(), Some(concept));

    // An attached reference outranks it — including one switched off, since
    // `enabled` decides what a backend is sent and was never a statement
    // about what the user should be able to see in a list.
    project.link_asset(node.id, reference, AssetRole::Pose, None).unwrap();
    project.update_asset_link(node.id, reference, AssetRole::Pose, None, Some(false)).unwrap();
    assert_eq!(node_thumb_asset(project.index(), node.id).unwrap(), Some(reference));

    // And an explicit cover outranks everything, because it is the one
    // answer the user gave on purpose.
    project.set_cover_asset(node.id, Some(cover)).unwrap();
    assert_eq!(node_thumb_asset(project.index(), node.id).unwrap(), Some(cover));

    // Repeats collapse: a caller may send whatever its window contains.
    assert_eq!(node_thumb_assets(&project, &[node.id, node.id]).unwrap(), vec![(node.id, cover)]);

    drop(project);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_read_only_thumbnail_snapshot_never_attempts_a_missing_file_write() {
    let root = std::env::temp_dir().join(format!("wobu-read-only-thumb-{}", wobu_core::new_id()));
    fs::create_dir_all(&root).unwrap();
    let target = wobu_store::ThumbTarget {
        asset_id: wobu_core::new_id(),
        hash: "a3f9c1d2e4b5a6978081726354453627a3f9c1d2e4b5a6978081726354453627".into(),
        rel_path: "assets/originals/a3/missing.png".into(),
    };

    assert_eq!(ensure_thumb_unlocked(&root, &target, false).unwrap(), None);
    assert!(
        !root.join("assets").exists(),
        "a read-only/missing thumbnail request must perform no write at all"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_oversized_or_out_of_order_chunk_destroys_its_temp_session() {
    let (oversized, oversized_id, oversized_path) =
        staged_asset_transfer((ASSET_TRANSFER_CHUNK_BYTES + 1) as u64);
    let too_large = vec![0; ASSET_TRANSFER_CHUNK_BYTES + 1];
    assert!(append_asset_transfer_chunk(&oversized, &oversized_id, 0, &too_large).is_err());
    assert!(!oversized_path.exists());

    let (out_of_order, out_of_order_id, out_of_order_path) = staged_asset_transfer(8);
    assert!(append_asset_transfer_chunk(&out_of_order, &out_of_order_id, 4, &[1; 4]).is_err());
    assert!(!out_of_order_path.exists());
}

/// Verbatim from the `WobuNode` interface in `src/lib/api.ts`, including
/// the tagged `SectionValue` shape and a `null` description state.
const NODE_FROM_THE_WEBVIEW: &str = r#"{
    "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
    "kind": "species",
    "name": "Vashk",
    "slug": "vashk",
    "summary": "Ash-adapted, subterranean.",
    "parentId": null,
    "notesRaw": "Notes typed in the editor.",
    "description": {
        "sections": {
            "silhouette": { "type": "text", "value": "Long-limbed." },
            "materials": { "type": "list", "value": ["ashglass", "bone"] }
        }
    },
    "descriptionState": "edited",
    "attributes": { "height_cm": 190 },
    "tags": ["playable"],
    "coverAssetId": null,
    "links": [
        { "toId": "01ARZ3NDEKTSV4RRFFQ69G5FAW", "role": "styled_by", "weight": 0.5, "enabled": true }
    ],
    "assetLinks": [
        { "assetId": "01ARZ3NDEKTSV4RRFFQ69G5FAX", "role": "pose", "weight": 0.5, "enabled": true }
    ],
    "createdAt": "2026-07-31T09:00:00Z",
    "updatedAt": "2026-07-31T09:30:00Z"
}"#;

#[test]
fn node_upsert_accepts_what_the_webview_sends() {
    let node: Node = serde_json::from_str(NODE_FROM_THE_WEBVIEW).expect("node should decode");

    assert_eq!(node.kind, NodeKind::Species);
    assert_eq!(node.name, "Vashk");
    assert_eq!(node.parent_id, None);
    assert_eq!(node.notes_raw, "Notes typed in the editor.");
    assert_eq!(node.description_state, wobu_core::DescriptionState::Edited);
    assert_eq!(node.tags, ["playable"]);
    assert_eq!(node.links.len(), 1);
    assert_eq!(node.links[0].role, wobu_core::LinkRole::StyledBy);

    let description = node.description.as_ref().expect("description should decode");
    assert_eq!(description.text("silhouette"), Some("Long-limbed."));
    assert_eq!(
        description.list("materials"),
        Some(&["ashglass".to_string(), "bone".to_string()][..])
    );
}

#[test]
fn a_node_serialises_back_under_the_keys_the_webview_reads() {
    let node: Node = serde_json::from_str(NODE_FROM_THE_WEBVIEW).unwrap();
    let json = serde_json::to_value(&node).unwrap();

    // The camelCase ones are the ones that would break silently: serde
    // renames them, TypeScript does not know that, and a missing key
    // arrives in the UI as `undefined` rather than as an error.
    for key in [
        "parentId",
        "notesRaw",
        "descriptionState",
        "coverAssetId",
        "assetLinks",
        "createdAt",
        "updatedAt",
    ] {
        assert!(json.get(key).is_some(), "`{key}` is missing from the node payload");
    }
    assert_eq!(json["links"][0]["toId"], "01ARZ3NDEKTSV4RRFFQ69G5FAW");
    assert_eq!(json["assetLinks"][0]["assetId"], "01ARZ3NDEKTSV4RRFFQ69G5FAX");
    assert_eq!(json["description"]["sections"]["materials"]["type"], "list");
}

#[test]
fn asset_roles_are_the_strings_the_role_picker_sends_back() {
    // `role` crosses as a bare snake_case string, and a mismatch fails at
    // the bridge rather than at compile time — the picker would simply stop
    // working with nothing on screen to say why. `full_ref` is the one an
    // automatic rename would get wrong.
    for role in AssetRole::ALL {
        let json = serde_json::to_value(role).unwrap();
        assert_eq!(json.as_str().unwrap(), role.as_str());
        assert_eq!(
            serde_json::from_value::<AssetRole>(json).unwrap(),
            role,
            "{role} does not survive the bridge"
        );
    }
    assert_eq!(serde_json::from_str::<AssetRole>("\"full_ref\"").unwrap(), AssetRole::FullRef);
    assert!(serde_json::from_str::<AssetRole>("\"fullRef\"").is_err());
    assert!(serde_json::from_str::<AssetRole>("\"Mood\"").is_err());
}

#[test]
fn an_asset_link_matches_the_assetlink_interface() {
    let link = wobu_core::asset::AssetRef::new(
        "01ARZ3NDEKTSV4RRFFQ69G5FAX".parse().unwrap(),
        AssetRole::Palette,
    );
    let json = serde_json::to_value(&link).unwrap();

    for key in ["assetId", "role", "weight", "enabled"] {
        assert!(json.get(key).is_some(), "`{key}` is missing from AssetLink");
    }
    assert_eq!(json["role"], "palette");
    assert_eq!(json["assetId"], "01ARZ3NDEKTSV4RRFFQ69G5FAX");

    // The commands take a role and a weight as loose arguments rather than
    // a whole link, so this is also the shape `assetLink` posts back inside
    // a node — and a link that omits both must arrive at the documented
    // defaults rather than at zero.
    let bare: wobu_core::asset::AssetRef =
        serde_json::from_str(r#"{"assetId":"01ARZ3NDEKTSV4RRFFQ69G5FAX","role":"mood"}"#).unwrap();
    assert_eq!(bare.weight, 1.0);
    assert!(bare.enabled);
}

#[test]
fn asset_usage_matches_the_project_wide_library_interface() {
    let usage = AssetUsage {
        asset_id: "01ARZ3NDEKTSV4RRFFQ69G5FAX".parse().unwrap(),
        node_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        node_name: "Vashk".into(),
        node_kind: NodeKind::Species,
        node_tags: vec!["playable".into()],
        roles: vec![AssetUsageRole { role: AssetRole::FullRef, weight: 0.8, enabled: true }],
        cover: true,
    };
    let json = serde_json::to_value(usage).unwrap();

    for key in ["assetId", "nodeId", "nodeName", "nodeKind", "nodeTags", "roles", "cover"] {
        assert!(json.get(key).is_some(), "`{key}` is missing from AssetUsage");
    }
    assert_eq!(json["roles"][0]["role"], "full_ref");
    assert_eq!(json["nodeTags"][0], "playable");
}

#[test]
fn node_link_roles_and_backlinks_match_the_relations_bridge() {
    for role in LinkRole::ALL {
        let json = serde_json::to_value(role).unwrap();
        assert_eq!(json.as_str(), Some(role.as_str()));
        assert_eq!(serde_json::from_value::<LinkRole>(json).unwrap(), role);
    }

    let edge = LinkEdge {
        from_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        to_id: "01ARZ3NDEKTSV4RRFFQ69G5FAW".parse().unwrap(),
        role: LinkRole::MemberOf,
        weight: 0.4,
        enabled: true,
    };
    let json = serde_json::to_value(edge).unwrap();
    for key in ["fromId", "toId", "role", "weight", "enabled"] {
        assert!(json.get(key).is_some(), "`{key}` is missing from LinkEdge");
    }
    assert_eq!(json["role"], "member_of");
}

#[test]
fn log_info_matches_the_loginfo_interface() {
    let json = serde_json::to_value(log_info()).unwrap();
    for key in ["path", "level", "exists", "sizeBytes"] {
        assert!(json.get(key).is_some(), "`{key}` is missing from LogInfo");
    }
}

#[test]
fn log_levels_are_the_strings_the_level_buttons_send_back() {
    // The Settings buttons post these values straight back through
    // `log_set_level`, so a rename on either side silently stops working:
    // serde would reject the string and the level would never change.
    let levels: Vec<String> = [
        diag::Level::Off,
        diag::Level::Error,
        diag::Level::Warn,
        diag::Level::Info,
        diag::Level::Debug,
    ]
    .iter()
    .map(|l| serde_json::to_value(l).unwrap().as_str().unwrap().to_owned())
    .collect();

    assert_eq!(levels, ["off", "error", "warn", "info", "debug"]);
    // And back the other way, which is the direction the buttons use.
    assert_eq!(serde_json::from_str::<diag::Level>("\"debug\"").unwrap(), diag::Level::Debug);
}

#[test]
fn a_peer_matches_the_peer_interface() {
    let peer = Peer {
        session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        user: "nadia".into(),
        host: "nadia-mbp".into(),
        seen_secs_ago: 4,
        editing: vec!["01ARZ3NDEKTSV4RRFFQ69G5FAW".parse().unwrap()],
    };
    let json = serde_json::to_value(&peer).unwrap();

    for key in ["sessionId", "user", "host", "seenSecsAgo", "editing"] {
        assert!(json.get(key).is_some(), "`{key}` is missing from Peer");
    }
    // Ids are strings on the far side; a ULID that serialised as an object
    // or a number would arrive as a key nothing in the navigator matches.
    assert_eq!(json["sessionId"], "01ARZ3NDEKTSV4RRFFQ69G5FAV");
    assert_eq!(json["editing"][0], "01ARZ3NDEKTSV4RRFFQ69G5FAW");
}

#[test]
fn a_queue_snapshot_matches_the_queuesnapshot_interface() {
    // `job:state` carries this on every transition and `job_list` returns
    // it, so the status bar (#56) reads it two ways and neither is
    // generated from the other side.
    let snapshot = wobu_jobs::QueueSnapshot {
        jobs: vec![wobu_jobs::JobSnapshot {
            id: wobu_jobs::JobId::new(),
            kind: wobu_jobs::JobKind::Enhance,
            label: "Enhance Vashk".into(),
            subject_id: Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".into()),
            state: wobu_jobs::JobState::Running,
            attempt: 1,
            elapsed_ms: 4200,
        }],
        queued: 2,
        running: 1,
        retrying: 0,
    };
    let json = serde_json::to_value(&snapshot).unwrap();

    for key in ["jobs", "queued", "running", "retrying"] {
        assert!(json.get(key).is_some(), "`{key}` is missing from QueueSnapshot");
    }
    // The state is flattened into the job rather than nested, because the
    // TypeScript is a discriminated union on one field. A `{ state: { … } }`
    // here would draw every job as unknown and no Rust test inside
    // `wobu-jobs` would notice which side was wrong.
    let job = &json["jobs"][0];
    assert_eq!(job["state"], "running");
    assert_eq!(job["kind"], "enhance");
    assert!(job["id"].is_string(), "a job id must cross as a string");
    for key in ["id", "kind", "label", "subjectId", "attempt", "elapsedMs"] {
        assert!(job.get(key).is_some(), "`{key}` is missing from JobSnapshot");
    }
}

#[test]
fn a_held_retry_reaches_the_webview_as_the_offer_it_is() {
    // `retryHeld` and `billed` are the whole of the "never auto-retry
    // something that costs money" design as the user experiences it: they
    // are what turns a failure into "try again — it will cost you". Dropped
    // on the wire, the UI has no way to tell a dead end from a question.
    let failure = wobu_jobs::Failure::new("provider.bad_response", "The response was cut short.")
        .retryable(true)
        .billed(wobu_jobs::Billed::Charged)
        .cost_note("812 in + 400 out");
    let state = wobu_jobs::JobState::Failed { failure, retry_held: true };
    let json = serde_json::to_value(&state).unwrap();

    assert_eq!(json["state"], "failed");
    assert_eq!(json["retryHeld"], true);
    assert_eq!(json["failure"]["billed"], "charged");
    assert_eq!(json["failure"]["costNote"], "812 in + 400 out");
    assert_eq!(json["failure"]["retryable"], true);
    // The same dotted codes command errors use, so `errorSurface` in
    // `src/lib/api.ts` can be pointed at either without a second taxonomy.
    assert_eq!(json["failure"]["code"], Code::ProviderBadResponse.as_str());
}

#[test]
fn presence_editing_accepts_the_node_ids_the_webview_sends() {
    // `presenceEditing` posts a bare array of id strings. Anything else on
    // this side and the call fails silently at the bridge, leaving the
    // editing list frozen on whatever node was open first.
    let ids: Vec<Id> =
        serde_json::from_str(r#"["01ARZ3NDEKTSV4RRFFQ69G5FAV"]"#).expect("ids should decode");
    assert_eq!(ids.len(), 1);
}

#[test]
fn a_conflict_matches_the_conflict_interface() {
    let conflict = Conflict {
        rel_path: "nodes/character/kael.conflict-nadia-20260731T142211Z.md".into(),
        node_rel_path: "nodes/character/kael.md".into(),
        node_id: Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()),
        node_name: Some("Kael Vantris".into()),
        user: Some("nadia".into()),
        saved_at: Some("2026-07-31T14:22:11Z".parse().unwrap()),
        mine: false,
        parked: "hers".into(),
        current: "ours".into(),
        current_hash: "abc123".into(),
    };
    let json = serde_json::to_value(&conflict).unwrap();

    for key in [
        "relPath",
        "nodeRelPath",
        "nodeId",
        "nodeName",
        "user",
        "savedAt",
        "mine",
        "parked",
        "current",
        "currentHash",
    ] {
        assert!(json.get(key).is_some(), "`{key}` is missing from Conflict");
    }
    // The card renders a time from this, so it has to arrive as something
    // `new Date()` understands rather than as a serde struct.
    assert_eq!(json["savedAt"], "2026-07-31T14:22:11Z");
    assert_eq!(json["nodeId"], "01ARZ3NDEKTSV4RRFFQ69G5FAV");
}

#[test]
fn conflict_resolve_accepts_the_choice_the_buttons_send() {
    // `keep` is a bare string on the wire. Anything else on this side and
    // the buttons fail at the bridge rather than at compile time.
    assert_eq!(serde_json::from_str::<Keep>("\"parked\"").unwrap(), Keep::Parked);
    assert_eq!(serde_json::from_str::<Keep>("\"current\"").unwrap(), Keep::Current);
    assert!(serde_json::from_str::<Keep>("\"mine\"").is_err());
}

#[test]
fn an_import_matches_the_importedasset_interface() {
    let imported = ImportedAsset {
        asset: Asset {
            id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
            hash: "a3f9c1d2e4b5a6978081726354453627a3f9c1d2e4b5a6978081726354453627".into(),
            kind: AssetKind::Reference,
            rel_path: "assets/originals/a3/a3f9c1.png".into(),
            thumb_path: None,
            mime: "image/png".into(),
            width: 1024,
            height: 768,
            bytes: 240_512,
            created_at: "2026-07-31T09:00:00Z".parse().unwrap(),
        },
        deduped: true,
        warnings: vec![ImportWarning::MeshTooSmall],
    };
    let json = serde_json::to_value(&imported).unwrap();

    assert!(json.get("deduped").is_some(), "`deduped` is missing from ImportedAsset");
    // The library card puts these next to the thumbnail, so they arrive as
    // the snake_case tags the far side switches on rather than as prose —
    // the wording is `ImportWarning::label`'s to change.
    assert_eq!(json["warnings"], serde_json::json!(["mesh_too_small"]));
    for key in [
        "id",
        "hash",
        "kind",
        "relPath",
        "thumbPath",
        "mime",
        "width",
        "height",
        "bytes",
        "createdAt",
    ] {
        assert!(json["asset"].get(key).is_some(), "`{key}` is missing from Asset");
    }
    // The id is the handle a node's `coverAssetId` and every AssetLink
    // carries, so it has to arrive as a plain ULID string.
    assert_eq!(json["asset"]["id"], "01ARZ3NDEKTSV4RRFFQ69G5FAV");
    // A thumbnail nothing has made yet is `null`, not an absent key —
    // `thumbPath: string | null` on the far side.
    assert!(json["asset"]["thumbPath"].is_null());
}

#[test]
fn asset_import_accepts_the_kind_the_webview_sends() {
    // `kind` is a bare snake_case string on the wire. A mismatch fails at
    // the bridge rather than at compile time, and every drop would be
    // rejected with nothing on screen to say why.
    assert_eq!(serde_json::from_str::<AssetKind>("\"reference\"").unwrap(), AssetKind::Reference);
    assert_eq!(serde_json::from_str::<AssetKind>("\"generated\"").unwrap(), AssetKind::Generated);
    assert_eq!(serde_json::from_str::<AssetKind>("\"upload\"").unwrap(), AssetKind::Upload);
    assert!(serde_json::from_str::<AssetKind>("\"Reference\"").is_err());
}

#[test]
fn the_kind_registry_matches_the_kinddef_interface() {
    let json = serde_json::to_value(kind_registry()).unwrap();
    let first = &json[0];

    for key in [
        "kind",
        "label",
        "plural",
        "icon",
        "color",
        "layer",
        "dir",
        "nests",
        "singleton",
        "attributes",
        "sections",
        "defaultLinkRoles",
    ] {
        assert!(first.get(key).is_some(), "`{key}` is missing from KindDef");
    }
    for key in ["key", "label", "valueKind"] {
        assert!(first["sections"][0].get(key).is_some(), "`{key}` is missing from SectionDef");
    }
    let world = json.as_array().unwrap().iter().find(|d| d["kind"] == "world_bible").unwrap();
    for key in ["key", "label", "valueKind"] {
        assert!(world["attributes"][0].get(key).is_some(), "`{key}` is missing from AttributeDef");
    }
    // The union in `api.ts` is snake_case; the enum has to agree.
    let kinds: Vec<&str> =
        json.as_array().unwrap().iter().map(|d| d["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"style_guide"), "got {kinds:?}");
    assert!(kinds.contains(&"world_bible"), "got {kinds:?}");
}

#[test]
fn the_three_capabilities_cross_as_the_strings_the_pane_sends_back() {
    // The Settings pane posts one of these on every provider change, and a
    // rename on either side is a dropdown that silently stops working:
    // serde refuses the string and nothing is ever written.
    for (capability, wire) in
        [(Capability::Text, "text"), (Capability::Image, "image"), (Capability::Mesh, "mesh")]
    {
        assert_eq!(serde_json::to_value(capability).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<Capability>(serde_json::json!(wire)).unwrap(),
            capability,
        );
        // And the key in `project.json` is the same string, which is the
        // half `enhance.rs` reads.
        assert_eq!(capability.key(), wire);
    }
    assert!(serde_json::from_str::<Capability>("\"Text\"").is_err());
}

#[test]
fn only_the_three_hunyuan_regions_can_cross_the_command_boundary() {
    for region in ["ap-singapore", "na-siliconvalley", "eu-frankfurt"] {
        assert_eq!(
            provider_region(Capability::Mesh, "hunyuan3d", Some(region.into())).unwrap(),
            Some(region.into()),
        );
    }
    assert!(provider_region(Capability::Mesh, "hunyuan3d", Some("ap-guangzhou".into())).is_err());
    assert!(provider_region(Capability::Image, "gemini", Some("eu-frankfurt".into())).is_err());
    assert_eq!(provider_region(Capability::Mesh, "hunyuan3d", None).unwrap(), None);
}

#[test]
fn a_probe_result_matches_the_proberesult_interface() {
    let result = ProbeResult {
        provider: "anthropic".into(),
        model: "claude-sonnet-5".into(),
        ok: false,
        message: "Anthropic rejected the API key".into(),
        code: Some("provider.bad_key".into()),
        usage: Usage { input_tokens: 412, cached_input_tokens: 0, output_tokens: 24 },
    };
    let json = serde_json::to_value(&result).unwrap();

    for key in ["provider", "model", "ok", "message", "code", "usage"] {
        assert!(json.get(key).is_some(), "`{key}` is missing from ProbeResult");
    }
    // The usage fields are camelCase on the wire and the pane prints them
    // as what the check cost; an absent one reads as zero and understates
    // the bill.
    for key in ["inputTokens", "cachedInputTokens", "outputTokens"] {
        assert!(json["usage"].get(key).is_some(), "`{key}` is missing from the probe usage");
    }
    assert_eq!(json["code"], "provider.bad_key");
}

#[test]
fn status_bar_models_use_the_same_defaults_and_only_known_context_windows() {
    assert_eq!(text_default(anthropic::ID), anthropic::DEFAULT_MODEL);
    assert_eq!(text_default(gemini::ID), gemini::DEFAULT_MODEL);
    assert_eq!(image_default(comfy::ID), comfy::DEFAULT_MODEL);
    assert_eq!(image_default(image_gemini::ID), image_gemini::DEFAULT_MODEL);
    assert_eq!(context_window(anthropic::ID, "claude-haiku-4-5"), Some(200_000));
    assert_eq!(context_window(anthropic::ID, "future-model"), None);
}

#[test]
fn status_bar_health_matches_the_frontend_union() {
    let status = StatusBarBackend {
        image: Some(ActiveModel {
            provider: comfy::ID.into(),
            label: comfy::LABEL.into(),
            model: "flux-dev".into(),
            context_tokens: None,
        }),
        text: ActiveModel {
            provider: anthropic::ID.into(),
            label: anthropic::LABEL.into(),
            model: anthropic::DEFAULT_MODEL.into(),
            context_tokens: Some(1_000_000),
        },
        health: BackendHealth::Connected { external_queue: Some(2) },
    };
    let json = serde_json::to_value(status).unwrap();
    assert_eq!(json["health"]["state"], "connected");
    assert_eq!(json["health"]["externalQueue"], 2);
    assert_eq!(json["text"]["contextTokens"], 1_000_000);
    assert_eq!(json["image"]["model"], "flux-dev");
}

#[test]
fn a_failed_probe_is_an_answer_rather_than_a_rejection() {
    // The regression: routing a rejected key through the command's `Err`
    // channel would put it in a toast, away from the field that caused it,
    // and the pane would have nothing to disable. Every "the call" failure
    // has to arrive as `ok: false` with a code the pane can style.
    let outcome = EnhanceOutcome::unbilled(wobu_llm::Error::BadKey { provider: "Anthropic" });
    let rejected = verdict(&ProbeAdapter, "claude-sonnet-5".into(), outcome);
    assert!(!rejected.ok);
    assert_eq!(rejected.code.as_deref(), Some("provider.bad_key"));
    assert_eq!(rejected.usage, Usage::default());

    // And the other half: an answer cut off by the token ceiling is what a
    // *passing* probe looks like, because everything the check set out to
    // establish was already settled by the time the provider started
    // writing. Reading it as a failure would report every good key as bad.
    let truncated = EnhanceOutcome::new(
        Usage { input_tokens: 400, cached_input_tokens: 0, output_tokens: 24 },
        Err(wobu_llm::Error::Truncated),
    );
    let passed = verdict(&ProbeAdapter, "claude-sonnet-5".into(), truncated);
    assert!(passed.ok, "{}", passed.message);
    assert_eq!(passed.code, None);
    assert_eq!(passed.usage.output_tokens, 24, "the pane says what the check cost");
}

/// Stands in for whichever adapter the probe built. Only the three name
/// methods are reached — [`verdict`] never calls one.
struct ProbeAdapter;

#[async_trait::async_trait]
impl TextProvider for ProbeAdapter {
    fn id(&self) -> &'static str {
        anthropic::ID
    }
    fn label(&self) -> &'static str {
        anthropic::LABEL
    }
    fn default_model(&self) -> &'static str {
        "claude-sonnet-5"
    }
    async fn enhance(
        &self,
        _request: &EnhanceRequest,
        _deltas: &mut dyn wobu_llm::DeltaSink,
        _cancel: &Cancel,
    ) -> EnhanceOutcome {
        unreachable!("the verdict tests never make a call")
    }
}

/* ── the shared selection ─────────────────────────────────────────────── */

#[test]
fn writing_a_selection_leaves_the_rest_of_project_json_alone() {
    // `project.json` is shared across a drive, so this file is written by
    // builds of different vintages. Re-serialising `ProjectMeta` would drop
    // every field this build has never heard of — including a fourth
    // capability under `providers` — and the loss would be invisible until
    // the collaborator who set it noticed their world had changed.
    let root = std::env::temp_dir().join(format!("wobu-providers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("project.json");
    std::fs::write(
        &path,
        r#"{
          "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
          "name": "Ashfall",
          "schemaVersion": 1,
          "createdAt": "2026-07-31T09:00:00Z",
          "somethingANewerBuildWrote": { "keep": "me" },
          "providers": { "image": { "provider": "comfyui" } }
        }"#,
    )
    .unwrap();

    let mut providers = serde_json::Map::new();
    providers.insert("image".to_owned(), serde_json::json!({ "provider": "comfyui" }));
    providers.insert(
        "text".to_owned(),
        serde_json::json!({ "provider": "gemini", "model": "gemini-3.6-flash" }),
    );
    write_providers(&root, &providers).unwrap();

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written["somethingANewerBuildWrote"]["keep"], "me");
    assert_eq!(written["name"], "Ashfall");
    assert_eq!(written["providers"]["text"]["provider"], "gemini");
    // The other capabilities are untouched: three selections are three
    // independent choices, and setting one must not clear the others.
    assert_eq!(written["providers"]["image"]["provider"], "comfyui");

    // Nothing is left in staging — a `.part` beside a project is litter that
    // replicates to everyone on the share.
    assert!(!root.join(".wobu/tmp/project.json.part").exists());
    let _ = std::fs::remove_dir_all(&root);
}
