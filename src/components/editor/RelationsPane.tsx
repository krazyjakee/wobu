import { useMemo, useState, type ReactNode } from 'react'
import type {
  InfluenceLayer,
  KindDef,
  Link,
  LinkEdge,
  LinkRole,
  NodeSummary,
  WobuNode,
} from '../../lib/api'
import {
  useAddNodeLink,
  useNodeBacklinks,
  useNodes,
  useRemoveNodeLink,
  useUpdateNodeLink,
} from '../../lib/queries'
import type { KindIndex } from '../../lib/kinds'

const ROLE_LABEL: Record<LinkRole, string> = {
  species_of: 'Species',
  member_of: 'Member of',
  located_in: 'Located in',
  styled_by: 'Styled by',
  related_to: 'Related to',
}

export function RelationsPane({
  node,
  def,
  kinds,
  readOnly,
  onJump,
}: {
  node: WobuNode
  def: KindDef | undefined
  kinds: KindIndex
  readOnly: boolean
  onJump: (id: string) => void
}) {
  return (
    <RelationsPaneSession
      key={node.id}
      node={node}
      def={def}
      kinds={kinds}
      readOnly={readOnly}
      onJump={onJump}
    />
  )
}

function RelationsPaneSession({
  node,
  def,
  kinds,
  readOnly,
  onJump,
}: {
  node: WobuNode
  def: KindDef | undefined
  kinds: KindIndex
  readOnly: boolean
  onJump: (id: string) => void
}) {
  const nodesQ = useNodes(true)
  const backlinksQ = useNodeBacklinks(node.id)
  const add = useAddNodeLink()
  const remove = useRemoveNodeLink()
  const update = useUpdateNodeLink()
  const nodes = useMemo(() => nodesQ.data ?? [], [nodesQ.data])
  const byId = useMemo(() => new Map(nodes.map((item) => [item.id, item])), [nodes])
  const allowedRoles = useMemo(() => def?.defaultLinkRoles ?? [], [def])
  const [role, setRole] = useState<LinkRole | ''>(allowedRoles[0] ?? '')
  const [target, setTarget] = useState('')

  const outgoingRoles = useMemo(() => {
    const roles = [...allowedRoles]
    for (const link of node.links) {
      if (!roles.includes(link.role)) roles.push(link.role)
    }
    return roles
  }, [allowedRoles, node.links])
  const availableTargets = nodes
    .filter(
      (candidate) =>
        candidate.id !== node.id &&
        (!role || kinds.get(candidate.kind)?.layer === targetLayerForRole(role)) &&
        (!role || !node.links.some((link) => link.role === role && link.toId === candidate.id)),
    )
    .sort((a, b) => a.name.localeCompare(b.name))
  const parent = node.parentId ? byId.get(node.parentId) : undefined
  const busy = add.isPending || remove.isPending || update.isPending

  return (
    <div className="relations-pane">
      <section className="relations-section">
        <div className="relations-heading">
          <div>
            <h2>Influences</h2>
            <p>What this node inherits from, grouped by the route into its prompt.</p>
          </div>
        </div>

        {node.parentId && (
          <RelationGroup title="Parent">
            <div className="relation-row is-implicit">
              <RelationTarget target={parent} fallback={node.parentId} onJump={onJump} />
              <span className="relation-note">Implicit · read-only</span>
              <span className="relation-weight-static">1.00</span>
            </div>
          </RelationGroup>
        )}

        {outgoingRoles.map((groupRole) => {
          const links = node.links.filter((link) => link.role === groupRole)
          if (links.length === 0) return null
          return (
            <RelationGroup key={groupRole} title={ROLE_LABEL[groupRole]}>
              {links.map((link) => (
                <OutgoingRow
                  key={`${link.role}:${link.toId}:${link.weight}`}
                  nodeId={node.id}
                  link={link}
                  target={byId.get(link.toId)}
                  readOnly={readOnly}
                  busy={busy}
                  onJump={onJump}
                  onRemove={() =>
                    remove.mutate({ nodeId: node.id, toId: link.toId, role: link.role })
                  }
                  onUpdate={(patch) =>
                    update.mutate({ nodeId: node.id, toId: link.toId, role: link.role, ...patch })
                  }
                />
              ))}
            </RelationGroup>
          )
        })}

        {!node.parentId && node.links.length === 0 && (
          <p className="relations-empty">
            No influences yet. Add the first explicit relation below.
          </p>
        )}

        <div className="relation-add">
          {allowedRoles.length === 0 ? (
            <p>This kind does not declare any editable relation roles.</p>
          ) : (
            <>
              <select
                value={role}
                disabled={readOnly || busy}
                onChange={(event) => {
                  setRole(event.target.value as LinkRole)
                  setTarget('')
                }}
                aria-label="Relation role"
              >
                {allowedRoles.map((allowed) => (
                  <option key={allowed} value={allowed}>
                    {ROLE_LABEL[allowed]}
                  </option>
                ))}
              </select>
              <select
                value={target}
                disabled={readOnly || busy || availableTargets.length === 0}
                onChange={(event) => setTarget(event.target.value)}
                aria-label="Relation target"
              >
                <option value="">Choose a node…</option>
                {availableTargets.map((candidate) => (
                  <option key={candidate.id} value={candidate.id}>
                    {candidate.name}
                  </option>
                ))}
              </select>
              <button
                className="btn btn-primary"
                disabled={readOnly || busy || !role || !target}
                onClick={() => {
                  if (!role || !target) return
                  add.mutate(
                    { nodeId: node.id, toId: target, role },
                    { onSuccess: () => setTarget('') },
                  )
                }}
              >
                Add relation
              </button>
            </>
          )}
        </div>
      </section>

      <section className="relations-section relations-backlinks">
        <div className="relations-heading">
          <div>
            <h2>Inherited by</h2>
            <p>{backlinkSummary(backlinksQ.data ?? [], byId, kinds)}</p>
          </div>
        </div>
        {backlinksQ.isPending && <p className="relations-empty">Reading backlinks…</p>}
        {backlinksQ.isError && <p className="relations-empty">Backlinks could not be read.</p>}
        {backlinksQ.data?.length === 0 && (
          <p className="relations-empty">Nothing explicitly inherits from this node.</p>
        )}
        {backlinkRoles(backlinksQ.data ?? []).map((groupRole) => (
          <RelationGroup key={groupRole} title={ROLE_LABEL[groupRole]}>
            {(backlinksQ.data ?? [])
              .filter((edge) => edge.role === groupRole)
              .map((edge) => (
                <BacklinkRow
                  key={`${edge.fromId}:${edge.role}`}
                  edge={edge}
                  source={byId.get(edge.fromId)}
                  onJump={onJump}
                />
              ))}
          </RelationGroup>
        ))}
      </section>
    </div>
  )
}

