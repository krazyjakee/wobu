import { useMemo, useState } from 'react'
import type { CSSProperties } from 'react'
import type { LinkEdge, LinkRole, NodeSummary } from '../../lib/api'
import { colorFor, labelFor, spriteFor, type KindIndex } from '../../lib/kinds'
import { BOARD_ASSET_MIME } from '../../lib/board'
import { Icon } from '../Icon'
import { GRAPH_LINK_LIMIT, limitRelationshipGraph } from './relationshipGraphLimit'
import {
  CARD_H,
  CARD_W,
  PAD_X,
  layoutRelationships,
  type GraphEdge,
  type GraphNode,
} from './relationshipLayout'

const ROLE_LABEL: Record<LinkRole, string> = {
  species_of: 'Species',
  member_of: 'Member of',
  located_in: 'Located in',
  styled_by: 'Styled by',
  related_to: 'Related to',
}

export function RelationshipGraph({
  nodes,
  links,
  kinds,
  selectedId,
  filter,
  loading,
  error,
  onSelect,
  readOnly = false,
  onAssetDrop,
}: {
  nodes: NodeSummary[]
  links: LinkEdge[]
  kinds: KindIndex
  selectedId: string | null
  filter: string
  loading: boolean
  error: string | null
  onSelect: (id: string) => void
  readOnly?: boolean
  onAssetDrop?: (assetId: string, nodeId: string) => void
}) {
  const [assetDropId, setAssetDropId] = useState<string | null>(null)
  const subset = useMemo(
    () => limitRelationshipGraph(nodes, links, selectedId, filter),
    [filter, links, nodes, selectedId],
  )
  const layout = useMemo(
    () => layoutRelationships(subset.nodes, subset.links, kinds),
    [kinds, subset.links, subset.nodes],
  )
  const byId = useMemo(() => new Map(layout.nodes.map((node) => [node.id, node])), [layout.nodes])
  const connected = useMemo(() => {
    const ids = new Set<string>()
    if (!selectedId) return ids
    ids.add(selectedId)
    for (const edge of layout.edges) {
      if (edge.fromId === selectedId) ids.add(edge.toId)
      if (edge.toId === selectedId) ids.add(edge.fromId)
    }
    return ids
  }, [layout.edges, selectedId])
  const needle = filter.trim().toLowerCase()

  return (
    <section className="relationship-graph" aria-label="Relationship graph">
      <div className="graph-explainer">
        <strong>{onAssetDrop ? 'Board targets' : 'Read-only map'}</strong>
        <span>
          {onAssetDrop
            ? 'Select a node, or drop a board image on one to attach it.'
            : 'Select a node to open it. Edit links in Relations.'}
        </span>
      </div>
      <div className="graph-legend" aria-label="Relationship legend">
        <span>
          <i className="is-parent" /> Parent
        </span>
        <span>
          <i className="is-influence" /> Influence
        </span>
        <span>
          <i className="is-muted" /> Muted
        </span>
      </div>

      {loading && <p className="nav-note">Reading relationships…</p>}
      {error && <p className="nav-note">Could not read relationships — {error}</p>}
      {!loading && !error && nodes.length === 0 && (
        <p className="nav-note">There are no nodes to map yet.</p>
      )}

      {!loading && !error && nodes.length > 0 && (
        <div className="graph-scroll">
          <div
            className={`graph-canvas${selectedId ? ' has-selection' : ''}`}
            style={{ width: layout.width, height: layout.height }}
          >
            <svg
              width={layout.width}
              height={layout.height}
              role="img"
              aria-label={`${layout.nodes.length} nodes and ${layout.edges.length} relationships`}
            >
              <defs>
                <marker
                  id="graph-arrow"
                  viewBox="0 0 8 8"
                  refX="7"
                  refY="4"
                  markerWidth="5"
                  markerHeight="5"
                  orient="auto-start-reverse"
                >
                  <path d="M 0 0 L 8 4 L 0 8 z" />
                </marker>
              </defs>
              {layout.groups.map((group) => (
                <g key={group.kind} className="graph-group-label">
                  <text x={PAD_X} y={group.y + 13}>
                    {group.label}
                  </text>
                  <line x1={PAD_X} x2={layout.width - PAD_X} y1={group.y + 20} y2={group.y + 20} />
                </g>
              ))}
              {layout.edges.map((edge, index) => {
                const from = byId.get(edge.fromId)
                const to = byId.get(edge.toId)
                if (!from || !to) return null
                const active = !selectedId || edge.fromId === selectedId || edge.toId === selectedId
                const cls = [
                  'graph-edge',
                  edge.kind === 'parent' ? 'is-parent' : 'is-influence',
                  edge.enabled ? '' : 'is-muted',
                  active ? 'is-connected' : '',
                ]
                  .filter(Boolean)
                  .join(' ')
                return (
                  <path
                    key={edge.key}
                    className={cls}
                    d={edgePath(from, to, index)}
                    markerEnd="url(#graph-arrow)"
                  >
                    <title>{edgeTitle(edge, from, to)}</title>
                  </path>
                )
              })}
            </svg>

            {layout.nodes.map((node) => {
              const def = kinds.get(node.kind)
              const matches =
                !needle ||
                node.name.toLowerCase().includes(needle) ||
                node.summary.toLowerCase().includes(needle)
              const faded =
                selectedId !== node.id && ((!!selectedId && !connected.has(node.id)) || !matches)
              return (
                <button
                  key={node.id}
                  className={`graph-node${selectedId === node.id ? ' is-sel' : ''}${faded ? ' is-dim' : ''}${assetDropId === node.id ? ' drop-target' : ''}`}
                  style={
                    {
                      left: node.x,
                      top: node.y,
                      '--graph-kind': colorFor(def, node.kind),
                    } as CSSProperties
                  }
                  aria-label={`Open ${node.name}, ${labelFor(def, node.kind)}`}
                  aria-current={selectedId === node.id ? 'true' : undefined}
                  title={node.summary || labelFor(def, node.kind)}
                  onClick={() => onSelect(node.id)}
                  onDragOver={(event) => {
                    if (
                      readOnly ||
                      !onAssetDrop ||
                      !Array.from(event.dataTransfer.types).includes(BOARD_ASSET_MIME)
                    )
                      return
                    event.preventDefault()
                    event.dataTransfer.dropEffect = 'link'
                    setAssetDropId(node.id)
                  }}
                  onDragLeave={() => setAssetDropId((id) => (id === node.id ? null : id))}
                  onDrop={(event) => {
                    if (readOnly || !onAssetDrop) return
                    const assetId = event.dataTransfer.getData(BOARD_ASSET_MIME)
                    if (!assetId) return
                    event.preventDefault()
                    onAssetDrop(assetId, node.id)
                    setAssetDropId(null)
                  }}
                >
                  <Icon name={spriteFor(def, node.kind)} size="sm" />
                  <span>
                    <strong>{node.name}</strong>
                    <small>{labelFor(def, node.kind)}</small>
                  </span>
                </button>
              )
            })}
          </div>
        </div>
      )}

      {!loading && !error && subset.limited && (
        <p className="graph-footnote" role="status">
          Showing {layout.nodes.length.toLocaleString()} of {nodes.length.toLocaleString()} nodes
          and up to {GRAPH_LINK_LIMIT.toLocaleString()} explicit relationships. Filter to bring
          matching nodes into view; Tree remains complete.
        </p>
      )}

      {!loading && !error && layout.edges.length === 0 && nodes.length > 0 && (
        <p className="graph-footnote">No parent or influence relationships yet.</p>
      )}
      {layout.dangling > 0 && (
        <p className="graph-footnote">
          {layout.dangling} relationship{layout.dangling === 1 ? '' : 's'} point to missing nodes.
        </p>
      )}
    </section>
  )
}

function edgePath(from: GraphNode, to: GraphNode, index: number): string {
  const x1 = from.x + CARD_W / 2
  const y1 = from.y + CARD_H / 2
  const x2 = to.x + CARD_W / 2
  const y2 = to.y + CARD_H / 2
  const bend = x1 === x2 ? (index % 2 ? 30 : -30) : (x2 - x1) * 0.45
  return `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`
}

function edgeTitle(edge: GraphEdge, from: GraphNode, to: GraphNode): string {
  if (edge.kind === 'parent') return `${from.name} is nested under ${to.name}`
  const role = edge.role ? ROLE_LABEL[edge.role] : 'Influence'
  return `${from.name} inherits from ${to.name} · ${role} · ${edge.weight.toFixed(2)}${edge.enabled ? '' : ' · muted'}`
}
