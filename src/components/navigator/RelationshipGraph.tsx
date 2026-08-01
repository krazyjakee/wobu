import { useMemo } from 'react'
import type { CSSProperties } from 'react'
import type { LinkEdge, LinkRole, NodeKind, NodeSummary } from '../../lib/api'
import { colorFor, labelFor, spriteFor, type KindIndex } from '../../lib/kinds'
import { Icon } from '../Icon'

const CARD_W = 112
const CARD_H = 46
const COL_GAP = 14
const ROW_GAP = 16
const PAD_X = 10
const GROUP_HEAD = 30
const GROUP_GAP = 26
const COLS = 2

const ROLE_LABEL: Record<LinkRole, string> = {
  species_of: 'Species',
  member_of: 'Member of',
  located_in: 'Located in',
  styled_by: 'Styled by',
  related_to: 'Related to',
}

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

/**
 * A deterministic, kind-banded layout. It intentionally has no drag state:
 * this map explains the document model but never becomes another editor for it.
 */
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

export function RelationshipGraph({
  nodes,
  links,
  kinds,
  selectedId,
  filter,
  loading,
  error,
  onSelect,
}: {
  nodes: NodeSummary[]
  links: LinkEdge[]
  kinds: KindIndex
  selectedId: string | null
  filter: string
  loading: boolean
  error: string | null
  onSelect: (id: string) => void
}) {
  const layout = useMemo(() => layoutRelationships(nodes, links, kinds), [nodes, links, kinds])
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
        <strong>Read-only map</strong>
        <span>Select a node to open it. Edit links in Relations.</span>
      </div>
      <div className="graph-legend" aria-label="Relationship legend">
        <span><i className="is-parent" /> Parent</span>
        <span><i className="is-influence" /> Influence</span>
        <span><i className="is-muted" /> Muted</span>
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
                <marker id="graph-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="5" markerHeight="5" orient="auto-start-reverse">
                  <path d="M 0 0 L 8 4 L 0 8 z" />
                </marker>
              </defs>
              {layout.groups.map((group) => (
                <g key={group.kind} className="graph-group-label">
                  <text x={PAD_X} y={group.y + 13}>{group.label}</text>
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
                ].filter(Boolean).join(' ')
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
                selectedId !== node.id &&
                ((!!selectedId && !connected.has(node.id)) || !matches)
              return (
                <button
                  key={node.id}
                  className={`graph-node${selectedId === node.id ? ' is-sel' : ''}${faded ? ' is-dim' : ''}`}
                  style={{
                    left: node.x,
                    top: node.y,
                    '--graph-kind': colorFor(def, node.kind),
                  } as CSSProperties}
                  aria-label={`Open ${node.name}, ${labelFor(def, node.kind)}`}
                  aria-current={selectedId === node.id ? 'true' : undefined}
                  title={node.summary || labelFor(def, node.kind)}
                  onClick={() => onSelect(node.id)}
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

      {!loading && !error && layout.edges.length === 0 && nodes.length > 0 && (
        <p className="graph-footnote">No parent or influence relationships yet.</p>
      )}
      {layout.dangling > 0 && (
        <p className="graph-footnote">{layout.dangling} relationship{layout.dangling === 1 ? '' : 's'} point to missing nodes.</p>
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
