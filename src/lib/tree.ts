import type { KindDef, LinkEdge, NodeKind, NodeSummary } from './api'

export interface TreeNode {
  node: NodeSummary
  depth: number
  children: TreeNode[]
}

export interface KindGroup {
  kind: NodeKind
  def: KindDef | undefined
  roots: TreeNode[]
  count: number
}

/** Nesting is within a kind (docs/02-data-model.md), so groups are built per kind. */
export function buildGroups(
  nodes: NodeSummary[],
  kindOrder: NodeKind[],
  defs: Map<NodeKind, KindDef>,
): KindGroup[] {
  const byKind = new Map<NodeKind, NodeSummary[]>()
  for (const n of nodes) {
    const list = byKind.get(n.kind)
    if (list) list.push(n)
    else byKind.set(n.kind, [n])
  }

  const kinds: NodeKind[] = [...kindOrder]
  for (const k of byKind.keys()) if (!kinds.includes(k)) kinds.push(k)

  const groups: KindGroup[] = []
  for (const kind of kinds) {
    const list = byKind.get(kind)
    if (!list || list.length === 0) continue
    groups.push({ kind, def: defs.get(kind), roots: nest(list), count: list.length })
  }
  return groups
}

function nest(list: NodeSummary[]): TreeNode[] {
  const byId = new Map<string, TreeNode>()
  for (const n of list) byId.set(n.id, { node: n, depth: 0, children: [] })

  const roots: TreeNode[] = []
  for (const t of byId.values()) {
    const pid = t.node.parentId
    const parent = pid ? byId.get(pid) : undefined
    // A parent outside this kind's slice is treated as a root, so nothing
    // silently disappears from the tree when data is odd.
    if (parent && parent !== t) parent.children.push(t)
    else roots.push(t)
  }

  const cmp = (a: TreeNode, b: TreeNode) =>
    a.node.name.localeCompare(b.node.name, undefined, { sensitivity: 'base' })
  const walk = (nodes: TreeNode[], depth: number) => {
    nodes.sort(cmp)
    for (const n of nodes) {
      n.depth = depth
      walk(n.children, depth + 1)
    }
  }
  walk(roots, 0)
  return roots
}

/** Keeps a node when it matches, or when a descendant matches. */
export function filterTree(roots: TreeNode[], q: string): TreeNode[] {
  const needle = q.trim().toLowerCase()
  if (!needle) return roots
  const keep = (t: TreeNode): TreeNode | null => {
    const kids = t.children.map(keep).filter((x): x is TreeNode => x !== null)
    const hit =
      t.node.name.toLowerCase().includes(needle) || t.node.summary.toLowerCase().includes(needle)
    if (!hit && kids.length === 0) return null
    return { ...t, children: kids }
  }
  return roots.map(keep).filter((x): x is TreeNode => x !== null)
}

/** Chain of same-kind ancestors, outermost first — the influence breadcrumb. */
export function ancestorsOf(id: string, byId: Map<string, NodeSummary>): NodeSummary[] {
  const chain: NodeSummary[] = []
  const seen = new Set<string>([id])
  let cur = byId.get(id)?.parentId ?? null
  while (cur && !seen.has(cur)) {
    const n = byId.get(cur)
    if (!n) break
    chain.unshift(n)
    seen.add(cur)
    cur = n.parentId
  }
  return chain
}

/** Every id beneath `id` (used to reject a drop onto one's own descendant). */
export function descendantsOf(id: string, nodes: NodeSummary[]): Set<string> {
  const kids = new Map<string, string[]>()
  for (const n of nodes) {
    if (!n.parentId) continue
    const list = kids.get(n.parentId)
    if (list) list.push(n.id)
    else kids.set(n.parentId, [n.id])
  }
  const out = new Set<string>()
  const stack = [...(kids.get(id) ?? [])]
  while (stack.length) {
    const cur = stack.pop() as string
    if (out.has(cur)) continue
    out.add(cur)
    stack.push(...(kids.get(cur) ?? []))
  }
  return out
}

/**
 * Every other subject whose resolved influence stack can contain `sourceId`.
 *
 * This mirrors the backend's project-wide inverse walk: singleton sources
 * influence the whole project, parent edges are implicit, disabled links are
 * absent, and the first reverse hop may cross a lateral `related_to` edge while
 * later hops may not. The generation compiler remains authoritative; it would
 * be wasteful to resolve a stack once per node just to show a consequence
 * sentence on a tile.
 */
export function influenceDependentsOf(
  sourceId: string,
  nodes: NodeSummary[],
  links: LinkEdge[],
): NodeSummary[] {
  const byId = indexNodes(nodes)
  if (!byId.has(sourceId)) return []

  const roots = [
    nodes.find((node) => node.kind === 'style_guide'),
    nodes.find((node) => node.kind === 'world_bible'),
  ].filter((node): node is NodeSummary => node !== undefined)

  // A singleton is seeded into every subject's stack rather than reached by
  // an edge, so its reverse adjacency is the whole project.
  if (roots.some((root) => root.id === sourceId)) {
    return nodes.filter((node) => node.id !== sourceId)
  }

  type Referrer = { id: string; lateral: boolean }
  const referrers = new Map<string, Referrer[]>()
  const addReferrer = (toId: string, referrer: Referrer) => {
    if (!byId.has(toId) || !byId.has(referrer.id)) return
    const entries = referrers.get(toId)
    if (entries) entries.push(referrer)
    else referrers.set(toId, [referrer])
  }

  // parentId is an implicit, always-enabled influence edge from child to
  // parent, so the child is a reverse referrer of its parent.
  for (const node of nodes) {
    if (node.parentId) addReferrer(node.parentId, { id: node.id, lateral: false })
  }
  for (const edge of links) {
    if (!edge.enabled) continue
    addReferrer(edge.toId, { id: edge.fromId, lateral: edge.role === 'related_to' })
  }

  const visited = new Set<string>([sourceId])
  const queue = [sourceId]
  for (let cursor = 0; cursor < queue.length; cursor += 1) {
    const current = queue[cursor]
    if (!current) continue

    for (const referrer of referrers.get(current) ?? []) {
      // A lateral source is included in a subject's stack but never expanded.
      // Reversed, that means any role may cross the first hop into sourceId,
      // while subsequent hops must reject related_to.
      if (cursor !== 0 && referrer.lateral) continue
      if (visited.has(referrer.id)) continue
      visited.add(referrer.id)
      queue.push(referrer.id)
    }
  }

  // Traversal order is an implementation detail; callers render in the stable
  // order supplied by the node query, as the former per-subject filter did.
  return nodes.filter((node) => node.id !== sourceId && visited.has(node.id))
}

export function indexNodes(nodes: NodeSummary[] | undefined): Map<string, NodeSummary> {
  const m = new Map<string, NodeSummary>()
  for (const n of nodes ?? []) m.set(n.id, n)
  return m
}
