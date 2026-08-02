import type { LinkEdge, LinkRole, NodeKind, NodeSummary } from '../../lib/api'
import type { KindIndex } from '../../lib/kinds'

export const CARD_W = 112
export const CARD_H = 46
const COL_GAP = 14
const ROW_GAP = 16
export const PAD_X = 10
const GROUP_HEAD = 30
const GROUP_GAP = 26
const COLS = 2

export interface GraphNode extends NodeSummary {
  x: number
  y: number
}

export interface GraphEdge {
  key: string
  fromId: string
  toId: string
  kind: 'parent' | 'influence'
  role: LinkRole | null
  weight: number
  enabled: boolean
}

export interface RelationshipLayout {
  nodes: GraphNode[]
  edges: GraphEdge[]
  width: number
  height: number
  dangling: number
  groups: Array<{ kind: NodeKind; label: string; y: number }>
}

/** A deterministic, kind-banded layout with no editable drag state. */
export function layoutRelationships(
  nodes: NodeSummary[],
  explicit: LinkEdge[],
  kinds: KindIndex,
): RelationshipLayout {
  const orderedKinds = [...kinds.keys()]
  for (const node of nodes) if (!orderedKinds.includes(node.kind)) orderedKinds.push(node.kind)

  const placed: GraphNode[] = []
  const groups: RelationshipLayout['groups'] = []
  let y = 14
  for (const kind of orderedKinds) {
    const members = nodes
      .filter((node) => node.kind === kind)
      .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }))
    if (!members.length) continue
    groups.push({ kind, label: kinds.get(kind)?.plural ?? kind.replaceAll('_', ' '), y })
    members.forEach((node, index) => {
      placed.push({
        ...node,
        x: PAD_X + (index % COLS) * (CARD_W + COL_GAP),
        y: y + GROUP_HEAD + Math.floor(index / COLS) * (CARD_H + ROW_GAP),
      })
    })
    y += GROUP_HEAD + Math.ceil(members.length / COLS) * (CARD_H + ROW_GAP) + GROUP_GAP
  }

  const ids = new Set(nodes.map((node) => node.id))
  const parentEdges: GraphEdge[] = nodes.flatMap((node) =>
    node.parentId && ids.has(node.parentId)
      ? [
          {
            key: `parent:${node.id}:${node.parentId}`,
            fromId: node.id,
            toId: node.parentId,
            kind: 'parent' as const,
            role: null,
            weight: 1,
            enabled: true,
          },
        ]
      : [],
  )
  let dangling = 0
  const influenceEdges: GraphEdge[] = []
  explicit.forEach((edge, index) => {
    if (!ids.has(edge.fromId) || !ids.has(edge.toId)) {
      dangling += 1
      return
    }
    influenceEdges.push({
      key: `link:${edge.fromId}:${edge.toId}:${edge.role}:${index}`,
      fromId: edge.fromId,
      toId: edge.toId,
      kind: 'influence',
      role: edge.role,
      weight: edge.weight,
      enabled: edge.enabled,
    })
  })

  return {
    nodes: placed,
    edges: [...parentEdges, ...influenceEdges],
    width: PAD_X * 2 + CARD_W * COLS + COL_GAP * (COLS - 1),
    height: Math.max(180, y),
    dangling,
    groups,
  }
}
