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

/**
 * The navigator's filter predicate: name and summary, nothing deeper.
 *
 * One function so the tree, the favourites list and the recent list all agree
 * on what "matches" means. Notes and descriptions are the palette's job
 * (`lib/search.ts`), for the reason spelled out where the filter box is drawn.
 */
export function matchesQuery(node: NodeSummary, needle: string): boolean {
  return node.name.toLowerCase().includes(needle) || node.summary.toLowerCase().includes(needle)
}

/** Keeps a node when it matches, or when a descendant matches. */
export function filterTree(roots: TreeNode[], q: string): TreeNode[] {
  const needle = q.trim().toLowerCase()
  if (!needle) return roots
  const keep = (t: TreeNode): TreeNode | null => {
    const kids = t.children.map(keep).filter((x): x is TreeNode => x !== null)
    if (!matchesQuery(t.node, needle) && kids.length === 0) return null
    return { ...t, children: kids }
  }
  return roots.map(keep).filter((x): x is TreeNode => x !== null)
}

/**
 * A node drawn outside its own branch — a favourite, a recent — as a tree row.
 *
 * Cached against the summary object rather than rebuilt, because the row that
 * renders it is memoized: a fresh `{ node, depth, children }` on every keystroke
 * would re-render every shortcut row in the navigator for no visible change.
 * The cache is a WeakMap, so a node the query layer has replaced is collected
 * with its wrapper.
 */
const FLAT = new WeakMap<NodeSummary, TreeNode>()

export function flatTree(node: NodeSummary): TreeNode {
  const known = FLAT.get(node)
  if (known) return known
  const made: TreeNode = { node, depth: 0, children: [] }
  FLAT.set(node, made)
  return made
}

/**
 * Past this many roots a kind group runs several screens deep, and scrolling is
 * the only way through it.
 */
export const BUCKET_MIN_ROOTS = 48

/**
 * The most headings an index will ever show.
 *
 * A per-letter index does not scale in the direction you want: two hundred
 * characters produce twenty-six headings, which is a shorter list than two
 * hundred names but a far less informative one. Capping the headings and
 * widening them into runs — `A–E`, `F–K` — keeps the index itself readable at
 * a glance whatever the group weighs.
 */
export const BUCKET_MAX = 12

/** Below this, a heading of its own costs the reader more than it saves. */
export const BUCKET_MIN_SIZE = 20

/** One heading of the alphabetical index inside an oversized kind group. */
export interface AlphaBucket {
  /** `M` for a single letter, `A–E` for a run of them. */
  label: string
  /** First letter of the run; the stable half of the heading's identity. */
  from: string
  /** Last letter of the run. */
  to: string
  roots: TreeNode[]
  /** Everything filed under the heading, nested descendants included. */
  count: number
}

/**
 * The letter a name files under: accents folded, non-letters collected as `#`.
 *
 * Folded rather than split so that `Élan` sits with `E` instead of gaining an
 * index entry of its own — the point of the index is fewer headings, and a
 * reader looking for Élan looks under E. Names that begin with a digit, a quote
 * or a symbol share `#`, which sorts last. The ASCII path is spelled out
 * because this runs once per root on every keystroke that changes the filter,
 * and `normalize()` is not cheap enough to pay for a name starting with `K`.
 */
export function bucketLetter(name: string): string {
  const first = name.trim().charAt(0)
  if (first >= 'A' && first <= 'Z') return first
  if (first >= 'a' && first <= 'z') return first.toUpperCase()
  const folded = first.normalize('NFD').replace(/\p{M}/gu, '').toUpperCase()
  return /^\p{L}/u.test(folded) ? folded : '#'
}

/** Alphabetical, with the symbol bucket last. */
function compareLetters(a: string, b: string): number {
  if (a === b) return 0
  if (a === '#') return 1
  if (b === '#') return -1
  return a.localeCompare(b)
}

/**
 * Split a group's roots into an alphabetical index, or `null` to leave it flat.
 *
 * Returning `null` is the interesting half: the navigator only draws headings
 * when they shorten something, so a short group — or a long one whose names all
 * begin with the same letter, which is what an imported batch or a naming
 * convention looks like — is rendered exactly as it was before.
 */
export function bucketRoots(roots: TreeNode[]): AlphaBucket[] | null {
  if (roots.length < BUCKET_MIN_ROOTS) return null

  const byLetter = new Map<string, TreeNode[]>()
  for (const root of roots) {
    const letter = bucketLetter(root.node.name)
    const list = byLetter.get(letter)
    if (list) list.push(root)
    else byLetter.set(letter, [root])
  }
  if (byLetter.size < 2) return null

  const letters = [...byLetter.keys()].sort(compareLetters)
  const target = Math.max(BUCKET_MIN_SIZE, Math.ceil(roots.length / BUCKET_MAX))
  const buckets: AlphaBucket[] = []
  let current: AlphaBucket | null = null

  for (const letter of letters) {
    const list = byLetter.get(letter) as TreeNode[]
    if (!current) {
      current = { label: letter, from: letter, to: letter, roots: [], count: 0 }
      buckets.push(current)
    }
    current.to = letter
    current.roots.push(...list)
    current.count += countTrees(list)
    if (current.roots.length >= target) current = null
  }

  // A trailing letter or two left over from the walk joins the run before it
  // rather than standing alone under a heading of its own.
  const last = buckets[buckets.length - 1]
  const previous = buckets[buckets.length - 2]
  if (buckets.length > 1 && last && previous && last.roots.length < BUCKET_MIN_SIZE) {
    previous.to = last.to
    previous.roots.push(...last.roots)
    previous.count += last.count
    buckets.pop()
  }
  if (buckets.length < 2) return null

  for (const bucket of buckets) {
    bucket.label = bucket.from === bucket.to ? bucket.from : `${bucket.from}–${bucket.to}`
  }
  return buckets
}

/** The heading a letter falls under, by its `from` — the key rows are drawn with. */
export function bucketOf(buckets: AlphaBucket[], letter: string): string | null {
  for (const bucket of buckets) {
    if (compareLetters(letter, bucket.from) >= 0 && compareLetters(letter, bucket.to) <= 0) {
      return bucket.from
    }
  }
  return null
}

function countTrees(trees: TreeNode[]): number {
  let total = 0
  for (const tree of trees) total += 1 + countTrees(tree.children)
  return total
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
