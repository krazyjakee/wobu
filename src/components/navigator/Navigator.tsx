import { useMemo, useState } from 'react'
import type { CorruptFile, NodeKind, NodeSummary } from '../../lib/api'
import { useDeleteNode, useDuplicateNode, useMoveNode } from '../../lib/queries'
import { colorFor, labelFor, pluralFor, spriteFor, type KindIndex } from '../../lib/kinds'
import { descendantsOf, filterTree, type KindGroup, type TreeNode } from '../../lib/tree'
import { canDrop as allow } from '../../lib/drop'
import { useUI, report, toast } from '../../store/ui'
import { Icon } from '../Icon'
import { ContextMenu } from './ContextMenu'
import { ConfirmSheet } from '../ConfirmSheet'
import { BrokenFiles } from './BrokenFiles'

const DRAG_MIME = 'application/x-wobu-node'

interface Ctx {
  x: number
  y: number
  node: NodeSummary
}

export function Navigator({
  nodes,
  byId,
  pinned,
  groups,
  kinds,
  loading,
  error,
  readOnly,
  corrupt,
  projectPath,
  onNewNode,
}: {
  nodes: NodeSummary[]
  byId: Map<string, NodeSummary>
  pinned: NodeSummary[]
  groups: KindGroup[]
  kinds: KindIndex
  loading: boolean
  error: string | null
  readOnly: boolean
  corrupt: CorruptFile[]
  projectPath: string
  onNewNode: (kind: NodeKind | null, parentId: string | null) => void
}) {
  const filter = useUI((s) => s.filter)
  const setFilter = useUI((s) => s.setFilter)
  const selectedId = useUI((s) => s.selectedId)
  const select = useUI((s) => s.select)
  const closedGroups = useUI((s) => s.closedGroups)
  const toggleGroup = useUI((s) => s.toggleGroup)
  const collapsedNodes = useUI((s) => s.collapsedNodes)
  const toggleNodeOpen = useUI((s) => s.toggleNodeOpen)

  const [ctx, setCtx] = useState<Ctx | null>(null)
  const [confirm, setConfirm] = useState<NodeSummary | null>(null)
  const [dragId, setDragId] = useState<string | null>(null)
  const [dropId, setDropId] = useState<string | null>(null)

  const move = useMoveNode()
  const del = useDeleteNode()
  const dup = useDuplicateNode()

  const forbidden = useMemo(
    () => (dragId ? descendantsOf(dragId, nodes) : new Set<string>()),
    [dragId, nodes],
  )

  /** The rules themselves live in lib/drop.ts, where they can be tested. */
  function canDrop(targetId: string | null, targetKind: NodeKind): boolean {
    return allow({ dragId, byId, forbidden, kinds, readOnly }, targetId, targetKind)
  }

  function doMove(targetId: string | null) {
    if (!dragId) return
    const src = byId.get(dragId)
    move.mutate(
      { id: dragId, newParentId: targetId },
      {
        onError: (e) => report(e),
        onSuccess: () =>
          toast(
            targetId
              ? `${src?.name ?? 'Node'} moved under ${byId.get(targetId)?.name ?? 'node'}`
              : `${src?.name ?? 'Node'} moved to the top level`,
          ),
      },
    )
    setDragId(null)
    setDropId(null)
  }

  return (
    <aside className="nav">
      <div className="nav-search">
        <Icon name="search" size="sm" />
        <input
          placeholder="Filter world…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          spellCheck={false}
        />
        {filter && (
          <button className="clear" onClick={() => setFilter('')} aria-label="Clear filter">
            <Icon name="x" size="sm" />
          </button>
        )}
      </div>

      {pinned.length > 0 && (
        <div className="nav-pinned">
          {pinned.map((n) => {
            const def = kinds.get(n.kind)
            return (
              <button
                key={n.id}
                className={`node node-pin${selectedId === n.id ? ' is-sel' : ''}`}
                onClick={() => select(n.id)}
                onContextMenu={(e) => {
                  e.preventDefault()
                  setCtx({ x: e.clientX, y: e.clientY, node: n })
                }}
                title={n.summary || labelFor(def, n.kind)}
              >
                <Icon
                  name={spriteFor(def, n.kind)}
                  size="sm"
                  style={{ color: colorFor(def, n.kind) }}
                />
                <span className="nm">{n.name}</span>
                {n.descriptionState === 'stale' && <span className="stale" title="stale" />}
              </button>
            )
          })}
        </div>
      )}

      <div className="nav-tree">
        <BrokenFiles files={corrupt} projectPath={projectPath} />
        {loading && <p className="nav-note">Reading the world…</p>}
        {error && <p className="nav-note">Could not list nodes — {error}</p>}
        {!loading && !error && nodes.length === 0 && (
          <p className="nav-note">
            This project has no nodes yet. <b>New entity</b> writes the first Markdown file into{' '}
            <b>nodes/</b>.
          </p>
        )}

        {groups.map((g) => {
          const visible = filterTree(g.roots, filter)
          if (filter && visible.length === 0) return null
          const open = !closedGroups[g.kind] || !!filter
          return (
            <div key={g.kind} className={open ? 'group open' : 'group'}>
              <button
                className={`group-h${dropId === `group:${g.kind}` ? ' drop-target' : ''}`}
                onClick={() => toggleGroup(g.kind)}
                onDragOver={(e) => {
                  if (!canDrop(null, g.kind)) return
                  e.preventDefault()
                  setDropId(`group:${g.kind}`)
                }}
                onDragLeave={() => setDropId((d) => (d === `group:${g.kind}` ? null : d))}
                onDrop={(e) => {
                  if (!canDrop(null, g.kind)) return
                  e.preventDefault()
                  doMove(null)
                }}
                onContextMenu={(e) => {
                  e.preventDefault()
                  setCtx(null)
                  onNewNode(g.kind, null)
                }}
              >
                <Icon name="chev" />
                {pluralFor(g.def, g.kind)}
                <span className="gcount">{g.count}</span>
              </button>
              {open && (
                <div className="group-items">
                  {visible.map((t) => (
                    <Row
                      key={t.node.id}
                      t={t}
                      kinds={kinds}
                      selectedId={selectedId}
                      collapsed={collapsedNodes}
                      forceOpen={!!filter}
                      dragId={dragId}
                      dropId={dropId}
                      onSelect={select}
                      onToggle={toggleNodeOpen}
                      onContext={(x, y, node) => setCtx({ x, y, node })}
                      onDragStart={(id) => setDragId(id)}
                      onDragEnd={() => {
                        setDragId(null)
                        setDropId(null)
                      }}
                      canDrop={canDrop}
                      onDropOn={(id) => doMove(id)}
                      setDropId={setDropId}
                      readOnly={readOnly}
                    />
                  ))}
                </div>
              )}
            </div>
          )
        })}
      </div>

      <button className="nav-new" onClick={() => onNewNode(null, null)} disabled={readOnly}>
        <Icon name="plus" size="sm" />
        New entity
      </button>

      {ctx && (
        <ContextMenu x={ctx.x} y={ctx.y} onClose={() => setCtx(null)}>
          <NodeMenu
            node={ctx.node}
            kinds={kinds}
            readOnly={readOnly}
            busy={dup.isPending || del.isPending}
            onClose={() => setCtx(null)}
            onNewNode={onNewNode}
            onDuplicate={() =>
              dup.mutate(ctx.node.id, {
                onError: (e) => report(e),
                onSuccess: (n) => {
                  select(n.id)
                  toast(`Duplicated as “${n.name}”`)
                },
              })
            }
            onDelete={() => setConfirm(ctx.node)}
          />
        </ContextMenu>
      )}

      {confirm && (
        <ConfirmSheet
          title={`Delete “${confirm.name}”?`}
          body={deleteWarning(confirm, nodes)}
          confirmLabel="Delete"
          danger
          busy={del.isPending}
          onCancel={() => setConfirm(null)}
          onConfirm={() => {
            const id = confirm.id
            del.mutate(id, {
              onError: (e) => report(e),
              onSuccess: () => {
                if (selectedId === id) select(null)
                toast('Node deleted')
              },
            })
            setConfirm(null)
          }}
        />
      )}
    </aside>
  )
}

