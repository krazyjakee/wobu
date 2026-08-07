//! Creating, editing, moving and linking nodes.
//!
//! Thin by design: every rule about what a node may contain lives in
//! `wobu-core`, and every rule about how it reaches disk lives in
//! `wobu-store`. What is left here is the argument shape and the lock.

use tauri::State;
// Aliased because the command below has to *be* called `kind_registry` —
// Tauri v2 derives the invoke name from the function name, with no rename.
use wobu_core::{Id, LinkEdge, LinkRole, Node, NodeKind, NodeSummary};

use super::saved;
use crate::error::CommandResult;
use crate::state::AppState;

#[tauri::command]
pub fn node_list(state: State<'_, AppState>) -> CommandResult<Vec<NodeSummary>> {
    state.with(|p| Ok(p.list_nodes()?))
}

/// All explicit influence edges for the read-only relationship map.
/// Parent edges are already present on `NodeSummary` and are derived by the UI.
#[tauri::command]
pub fn node_links(state: State<'_, AppState>) -> CommandResult<Vec<LinkEdge>> {
    state.with(|p| Ok(p.node_links()?))
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
    state.with(|p| saved(p.save_node(node)?))
}

/// Lock or clear the entity seed without posting a stale copy of the node.
#[tauri::command]
pub fn node_seed_lock_set(
    state: State<'_, AppState>,
    node_id: Id,
    seed: Option<u64>,
) -> CommandResult<Node> {
    state.with(|project| saved(project.set_locked_seed(node_id, seed)?))
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

/// Add one explicit influence edge. Parent relationships are edited through
/// `node_move`; they are implicit, fixed at weight 1, and never represented as
/// an editable `Link`.
#[tauri::command]
pub fn node_link_add(
    state: State<'_, AppState>,
    node_id: Id,
    to_id: Id,
    role: LinkRole,
    weight: Option<f32>,
    enabled: Option<bool>,
) -> CommandResult<Node> {
    state.with(|p| saved(p.add_node_link(node_id, to_id, role, weight, enabled)?))
}

#[tauri::command]
pub fn node_link_remove(
    state: State<'_, AppState>,
    node_id: Id,
    to_id: Id,
    role: LinkRole,
) -> CommandResult<Node> {
    state.with(|p| saved(p.remove_node_link(node_id, to_id, role)?))
}

/// Change either mutable property without posting a stale copy of the other.
#[tauri::command]
pub fn node_link_update(
    state: State<'_, AppState>,
    node_id: Id,
    to_id: Id,
    role: LinkRole,
    weight: Option<f32>,
    enabled: Option<bool>,
) -> CommandResult<Node> {
    state.with(|p| saved(p.update_node_link(node_id, to_id, role, weight, enabled)?))
}

/// Every explicit edge pointing at this node. Names and kind labels stay out
/// of the payload because the webview already holds the node summaries.
#[tauri::command]
pub fn node_backlinks(state: State<'_, AppState>, id: Id) -> CommandResult<Vec<LinkEdge>> {
    state.with(|p| Ok(p.node_backlinks(id)?))
}

/* ── assets ───────────────────────────────────────────────────────────────── */
