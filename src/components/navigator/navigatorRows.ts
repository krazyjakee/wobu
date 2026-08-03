import type { NodeKind, NodeSummary } from '../../lib/api'
import {
  bucketRoots,
  filterTree,
  flatTree,
  matchesQuery,
  type KindGroup,
  type TreeNode,
} from '../../lib/tree'

/**
 * Where a row sits, which is not the same as which node it draws.
 *
 * The same entity can appear three times — starred at the top, visited a minute
 * ago, and in its own branch — and the three rows are not interchangeable: only
 * the tree one can be dragged to a new parent, and only the tree one is what
 * "scroll to the selection" should scroll to. The place travels with the row so
 * that neither has to be inferred from where it happened to be found.
 */
export type NavigatorPlace = 'tree' | 'favourites' | 'recent'

export type NavigatorListRow =
  | { type: 'group'; key: string; group: KindGroup; open: boolean }
  /** A section or letter heading: a title, a count and a disclosure. */
  | { type: 'band'; key: string; label: string; count: number; open: boolean; nested: boolean }
  | {
      type: 'node'
      key: string
      place: NavigatorPlace
      tree: TreeNode
      open: boolean
      hasChildren: boolean
    }

export interface NavigatorBuildStats {
  filteredGroups: number
}

export const FAVOURITES_BAND = 'section:favourites'
export const RECENT_BAND = 'section:recent'

/**
 * The size of world at which a Recent section starts earning its place.
 *
 * Below it the whole project is on screen already, and a section that repeats
 * four of the twelve rows underneath it is not a shortcut — it is the same
 * entity drawn twice, in a navigator small enough that nobody was lost.
 * Favourites has no such threshold: somebody asked for each of those rows.
 */
export const RECENT_MIN_WORLD = 30

export function bucketBand(kind: NodeKind, letter: string): string {
  return `bucket:${kind}:${letter}`
}

/**
 * Sections start open, letter buckets start closed.
 *
 * The asymmetry is the whole point of the index: a section the reader chose to
 * fill should be readable without a click, while a letter exists precisely to
 * keep nine hundred names off the screen until one is asked for.
 */
export function bandOpenByDefault(key: string): boolean {
  return !key.startsWith('bucket:')
}

export interface NavigatorRowsInput {
  groups: KindGroup[]
  filter: string
  closedGroups: Record<string, true>
  collapsedNodes: Record<string, true>
  bands: Record<string, boolean>
  /** Starred nodes, already in the order the section should draw them. */
  favourites?: NodeSummary[]
  /** Recently opened nodes, most recent first. */
  recents?: NodeSummary[]
  stats?: NavigatorBuildStats
}

export interface NavigatorRows {
  rows: NavigatorListRow[]
  hasMatches: boolean
  /** Tree rows drawn — what the header counts when a filter is narrowing. */
  shown: number
}

export function buildNavigatorRows(input: NavigatorRowsInput): NavigatorRows {
  const { groups, filter, closedGroups, collapsedNodes, bands, stats } = input
  const rows: NavigatorListRow[] = []
  const needle = filter.trim().toLowerCase()
  const filtering = needle.length > 0
  let hasMatches = false
  let shown = 0

  const open = (key: string) => bands[key] ?? bandOpenByDefault(key)

  const section = (key: string, label: string, place: NavigatorPlace, nodes: NodeSummary[]) => {
    const list = filtering ? nodes.filter((node) => matchesQuery(node, needle)) : nodes
    if (list.length === 0) return
    hasMatches = true
    const isOpen = open(key) || filtering
    rows.push({ type: 'band', key, label, count: list.length, open: isOpen, nested: false })
    if (!isOpen) return
    for (const node of list) {
      rows.push({
        type: 'node',
        key: `${place}:${node.id}`,
        place,
        tree: flatTree(node),
        open: false,
        hasChildren: false,
      })
    }
  }

  section(FAVOURITES_BAND, 'Favourites', 'favourites', input.favourites ?? [])
  section(RECENT_BAND, 'Recent', 'recent', input.recents ?? [])

  for (const group of groups) {
    if (stats) stats.filteredGroups += 1
    const roots = filterTree(group.roots, filter)
    if (filtering && roots.length === 0) continue
    hasMatches ||= roots.length > 0
    const groupOpen = !closedGroups[group.kind] || filtering
    rows.push({ type: 'group', key: `group:${group.kind}`, group, open: groupOpen })
    if (!groupOpen) continue

    // Bucketed on what survived the filter, not on the whole group: narrowing
    // a thousand characters down to five must not leave the five behind an
    // index of five letters.
    const buckets = bucketRoots(roots)
    if (!buckets) {
      shown += flattenNodes(rows, roots, filtering, collapsedNodes)
      continue
    }
    for (const bucket of buckets) {
      const key = bucketBand(group.kind, bucket.from)
      const bucketOpen = filtering || open(key)
      rows.push({
        type: 'band',
        key,
        label: bucket.label,
        count: bucket.count,
        open: bucketOpen,
        nested: true,
      })
      if (bucketOpen) shown += flattenNodes(rows, bucket.roots, filtering, collapsedNodes)
    }
  }

  return { rows, hasMatches, shown }
}

function flattenNodes(
  rows: NavigatorListRow[],
  trees: TreeNode[],
  forceOpen: boolean,
  collapsedNodes: Record<string, true>,
): number {
  let count = 0
  for (const tree of trees) {
    const hasChildren = tree.children.length > 0
    const open = forceOpen || !collapsedNodes[tree.node.id]
    rows.push({ type: 'node', key: tree.node.id, place: 'tree', tree, open, hasChildren })
    count += 1
    if (hasChildren && open) count += flattenNodes(rows, tree.children, forceOpen, collapsedNodes)
  }
  return count
}

export function groupDropId(kind: NodeKind): string {
  return `group:${kind}`
}
