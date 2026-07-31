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
use wobu_store::{
    Conflict, CorruptFile, Keep, Peer, Project, ProjectSummary, Resolved, SaveOutcome, recent,
};

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

/// Emitted while a first open is scanning. Payload is `ScanProgress`.
pub const OPEN_PROGRESS: &str = "project:open-progress";

/// Opening a project, which on a NAS is the one operation that can take
/// minutes.
///
/// `async` so the webview keeps painting, and the scan itself runs on a
/// blocking thread because it is filesystem-bound rather than await-bound. A
/// stalled mount must never present as a frozen app, which is the whole reason
/// this is not the three-line synchronous command it used to be.
///
/// Progress is emitted rather than returned: a return value arrives once, at
/// the end, which is exactly when the user no longer needs it.
#[tauri::command]
pub async fn project_open(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> CommandResult<ProjectSummary> {
    let cancel = state.begin_open();
    let emitter = app.clone();
    let root = PathBuf::from(path);

    let opened = tauri::async_runtime::spawn_blocking(move || {
        let mut last = 0u8;
        Project::open_with(&root, &cancel, &mut |p| {
            // Throttled to whole percentage points. A world of 4000 files would
            // otherwise put 4000 events through the bridge, and repainting the
            // same number is work taken from the scan itself.
            let pct = p.percent();
            if pct != last {
                last = pct;
                let _ = emitter.emit(OPEN_PROGRESS, p);
            }
        })
    })
    .await
    .map_err(|e| WobuError::new(Code::Internal, "The scan thread stopped unexpectedly.")
        .with_detail(e.to_string()))?;

    state.finish_open();
    Ok(adopt(&app, &state, opened?))
}

/// Stop a scan in progress.
///
/// A no-op when nothing is open — the user can press this at the moment the
/// scan finishes, and that race must not be an error.
#[tauri::command]
pub fn project_open_cancel(state: State<'_, AppState>) {
    state.cancel_open();
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

/// Full-text search over names, summaries, notes and descriptions.
///
/// Returns ids in rank order rather than whole nodes: the caller already holds
/// every summary from `node_list`, so sending them back would duplicate the
/// world across the bridge on every keystroke. The order is the part that
/// cannot be reconstructed on the other side.
#[tauri::command]
pub fn node_search(state: State<'_, AppState>, query: String) -> CommandResult<Vec<Id>> {
    state.with(|p| Ok(p.index().search(&query)?))
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

/* ── conflicts ────────────────────────────────────────────────────────────── */

/// Unresolved conflict siblings in the open project.
///
/// Read from the folder on every call rather than pushed. A sibling can be
/// parked by a *different machine*, so there is no event on this side that
/// could keep a cached list honest — `world:changed` invalidates this alongside
/// the node list, and a failing save invalidates it directly.
#[tauri::command]
pub fn conflicts(state: State<'_, AppState>) -> CommandResult<Vec<Conflict>> {
    state.with(|p| Ok(p.conflicts()?))
}

/// Carry out the user's decision about one conflict sibling.
///
/// The only command in Wobu that deletes a file the user did not name as a
/// node, which is why `expected_hash` is required rather than optional: it is
/// the hash of the node file as the card rendered it, and a mismatch means the
/// diff the user answered is not the one on disk. That comes back as
/// `{ outcome: "stale" }` with both files untouched, and the card redraws.
#[tauri::command]
pub fn conflict_resolve(
    app: AppHandle,
    state: State<'_, AppState>,
    rel_path: String,
    keep: Keep,
    expected_hash: String,
) -> CommandResult<Resolved> {
    let outcome = state.with(|p| Ok(p.resolve_conflict(&rel_path, keep, &expected_hash)?))?;
    if matches!(outcome, Resolved::Done) {
        diag::info(&format!("conflict resolved at {rel_path} keeping {keep:?}"));
        // Only on a real change. A stale or raced resolution left the folder
        // exactly as it was, and waking the whole world for it would refetch
        // the node the user is mid-decision about.
        let _ = app.emit(WORLD_CHANGED, ());
    }
    Ok(outcome)
}

/* ── presence ─────────────────────────────────────────────────────────────── */

/// Who else has this project open.
///
/// Polled rather than pushed. The answer only changes at human speed, and an
/// event per beat per peer would be traffic on the very share whose latency the
/// presence is there to explain.
///
/// Never fails, and is empty when no project is open — advisory information
/// arriving as an error would put a toast on screen for a poll that merely
/// raced a close.
#[tauri::command]
pub fn presence_peers(state: State<'_, AppState>) -> Vec<Peer> {
    state.peers()
}

/// Record which nodes this session has open, for everyone else's benefit.
///
/// The whole list rather than an add/remove pair: the frontend already knows
/// which nodes are open, and a delta protocol drifts the first time one closes
/// during a disconnection and then stays wrong for the rest of the session.
///
/// Advisory in the strongest sense. No write path reads this, and naming a node
/// here neither reserves it nor stops anyone — including the person who named
/// it — from saving or deleting it.
#[tauri::command]
pub fn presence_editing(state: State<'_, AppState>, node_ids: Vec<Id>) {
    state.set_editing(node_ids);
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
            "relPath", "nodeRelPath", "nodeId", "nodeName", "user", "savedAt", "mine", "parked",
            "current", "currentHash",
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
