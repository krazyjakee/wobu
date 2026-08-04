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
import { useNodeThumbs } from '../../lib/nodeThumbs'
import { useContextMenu } from '../../hooks/useContextMenu'
import { colorFor, spriteFor, type KindIndex } from '../../lib/kinds'
import { NodeThumbnail } from '../AssetMedia'
import { Combobox } from '../Combobox'
import { ContextMenu, MenuItem, MenuLabel, MenuSeparator } from '../ContextMenu'
import { Icon } from '../Icon'

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
  const availableTargets = nodes.filter(
    (candidate) =>
      candidate.id !== node.id &&
      (!role || kinds.get(candidate.kind)?.layer === targetLayerForRole(role)) &&
      (!role || !node.links.some((link) => link.role === role && link.toId === candidate.id)),
  )
  const parent = node.parentId ? byId.get(node.parentId) : undefined
  const busy = add.isPending || remove.isPending || update.isPending

  /*
   * Every entity this pane names, asked for once.
   *
   * Both halves of the pane go into one list on purpose: influences and
   * backlinks are drawn in the same commit, so asking for them together is one
   * call rather than two, and a node that appears on both sides — A styled by
   * B, B located in A — is asked for once rather than twice. Neither list is
   * virtualized, but neither is unbounded either: `nodeThumbBatch` pages at the
   * backend's limit, so a subject that a hundred others inherit from still
   * costs a bounded number of calls.
   *
   * The target picker's open rows join the same list. It reports only the rows
   * it is currently drawing, so a candidate list the length of the world adds
   * one screenful of ids here rather than all of them.
   */
  const [drawnTargets, setDrawnTargets] = useState<string[]>([])
  const thumbIds = useMemo(() => {
    const ids = new Set<string>()
    if (node.parentId) ids.add(node.parentId)
    for (const link of node.links) ids.add(link.toId)
    for (const edge of backlinksQ.data ?? []) ids.add(edge.fromId)
    for (const id of drawnTargets) ids.add(id)
    return [...ids]
  }, [backlinksQ.data, drawnTargets, node.links, node.parentId])
  const thumbs = useNodeThumbs(thumbIds)

  const roleOptions = useMemo(
    () => allowedRoles.map((allowed) => ({ value: allowed, label: ROLE_LABEL[allowed] })),
    [allowedRoles],
  )
  // Not memoised: a thumbnail resolving re-renders the pane, and a list keyed
  // on the candidates alone would go on drawing the icons it was built with.
  const targetOptions = availableTargets.map((candidate) => {
    const candidateDef = kinds.get(candidate.kind)
    return {
      value: candidate.id,
      label: candidate.name,
      keywords: candidate.kind,
      icon: (
        <NodeThumbnail
          path={thumbs.get(candidate.id)}
          fallback={
            <Icon
              name={spriteFor(candidateDef, candidate.kind)}
              size="sm"
              style={{ color: colorFor(candidateDef, candidate.kind) }}
            />
          }
        />
      ),
    }
  })

  return (
    <div className="relations-pane">
      <section className="relations-section">
        <div className="relations-heading">
          <div>
            <h2>Influences</h2>
            <p>What this entity inherits from, grouped by how each one reaches its prompt.</p>
          </div>
        </div>

        {node.parentId && (
          <RelationGroup title="Parent">
            <div className="relation-row is-implicit">
              <RelationTarget
                target={parent}
                fallback={node.parentId}
                kinds={kinds}
                thumb={thumbs.get(node.parentId)}
                onJump={onJump}
              />
              <span className="relation-note">From its parent · not editable here</span>
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
                  kinds={kinds}
                  thumb={thumbs.get(link.toId)}
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
          <p className="relations-empty">No influences yet. Add the first one below.</p>
        )}

        <div className="relation-add">
          {allowedRoles.length === 0 ? (
            <p>This kind of entity has no relations you can add by hand.</p>
          ) : (
            <>
              <Combobox
                label="Relation role"
                value={role}
                options={roleOptions}
                disabled={readOnly || busy}
                onChange={(next) => {
                  setRole(next as LinkRole)
                  setTarget('')
                }}
              />
              <Combobox
                label="Relation target"
                value={target}
                options={targetOptions}
                sort="title"
                placeholder="Choose an entity…"
                emptyMessage="No entity of that sort matches."
                disabled={readOnly || busy || availableTargets.length === 0}
                onChange={setTarget}
                onDrawnRows={setDrawnTargets}
              />
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
        {backlinksQ.isPending && (
          <p className="relations-empty">Reading what inherits from this…</p>
        )}
        {backlinksQ.isError && (
          <p className="relations-empty">Wobu could not read what inherits from this.</p>
        )}
        {backlinksQ.data?.length === 0 && (
          <p className="relations-empty">Nothing else inherits from this entity.</p>
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
                  kinds={kinds}
                  thumb={thumbs.get(edge.fromId)}
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
function targetLayerForRole(role: LinkRole): InfluenceLayer {
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

/**
 * One end of a relation: its picture, its name, and what it is.
 *
 * The thumbnail sits *inside* the jump button rather than beside it, so the
 * picture is part of the same target the user is already aiming at — and so the
 * row's grid keeps the four columns it was laid out with. It is decorative
 * (`NodeThumbnail` marks it so), which leaves the button's accessible name the
 * entity's name, exactly as before.
 */
function RelationTarget({
  target,
  fallback,
  kinds,
  thumb,
  onJump,
}: {
  target: NodeSummary | undefined
  fallback: string
  kinds: KindIndex
  /** Resolved by the pane above; `null` while unknown and when there is none. */
  thumb: string | null
  onJump: (id: string) => void
}) {
  const def = target ? kinds.get(target.kind) : undefined
  return (
    <button className="relation-target" onClick={() => onJump(target?.id ?? fallback)}>
      <NodeThumbnail
        path={thumb}
        fallback={
          <Icon
            // A link whose other end is gone still occupies the same slot: it
            // is the one row in this pane the user most needs to see and fix.
            name={target ? spriteFor(def, target.kind) : 'cube'}
            size="sm"
            style={target ? { color: colorFor(def, target.kind) } : undefined}
          />
        }
      />
      <span className="relation-target-text">
        <strong>{target?.name ?? 'Missing entity'}</strong>
        <span>{target?.kind.replaceAll('_', ' ') ?? fallback}</span>
      </span>
    </button>
  )
}

function OutgoingRow({
  nodeId: _nodeId,
  link,
  target,
  kinds,
  thumb,
  readOnly,
  busy,
  onJump,
  onRemove,
  onUpdate,
}: {
  nodeId: string
  link: Link
  target: NodeSummary | undefined
  kinds: KindIndex
  thumb: string | null
  readOnly: boolean
  busy: boolean
  onJump: (id: string) => void
  onRemove: () => void
  onUpdate: (patch: { weight?: number; enabled?: boolean }) => void
}) {
  const [weight, setWeight] = useState(String(link.weight))
  const menu = useContextMenu<Link>()
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

  const frozen = readOnly
    ? 'This project folder is read-only, so its relations cannot be changed.'
    : busy
      ? 'Another change to this entity’s relations is still being saved.'
      : null

  return (
    <div
      className={link.enabled ? 'relation-row' : 'relation-row is-muted'}
      // Not a tab stop: the row's own controls are. It takes focus only when a
      // menu is opened on it, so that closing the menu lands somewhere.
      tabIndex={-1}
      {...menu.trigger(link)}
    >
      <RelationTarget
        target={target}
        fallback={link.toId}
        kinds={kinds}
        thumb={thumb}
        onJump={onJump}
      />
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

      {/*
        The three things a relation row does, without hunting for its controls.
        Weight is left out on purpose: it is a value rather than an action, the
        field beside it is already the shortest route to typing one, and a menu
        cannot hold a number without becoming a form.
      */}
      {menu.anchor && (
        <ContextMenu
          x={menu.anchor.x}
          y={menu.anchor.y}
          onClose={menu.close}
          restoreFocus={menu.anchor.opener}
          label={`Actions for the relation to ${target?.name ?? link.toId}`}
        >
          <MenuLabel>
            {ROLE_LABEL[link.role]} · weight {link.weight.toFixed(2)}
          </MenuLabel>
          <MenuItem
            icon={<Icon name="link" size="sm" />}
            disabledReason={
              target
                ? null
                : 'The other end of this relation is missing from the project, so there is nothing to open.'
            }
            onSelect={() => onJump(link.toId)}
          >
            Open {target?.name ?? 'the missing entity'}
          </MenuItem>
          <MenuItem
            icon={<Icon name={link.enabled ? 'minus' : 'check'} size="sm" />}
            disabledReason={frozen}
            onSelect={() => onUpdate({ enabled: !link.enabled })}
          >
            {link.enabled ? 'Mute this influence' : 'Unmute this influence'}
          </MenuItem>
          <MenuSeparator />
          <MenuItem
            danger
            icon={<Icon name="trash" size="sm" />}
            disabledReason={frozen}
            onSelect={onRemove}
          >
            Remove relation
          </MenuItem>
        </ContextMenu>
      )}
    </div>
  )
}

function BacklinkRow({
  edge,
  source,
  kinds,
  thumb,
  onJump,
}: {
  edge: LinkEdge
  source: NodeSummary | undefined
  kinds: KindIndex
  thumb: string | null
  onJump: (id: string) => void
}) {
  return (
    <div
      className={edge.enabled ? 'relation-row is-backlink' : 'relation-row is-backlink is-muted'}
    >
      <RelationTarget
        target={source}
        fallback={edge.fromId}
        kinds={kinds}
        thumb={thumb}
        onJump={onJump}
      />
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
  return parts.length > 0 ? `${parts.join(', ')} inherit from this.` : 'Nothing inherits from this.'
}
