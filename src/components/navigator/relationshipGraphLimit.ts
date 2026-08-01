import type { LinkEdge, NodeSummary } from '../../lib/api'

export const GRAPH_NODE_LIMIT = 500
export const GRAPH_LINK_LIMIT = 1_500

export function limitRelationshipGraph(
  nodes: NodeSummary[],
  links: LinkEdge[],
  selectedId: string | null,
  filter: string,
) {
  if (nodes.length <= GRAPH_NODE_LIMIT && links.length <= GRAPH_LINK_LIMIT) {
    return { nodes, links, limited: false }
  }

  const needle = filter.trim().toLowerCase()
  const chosen: NodeSummary[] = []
  const ids = new Set<string>()
  const add = (node: NodeSummary | undefined) => {
    if (!node || ids.has(node.id) || chosen.length >= GRAPH_NODE_LIMIT) return
    ids.add(node.id)
    chosen.push(node)
  }

  if (selectedId) add(nodes.find((node) => node.id === selectedId))
  if (needle) {
    for (const node of nodes) {
      if (node.name.toLowerCase().includes(needle) || node.summary.toLowerCase().includes(needle)) {
        add(node)
      }
    }
  }
  for (const node of nodes) add(node)

  const visibleLinks: LinkEdge[] = []
  for (const link of links) {
    if (visibleLinks.length >= GRAPH_LINK_LIMIT) break
    if (ids.has(link.fromId) && ids.has(link.toId)) visibleLinks.push(link)
  }
  return { nodes: chosen, links: visibleLinks, limited: true }
}
