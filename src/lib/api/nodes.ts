import { call } from './call'
import type { LinkEdge, LinkRole, NodeKind, NodeSummary, WobuNode } from './model'
/* ── domain types ─────────────────────────────────────────────────────────── */

export const nodeList = () => call<NodeSummary[]>('node_list')

/** Every explicit influence edge. Parent edges are derived from NodeSummary. */
export const nodeLinks = () => call<LinkEdge[]>('node_links')

/**
 * A node file that is on disk and cannot be read — a sync client copied it
 * half-written, most likely, leaving truncated YAML frontmatter.
 */
export interface CorruptFile {
  /** Project-relative, `/`-separated. Never absolute. */
  relPath: string
  /** Set when the index still remembers the entity this file used to be. */
  nodeId: string | null
  /** The parser's own words — the only thing that says what to fix. */
  error: string
  /** When it was first seen broken, not when it was last scanned. */
  detectedAt: string
}

export const corruptFiles = () => call<CorruptFile[]>('corrupt_files')

/** Re-read the folder now instead of waiting for the watcher's debounce. */
export const projectReload = () => call<void>('project_reload')

/**
 * Full-text search over names, summaries, notes and descriptions.
 *
 * Returns ids in rank order, not nodes: the caller already holds every summary
 * from `node_list`, and sending them back would duplicate the world across the
 * bridge on every keystroke. Rank order is the part that cannot be
 * reconstructed on this side.
 */
export const nodeSearch = (query: string) => call<string[]>('node_search', { query })

export const nodeGet = (id: string) => call<WobuNode>('node_get', { id })

export const nodeCreate = (kind: NodeKind, name: string, parentId: string | null) =>
  call<WobuNode>('node_create', { kind, name, parentId })

export const nodeUpsert = (node: WobuNode) => call<WobuNode>('node_upsert', { node })

/** Set the node-persisted identity seed, or null to clear it. */
export const nodeSeedLockSet = (nodeId: string, seed: number | null) =>
  call<WobuNode>('node_seed_lock_set', { nodeId, seed })

export const nodeDelete = (id: string) => call<void>('node_delete', { id })

export const nodeMove = (id: string, newParentId: string | null) =>
  call<void>('node_move', { id, newParentId })

/** Add an explicit influence edge. The backend also enforces the kind registry's roles. */
export const nodeLinkAdd = (
  nodeId: string,
  toId: string,
  role: LinkRole,
  options: { weight?: number; enabled?: boolean } = {},
) => call<WobuNode>('node_link_add', { nodeId, toId, role, ...options })

/** Remove one `(target, role)` edge; the target node itself is untouched. */
export const nodeLinkRemove = (nodeId: string, toId: string, role: LinkRole) =>
  call<WobuNode>('node_link_remove', { nodeId, toId, role })

/** Re-weight or mute an edge while leaving omitted properties unchanged. */
export const nodeLinkUpdate = (
  nodeId: string,
  toId: string,
  role: LinkRole,
  patch: { weight?: number; enabled?: boolean },
) => call<WobuNode>('node_link_update', { nodeId, toId, role, ...patch })

/** Every explicit link whose target is `id`. */
export const nodeBacklinks = (id: string) => call<LinkEdge[]>('node_backlinks', { id })

/* ── assets ───────────────────────────────────────────────────────────────── */