/** A relation's route determines which kind of node can meaningfully supply it. */
export function targetLayerForRole(role: LinkRole): InfluenceLayer {
  switch (role) {
    case 'styled_by':
      return 'style'
    case 'species_of':
      return 'ancestry'
    case 'member_of':
      return 'culture'
    case 'located_in':
      return 'place'
    case 'related_to':
      return 'subject'
  }
}

function RelationGroup({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="relation-group">
      <h3>{title}</h3>
      <div className="relation-list">{children}</div>
    </div>
  )
}

function RelationTarget({
  target,
  fallback,
  onJump,
}: {
  target: NodeSummary | undefined
  fallback: string
  onJump: (id: string) => void
}) {
  return (
    <button className="relation-target" onClick={() => onJump(target?.id ?? fallback)}>
      <strong>{target?.name ?? 'Missing node'}</strong>
      <span>{target?.kind.replaceAll('_', ' ') ?? fallback}</span>
    </button>
  )
}

function OutgoingRow({
  nodeId: _nodeId,
  link,
  target,
  readOnly,
  busy,
  onJump,
  onRemove,
  onUpdate,
}: {
  nodeId: string
  link: Link
  target: NodeSummary | undefined
  readOnly: boolean
  busy: boolean
  onJump: (id: string) => void
  onRemove: () => void
  onUpdate: (patch: { weight?: number; enabled?: boolean }) => void
}) {
  const [weight, setWeight] = useState(String(link.weight))
  const commitWeight = () => {
    const parsed = Number(weight)
    if (!Number.isFinite(parsed)) {
      setWeight(String(link.weight))
      return
    }
    const clamped = Math.max(0, Math.min(1, parsed))
    setWeight(String(clamped))
    if (clamped !== link.weight) onUpdate({ weight: clamped })
  }

  return (
    <div className={link.enabled ? 'relation-row' : 'relation-row is-muted'}>
      <RelationTarget target={target} fallback={link.toId} onJump={onJump} />
      <label className="relation-enabled">
        <input
          type="checkbox"
          checked={link.enabled}
          disabled={readOnly || busy}
          onChange={(event) => onUpdate({ enabled: event.target.checked })}
        />
        {link.enabled ? 'Active' : 'Muted'}
      </label>
      <label className="relation-weight">
        Weight
        <input
          type="number"
          min="0"
          max="1"
          step="0.05"
          value={weight}
          disabled={readOnly || busy}
          onChange={(event) => setWeight(event.target.value)}
          onBlur={commitWeight}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur()
            if (event.key === 'Escape') {
              setWeight(String(link.weight))
              event.currentTarget.blur()
            }
          }}
        />
      </label>
      <button className="btn btn-mini" disabled={readOnly || busy} onClick={onRemove}>
        Remove
      </button>
    </div>
  )
}

function BacklinkRow({
  edge,
  source,
  onJump,
}: {
  edge: LinkEdge
  source: NodeSummary | undefined
  onJump: (id: string) => void
}) {
  return (
    <div
      className={edge.enabled ? 'relation-row is-backlink' : 'relation-row is-backlink is-muted'}
    >
      <RelationTarget target={source} fallback={edge.fromId} onJump={onJump} />
      <span className="relation-note">{edge.enabled ? 'Inherits from this' : 'Muted'}</span>
      <span className="relation-weight-static">{edge.weight.toFixed(2)}</span>
    </div>
  )
}

function backlinkRoles(edges: LinkEdge[]): LinkRole[] {
  const roles: LinkRole[] = []
  for (const edge of edges) {
    if (!roles.includes(edge.role)) roles.push(edge.role)
  }
  return roles
}

function backlinkSummary(
  edges: LinkEdge[],
  nodes: Map<string, NodeSummary>,
  kinds: KindIndex,
): string {
  const counts = new Map<string, { count: number; singular: string; plural: string }>()
  for (const edge of edges.filter((item) => item.enabled)) {
    const source = nodes.get(edge.fromId)
    if (!source) continue
    const def = kinds.get(source.kind)
    const current = counts.get(source.kind) ?? {
      count: 0,
      singular: def?.label.toLocaleLowerCase() ?? source.kind.replaceAll('_', ' '),
      plural: def?.plural.toLocaleLowerCase() ?? `${source.kind.replaceAll('_', ' ')}s`,
    }
    current.count += 1
    counts.set(source.kind, current)
  }
  const parts = [...counts.values()].map(
    ({ count, singular, plural }) => `${count} ${count === 1 ? singular : plural}`,
  )
  return parts.length > 0 ? `${parts.join(', ')} inherit from this.` : 'No active backlinks.'
}
