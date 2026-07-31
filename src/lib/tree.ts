import type { KindDef, NodeKind, NodeSummary } from './api'

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

export function indexNodes(nodes: NodeSummary[] | undefined): Map<string, NodeSummary> {
  const m = new Map<string, NodeSummary>()
  for (const n of nodes ?? []) m.set(n.id, n)
  return m
}
