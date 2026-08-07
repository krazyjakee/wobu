//! What the app can say about its own state.
//!
//! Conflicts, peers, the index, the build and the log. Read-only apart from
//! resolving a conflict and rebuilding the index — both of which are the user
//! answering a question the app has already put on screen.

use tauri::{AppHandle, Emitter, State};
// Aliased because the command below has to *be* called `kind_registry` —
// Tauri v2 derives the invoke name from the function name, with no rename.
use wobu_core::Id;
use wobu_store::{Conflict, Keep, Peer, Resolved};

use crate::diag;
use crate::error::{Code, CommandResult, WobuError};
use crate::state::{AppState, WORLD_CHANGED};

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
        diag::info(format!("conflict resolved at {rel_path} keeping {keep:?}"));
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
    LogInfo { exists: path.is_file(), path: path.to_string_lossy().into_owned(), level, size_bytes }
}

#[tauri::command]
pub fn log_set_level(level: diag::Level) {
    if let Some(d) = diag::global() {
        d.set_level(level);
        // Recorded at error so it lands whatever the new level is — when
        // reading a log the first question is always "was it even on?".
        diag::error(format!("log level set to {level:?}"));
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
    std::fs::create_dir_all(&dir).map_err(|e| {
        WobuError::new(Code::Io, "Could not open the log folder.").with_detail(e.to_string())
    })?;

    let path = diag::global().map(|d| d.path()).unwrap_or_else(|| dir.join("wobu.log"));
    let target = if path.is_file() { path } else { dir };

    tauri_plugin_opener::reveal_item_in_dir(&target).map_err(|e| {
        WobuError::new(Code::Io, "Could not show the log in the file manager.")
            .with_detail(e.to_string())
    })
}

/* ── provider keys ────────────────────────────────────────────────────────── */
