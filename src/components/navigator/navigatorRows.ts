import type { NodeKind } from '../../lib/api'
import { filterTree, type KindGroup, type TreeNode } from '../../lib/tree'

export type NavigatorListRow =
  | { type: 'group'; key: string; group: KindGroup; open: boolean }
  | { type: 'node'; key: string; tree: TreeNode; open: boolean; hasChildren: boolean }

export interface NavigatorBuildStats {
  filteredGroups: number
}

export function buildNavigatorRows(
  groups: KindGroup[],
  filter: string,
  closedGroups: Record<string, true>,
  collapsedNodes: Record<string, true>,
  stats?: NavigatorBuildStats,
): { rows: NavigatorListRow[]; hasMatches: boolean } {
  const rows: NavigatorListRow[] = []
  const filtering = filter.trim().length > 0
  let hasMatches = false

  for (const group of groups) {
    if (stats) stats.filteredGroups += 1
    const roots = filterTree(group.roots, filter)
    if (filtering && roots.length === 0) continue
    hasMatches ||= roots.length > 0
    const open = !closedGroups[group.kind] || filtering
    rows.push({ type: 'group', key: `group:${group.kind}`, group, open })
    if (open) flattenNodes(rows, roots, filtering, collapsedNodes)
  }

  return { rows, hasMatches }
}

function flattenNodes(
  rows: NavigatorListRow[],
  trees: TreeNode[],
  forceOpen: boolean,
  collapsedNodes: Record<string, true>,
) {
  for (const tree of trees) {
    const hasChildren = tree.children.length > 0
    const open = forceOpen || !collapsedNodes[tree.node.id]
    rows.push({ type: 'node', key: tree.node.id, tree, open, hasChildren })
    if (hasChildren && open) flattenNodes(rows, tree.children, forceOpen, collapsedNodes)
  }
}

export function groupDropId(kind: NodeKind): string {
  return `group:${kind}`
}