function deleteWarning(node: NodeSummary, nodes: NodeSummary[]): string {
  const kids = descendantsOf(node.id, nodes).size
  const base = 'Its Markdown file is removed from the project folder.'
  if (kids === 0) return base
  return `${base} ${kids} node${kids === 1 ? '' : 's'} nest inside it — how the backend treats them is its call, so check the tree afterwards.`
}

function NodeMenu({
  node,
  kinds,
  readOnly,
  busy,
  onClose,
  onNewNode,
  onDuplicate,
  onDelete,
}: {
  node: NodeSummary
  kinds: KindIndex
  readOnly: boolean
  busy: boolean
  onClose: () => void
  onNewNode: (kind: NodeKind | null, parentId: string | null) => void
  onDuplicate: () => void
  onDelete: () => void
}) {
  const def = kinds.get(node.kind)
  const pick = (fn: () => void) => () => {
    onClose()
    fn()
  }
  return (
    <>
      <div className="ctx-label">{labelFor(def, node.kind)}</div>
      <button disabled={readOnly} onClick={pick(() => onNewNode(node.kind, node.parentId))}>
        <Icon name="plus" size="sm" />
        New {labelFor(def, node.kind).toLowerCase()}
      </button>
      {def?.nests && (
        <button disabled={readOnly} onClick={pick(() => onNewNode(node.kind, node.id))}>
          <Icon name="plus" size="sm" />
          New child of {node.name}
        </button>
      )}
      <div className="ctx-sep" />
      <button disabled={readOnly || def?.singleton || busy} onClick={pick(onDuplicate)}>
        <Icon name="copy" size="sm" />
        Duplicate
      </button>
      <button className="danger" disabled={readOnly || busy} onClick={pick(onDelete)}>
        <Icon name="trash" size="sm" />
        Delete
      </button>
    </>
  )
}

