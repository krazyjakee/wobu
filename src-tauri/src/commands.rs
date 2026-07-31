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

use crate::diag;
use crate::error::{Code, CommandResult, WobuError};
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
        diag::error(&format!("could not record recent project: {e}"));
    }
    // The path is the single most useful line in a bug report: it says whether
    // the world was on a share, and `redact::scrub` leaves it intact because a
    // filesystem path is not a credential.
    diag::info(&format!("opened project {} at {}", summary.id, summary.path));
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

/* ── storage and about ────────────────────────────────────────────────────── */

/// The local index for the open project.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    path: String,
    size_bytes: u64,
    /// Zero until something has been indexed — a project that failed to scan
    /// looks identical to one that has not been opened otherwise.
    node_count: u64,
}

#[tauri::command]
pub fn index_info(state: State<'_, AppState>) -> CommandResult<IndexInfo> {
    state.with(|p| {
        let path = p.index_path();
        Ok(IndexInfo {
            size_bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            path: path.to_string_lossy().into_owned(),
            node_count: p.list_nodes()?.len() as u64,
        })
    })
}

/// Throw the index away and rebuild it from the Markdown.
///
/// Safe to expose because the index holds no canonical data. It is the support
/// answer to "the navigator is showing something that isn't there".
#[tauri::command]
pub fn index_rebuild(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    state.with(|p| {
        p.rebuild_index()?;
        Ok(())
    })?;
    diag::info("index rebuilt on request");
    let _ = app.emit(WORLD_CHANGED, ());
    Ok(())
}

/// Version numbers worth quoting in a bug report.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutInfo {
    app_version: String,
    /// The on-disk format of the project folder. The number that decides
    /// whether a newer Wobu's world can be opened here at all.
    project_schema_version: u32,
    /// The local index layout. Bumping it silently rebuilds, so a user seeing
    /// a long pause after an update is seeing this change.
    index_schema_version: u32,
    log_path: String,
}

#[tauri::command]
pub fn about_info() -> AboutInfo {
    AboutInfo {
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        project_schema_version: wobu_core::SCHEMA_VERSION,
        index_schema_version: wobu_store::index::INDEX_VERSION,
        log_path: diag::global()
            .map(|d| d.path())
            .unwrap_or_else(|| diag::dir().join("wobu.log"))
            .to_string_lossy()
            .into_owned(),
    }
}

/* ── diagnostics ──────────────────────────────────────────────────────────── */

/// What Settings needs to describe the log without reading it.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogInfo {
    /// Absolute, and shown to the user — they may well go and find it by hand.
    path: String,
    level: diag::Level,
    /// False until something has been recorded. The UI says so rather than
    /// offering to reveal a file that is not there.
    exists: bool,
    size_bytes: u64,
}

#[tauri::command]
pub fn log_info() -> LogInfo {
    let (path, level) = match diag::global() {
        Some(d) => (d.path(), d.level()),
        None => (diag::dir().join("wobu.log"), diag::Level::default()),
    };
    let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    LogInfo {
        exists: path.is_file(),
        path: path.to_string_lossy().into_owned(),
        level,
        size_bytes,
    }
}

#[tauri::command]
pub fn log_set_level(level: diag::Level) {
    if let Some(d) = diag::global() {
        d.set_level(level);
        // Recorded at error so it lands whatever the new level is — when
        // reading a log the first question is always "was it even on?".
        diag::error(&format!("log level set to {level:?}"));
    }
}

/// The end of the log, for showing the user what they are about to hand over.
#[tauri::command]
pub fn log_tail(lines: usize) -> String {
    diag::global().map(|d| d.tail(lines)).unwrap_or_default()
}

/// Show the file in the OS file manager, which is how it gets attached to
/// something. Falls back to the folder when nothing has been logged yet.
#[tauri::command]
pub fn log_reveal() -> CommandResult<()> {
    let dir = diag::dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| WobuError::new(Code::Io, "Could not open the log folder.").with_detail(e.to_string()))?;

    let path = diag::global().map(|d| d.path()).unwrap_or_else(|| dir.join("wobu.log"));
    let target = if path.is_file() { path } else { dir };

    tauri_plugin_opener::reveal_item_in_dir(&target).map_err(|e| {
        WobuError::new(Code::Io, "Could not show the log in the file manager.")
            .with_detail(e.to_string())
    })
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
        assert_eq!(
            serde_json::from_str::<diag::Level>("\"debug\"").unwrap(),
            diag::Level::Debug
        );
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
