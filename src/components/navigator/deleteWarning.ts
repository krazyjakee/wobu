import type { NodeSummary } from '../../lib/api'
import { descendantsOf } from '../../lib/tree'

export function deleteWarning(node: NodeSummary, nodes: NodeSummary[]): string {
  const kids = descendantsOf(node.id, nodes).size
  const base = 'Its Markdown file is removed from the project folder.'
  if (kids === 0) return base
  const nested = kids === 1 ? '1 entity nests' : `${kids} entities nest`
  return `${base} ${nested} inside it; they move up to this one’s parent rather than being deleted with it.`
}
