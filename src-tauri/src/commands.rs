//! The command surface the webview calls.
//!
//! Everything here is glue: argument shapes, the lock, and the error mapping.
//! There is no domain logic in this file and there should never be any — the
//! rules live in `wobu-core`, the persistence in `wobu-store`. If a command
//! grows a branch that decides something about the world, it is in the wrong
//! crate.
//!
//! Argument names are snake_case; Tauri v2 matches them against the camelCase
//! keys `src/lib/api.ts` sends.

use std::path::PathBuf;

use tauri::{AppHandle, Emitter, State};
// Aliased because the command below has to *be* called `kind_registry` —
// Tauri v2 derives the invoke name from the function name, with no rename.
use wobu_core::kind_registry as registry;
use wobu_core::{Id, KindDef, Node, NodeKind, NodeSummary};
use wobu_store::{CorruptFile, Project, ProjectSummary, SaveOutcome, recent};

use crate::error::{CommandResult, WobuError};
use crate::state::{AppState, WORLD_CHANGED};

/* ── registry ─────────────────────────────────────────────────────────────── */

/// Static, and the frontend caches it forever (`staleTime: Infinity`).
#[tauri::command]
pub fn kind_registry() -> Vec<KindDef> {
    registry().to_vec()
}

/* ── project ──────────────────────────────────────────────────────────────── */

#[tauri::command]
pub fn project_create(
    app: AppHandle,
    state: State<'_, AppState>,
    parent_dir: String,
    name: String,
) -> CommandResult<ProjectSummary> {
    let project = Project::create(&PathBuf::from(parent_dir), &name)?;
    Ok(adopt(&app, &state, project))
}

