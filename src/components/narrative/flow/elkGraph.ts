import type { ElkNode } from 'elkjs/lib/elk-api'
import type { LayoutRequest, XY } from './layout'

const ROOT_OPTIONS = {
  'elk.algorithm': 'layered',
  'elk.direction': 'DOWN',
  'elk.hierarchyHandling': 'INCLUDE_CHILDREN',
  'elk.layered.considerModelOrder.strategy': 'NODES_AND_EDGES',
  'elk.layered.crossingMinimization.forceNodeModelOrder': 'true',
  'elk.layered.spacing.nodeNodeBetweenLayers': '64',
  'elk.spacing.nodeNode': '48',
  'elk.padding': '[top=44,left=24,bottom=24,right=24]',
}

/** Preserve grouped subgraphs and authored branch order in the upstream worker request. */
export function elkGraph(request: LayoutRequest): ElkNode {
  const children = new Map<string, ElkNode[]>()
  for (const group of request.groups) children.set(group, [])
  const roots: ElkNode[] = []
  for (const node of request.nodes) {
    const box: ElkNode = { id: node.id, width: node.width, height: node.height }
    const siblings = node.groupId === null ? null : children.get(node.groupId)
    if (siblings) siblings.push(box)
    else roots.push(box)
  }
  for (const group of request.groups) roots.push({ id: group, children: children.get(group) ?? [] })
  return {
    id: 'root',
    layoutOptions: ROOT_OPTIONS,
    children: roots,
    edges: request.edges.map((edge) => ({
      id: edge.id,
      sources: [edge.source],
      targets: [edge.target],
    })),
  }
}

export function elkPositions(graph: ElkNode): Record<string, XY> {
  const positions: Record<string, XY> = {}
  const visit = (node: ElkNode, offsetX: number, offsetY: number) => {
    for (const child of node.children ?? []) {
      const x = offsetX + (child.x ?? 0)
      const y = offsetY + (child.y ?? 0)
      positions[child.id] = { x, y }
      visit(child, x, y)
    }
  }
  visit(graph, 0, 0)
  return positions
}