function Row({
  t,
  kinds,
  selectedId,
  collapsed,
  forceOpen,
  dragId,
  dropId,
  onSelect,
  onToggle,
  onContext,
  onDragStart,
  onDragEnd,
  canDrop,
  onDropOn,
  setDropId,
  readOnly,
}: {
  t: TreeNode
  kinds: KindIndex
  selectedId: string | null
  collapsed: Record<string, true>
  forceOpen: boolean
  dragId: string | null
  dropId: string | null
  onSelect: (id: string) => void
  onToggle: (id: string) => void
  onContext: (x: number, y: number, node: NodeSummary) => void
  onDragStart: (id: string) => void
  onDragEnd: () => void
  canDrop: (targetId: string | null, kind: NodeKind) => boolean
  onDropOn: (id: string) => void
  setDropId: (id: string | null) => void
  readOnly: boolean
}) {
  const n = t.node
  const def = kinds.get(n.kind)
  const hasKids = t.children.length > 0
  const open = forceOpen || !collapsed[n.id]
  const cls = [
    'node',
    selectedId === n.id ? 'is-sel' : '',
    dragId === n.id ? 'is-dragging' : '',
    dropId === n.id ? 'drop-target' : '',
  ]
    .filter(Boolean)
    .join(' ')

  return (
    <>
      <button
        className={cls}
        style={{ paddingLeft: 8 + t.depth * 14 }}
        onClick={() => onSelect(n.id)}
        onContextMenu={(e) => {
          e.preventDefault()
          onContext(e.clientX, e.clientY, n)
        }}
        draggable={!readOnly}
        onDragStart={(e) => {
          e.dataTransfer.setData(DRAG_MIME, n.id)
          e.dataTransfer.effectAllowed = 'move'
          onDragStart(n.id)
        }}
        onDragEnd={onDragEnd}
        onDragOver={(e) => {
          if (!canDrop(n.id, n.kind)) return
          e.preventDefault()
          e.dataTransfer.dropEffect = 'move'
          setDropId(n.id)
        }}
        onDragLeave={() => setDropId(null)}
        onDrop={(e) => {
          if (!canDrop(n.id, n.kind)) return
          e.preventDefault()
          onDropOn(n.id)
        }}
        title={n.summary || n.name}
      >
        {hasKids ? (
          <span
            className={open ? 'twist open' : 'twist'}
            onClick={(e) => {
              e.stopPropagation()
              onToggle(n.id)
            }}
            role="presentation"
          >
            <Icon name="chev" size="sm" style={{ width: 11, height: 11 }} />
          </span>
        ) : (
          <span className="twist" />
        )}
        <Icon name={spriteFor(def, n.kind)} size="sm" style={{ color: colorFor(def, n.kind) }} />
        <span className="nm">{n.name}</span>
        {n.descriptionState === 'stale' && (
          <span className="stale" title="description is stale" />
        )}
      </button>
      {hasKids &&
        open &&
        t.children.map((c) => (
          <Row
            key={c.node.id}
            t={c}
            kinds={kinds}
            selectedId={selectedId}
            collapsed={collapsed}
            forceOpen={forceOpen}
            dragId={dragId}
            dropId={dropId}
            onSelect={onSelect}
            onToggle={onToggle}
            onContext={onContext}
            onDragStart={onDragStart}
            onDragEnd={onDragEnd}
            canDrop={canDrop}
            onDropOn={onDropOn}
            setDropId={setDropId}
            readOnly={readOnly}
          />
        ))}
    </>
  )
}