#[tauri::command]
pub fn project_open(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> CommandResult<ProjectSummary> {
    let project = Project::open(&PathBuf::from(path))?;
    Ok(adopt(&app, &state, project))
}

#[tauri::command]
pub fn project_close(state: State<'_, AppState>) -> CommandResult<()> {
    state.close();
    Ok(())
}

#[tauri::command]
pub fn project_current(state: State<'_, AppState>) -> Option<ProjectSummary> {
    state.peek(|p| p.map(Project::summary))
}

#[tauri::command]
pub fn project_recent() -> Vec<ProjectSummary> {
    recent::list_summaries()
}

/// Whether the open project's folder is currently unreachable.
///
/// The `share:offline` / `share:online` events are the live signal; this is
/// for the one case events cannot cover — a webview that reloaded while
/// disconnected and so missed the event that would have raised the banner.
#[tauri::command]
pub fn share_offline(state: State<'_, AppState>) -> bool {
    state.is_offline()
}

/// Quit despite the share being away, having been told what that costs.
///
/// The window's close handler refuses the first attempt while offline; this is
/// the only way past it, and it exists so that the refusal is a warning rather
/// than a trap the user cannot get out of.
#[tauri::command]
pub fn force_quit(app: AppHandle, state: State<'_, AppState>) {
    // Drop the project first so the watcher and reconnect threads stop before
    // the process does, rather than being killed mid-reconcile.
    state.close();
    app.exit(0);
}

/// Take ownership of a just-opened project: remember it in the recents list,
/// then hand it to the state (which starts the watcher).
fn adopt(app: &AppHandle, state: &AppState, project: Project) -> ProjectSummary {
    let summary = project.summary();
    // A recents file we cannot write is an annoyance, not a failure to open —
    // the project itself is already fine.
    if let Err(e) = recent::record(&summary) {
        eprintln!("wobu: could not record recent project: {e}");
    }
    state.install(app, project);
    summary
}

/* ── nodes ────────────────────────────────────────────────────────────────── */

#[tauri::command]
pub fn node_list(state: State<'_, AppState>) -> CommandResult<Vec<NodeSummary>> {
    state.with(|p| Ok(p.list_nodes()?))
}

/// Node files that are on disk and cannot be parsed.
///
/// Separate from `node_list` because a file a sync client truncated may never
/// have had a node row to attach to.
#[tauri::command]
pub fn corrupt_files(state: State<'_, AppState>) -> CommandResult<Vec<CorruptFile>> {
    state.with(|p| Ok(p.corrupt_files()?))
}

/// Re-read the folder now, rather than waiting for the watcher.
///
/// The "reload" a broken file offers: the user has fixed it in a text editor
/// or restored it from a backup and wants to know whether that worked, without
/// having to guess at the debounce.
#[tauri::command]
pub fn project_reload(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    state.with(|p| {
        p.reconcile()?;
        Ok(())
    })?;
    let _ = app.emit(WORLD_CHANGED, ());
    Ok(())
}

#[tauri::command]
pub fn node_get(state: State<'_, AppState>, id: Id) -> CommandResult<Node> {
    state.with(|p| Ok(p.get_node(id)?))
}

#[tauri::command]
pub fn node_create(
    state: State<'_, AppState>,
    kind: NodeKind,
    name: String,
    parent_id: Option<Id>,
) -> CommandResult<Node> {
    state.with(|p| Ok(p.create_node(kind, &name, parent_id)?))
}

#[tauri::command]
pub fn node_upsert(state: State<'_, AppState>, node: Node) -> CommandResult<Node> {
    state.with(|p| match p.save_node(node)? {
        SaveOutcome::Saved(saved) => Ok(*saved),
        SaveOutcome::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    })
}

#[tauri::command]
pub fn node_delete(state: State<'_, AppState>, id: Id) -> CommandResult<()> {
    state.with(|p| Ok(p.delete_node(id)?))
}

#[tauri::command]
pub fn node_move(
    state: State<'_, AppState>,
    id: Id,
    new_parent_id: Option<Id>,
) -> CommandResult<()> {
    state.with(|p| Ok(p.move_node(id, new_parent_id)?))
}

/// The bridge contract, pinned.
///
/// `src/lib/api.ts` hand-writes the TypeScript for every payload here. Nothing
/// generates one from the other, so the only thing stopping them drifting is a
/// test that feeds the Rust side exactly the JSON the webview sends. These use
/// literal JSON rather than round-tripping a Rust value on purpose — a
/// round-trip would agree with itself no matter what the frontend believes.
#[cfg(test)]
mod bridge {
    use super::*;

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
        assert_eq!(description.list("materials"), Some(&["ashglass".to_string(), "bone".to_string()][..]));
    }

    #[test]
    fn a_node_serialises_back_under_the_keys_the_webview_reads() {
        let node: Node = serde_json::from_str(NODE_FROM_THE_WEBVIEW).unwrap();
        let json = serde_json::to_value(&node).unwrap();

        // The camelCase ones are the ones that would break silently: serde
        // renames them, TypeScript does not know that, and a missing key
        // arrives in the UI as `undefined` rather than as an error.
        for key in ["parentId", "notesRaw", "descriptionState", "coverAssetId", "createdAt", "updatedAt"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from the node payload");
        }
        assert_eq!(json["links"][0]["toId"], "01ARZ3NDEKTSV4RRFFQ69G5FAW");
        assert_eq!(json["description"]["sections"]["materials"]["type"], "list");
    }

    #[test]
    fn the_kind_registry_matches_the_kinddef_interface() {
        let json = serde_json::to_value(kind_registry()).unwrap();
        let first = &json[0];

        for key in ["kind", "label", "plural", "icon", "color", "layer", "dir", "nests", "singleton", "sections", "defaultLinkRoles"] {
            assert!(first.get(key).is_some(), "`{key}` is missing from KindDef");
        }
        for key in ["key", "label", "valueKind"] {
            assert!(first["sections"][0].get(key).is_some(), "`{key}` is missing from SectionDef");
        }
        // The union in `api.ts` is snake_case; the enum has to agree.
        let kinds: Vec<&str> = json.as_array().unwrap().iter().map(|d| d["kind"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"style_guide"), "got {kinds:?}");
        assert!(kinds.contains(&"world_bible"), "got {kinds:?}");
    }
}
