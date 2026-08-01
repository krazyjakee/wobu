import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage, type CorruptFile, type NodeKind, type NodeSummary } from '../../lib/api'
import { useDeleteNode, useDuplicateNode, useMoveNode, useNodeLinks } from '../../lib/queries'
import { colorFor, labelFor, pluralFor, spriteFor, type KindIndex } from '../../lib/kinds'
import { descendantsOf, type KindGroup, type TreeNode } from '../../lib/tree'
import { canDrop as allow } from '../../lib/drop'
import { BOARD_ASSET_MIME } from '../../lib/board'
import { editingTitle } from '../../lib/presence'
import { useUI, report, toast } from '../../store/ui'
import { Icon } from '../Icon'
import { ContextMenu } from './ContextMenu'
import { ConfirmSheet } from '../ConfirmSheet'
import { BrokenFiles } from './BrokenFiles'
import { RelationshipGraph } from './RelationshipGraph'
import { buildNavigatorRows, groupDropId, type NavigatorListRow } from './navigatorRows'

const DRAG_MIME = 'application/x-wobu-node'

interface Ctx {
  x: number
  y: number
  node: NodeSummary
  opener: HTMLButtonElement
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
  editedElsewhere,
  projectPath,
  onNewNode,
  onStyleTransfer,
  onAssetDrop,
  onRowRender,
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
  /** Node id → who else has it open. Advisory; see `lib/presence.ts`. */
  editedElsewhere: Map<string, string>
  projectPath: string
  onNewNode: (kind: NodeKind | null, parentId: string | null) => void
  onStyleTransfer?: () => void
  /** Board-only drop path; absent in Library so node reparenting is unchanged. */
  onAssetDrop?: (assetId: string, nodeId: string) => void
  /** Performance-test instrumentation; omitted by the application. */
  onRowRender?: (nodeId: string) => void
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
  const [view, setView] = useState<'tree' | 'graph'>('tree')

  const move = useMoveNode()
  const del = useDeleteNode()
  const dup = useDuplicateNode()
  const linksQ = useNodeLinks(view === 'graph')
  const moveNode = move.mutate
  const assetDropRef = useRef(onAssetDrop)
  useEffect(() => {
    assetDropRef.current = onAssetDrop
  }, [onAssetDrop])

  const forbidden = useMemo(
    () => (dragId ? descendantsOf(dragId, nodes) : new Set<string>()),
    [dragId, nodes],
  )

  const list = useMemo(
    () => buildNavigatorRows(groups, filter, closedGroups, collapsedNodes),
    [closedGroups, collapsedNodes, filter, groups],
  )

  /** The rules themselves live in lib/drop.ts, where they can be tested. */
  const canDrop = useCallback(
    (targetId: string | null, targetKind: NodeKind): boolean =>
      allow({ dragId, byId, forbidden, kinds, readOnly }, targetId, targetKind),
    [byId, dragId, forbidden, kinds, readOnly],
  )

  const doMove = useCallback(
    (targetId: string | null) => {
      if (!dragId) return
      const src = byId.get(dragId)
      moveNode(
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
    },
    [byId, dragId, moveNode],
  )

  const handleContext = useCallback(
    (x: number, y: number, node: NodeSummary, opener: HTMLButtonElement) =>
      setCtx({ x, y, node, opener }),
    [],
  )
  const handleDragEnd = useCallback(() => {
    setDragId(null)
    setDropId(null)
  }, [])
  const handleAssetDrop = useCallback(
    (assetId: string, nodeId: string) => assetDropRef.current?.(assetId, nodeId),
    [],
  )
  const handleGroupContext = useCallback(
    (kind: NodeKind) => {
      setCtx(null)
      if (!readOnly) onNewNode(kind, null)
    },
    [onNewNode, readOnly],
  )

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

      <div className="nav-view-switch" role="group" aria-label="Navigator view">
        <button aria-pressed={view === 'tree'} onClick={() => setView('tree')}>
          Tree
        </button>
        <button aria-pressed={view === 'graph'} onClick={() => setView('graph')}>
          Graph
        </button>
      </div>

      {view === 'tree' && pinned.length > 0 && (
        <div className="nav-pinned">
          {pinned.map((n) => {
            const def = kinds.get(n.kind)
            return (
              <button
                key={n.id}
                className={`node node-pin${selectedId === n.id ? ' is-sel' : ''}${dropId === n.id ? ' drop-target' : ''}`}
                aria-current={selectedId === n.id ? 'true' : undefined}
                onClick={() => select(n.id)}
                onContextMenu={(e) => {
                  e.preventDefault()
                  e.currentTarget.focus()
                  setCtx({ x: e.clientX, y: e.clientY, node: n, opener: e.currentTarget })
                }}
                title={n.summary || labelFor(def, n.kind)}
                onDragOver={(event) => {
                  if (readOnly || !onAssetDrop || !hasBoardAsset(event.dataTransfer)) return
                  event.preventDefault()
                  event.dataTransfer.dropEffect = 'link'
                  setDropId(n.id)
                }}
                onDragLeave={() => setDropId((id) => (id === n.id ? null : id))}
                onDrop={(event) => {
                  const assetId = boardAssetFrom(event.dataTransfer)
                  if (readOnly || !onAssetDrop || !assetId) return
                  event.preventDefault()
                  onAssetDrop(assetId, n.id)
                  setDropId(null)
                }}
              >
                <Icon
                  name={spriteFor(def, n.kind)}
                  size="sm"
                  style={{ color: colorFor(def, n.kind) }}
                />
                <span className="nm">{n.name}</span>
                <PeerDot who={editedElsewhere.get(n.id)} />
                <StaleDot state={n.descriptionState} />
              </button>
            )
          })}
        </div>
      )}

      {view === 'tree' ? (
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

          {/*
          The filter stays name-and-summary only; the palette is the search
          surface. This is a decision, not an omission (#12).

          The navigator renders a *tree*, and `filterTree` keeps the ancestors
          of anything that matches. Feeding it hits from notes would expand a
          branch and highlight a row for a reason that is nowhere on screen —
          the user sees a species surface because of a word buried three
          paragraphs into its notes, with no way to tell why. Narrowing what is
          in front of you and finding something you half-remember are different
          jobs, and the palette does the second one honestly, with a heading
          that says which half of the search found each row.
        */}
          {filter && !list.hasMatches && (
            <p className="nav-note">
              Nothing here matches <b>{filter}</b>. This box filters names and summaries — press{' '}
              <kbd>Ctrl+K</kbd> to search inside notes and descriptions too.
            </p>
          )}
          <VirtualNavigatorRows
            rows={list.rows}
            kinds={kinds}
            selectedId={selectedId}
            dragId={dragId}
            dropId={dropId}
            readOnly={readOnly}
            editedElsewhere={editedElsewhere}
            onSelect={select}
            onToggle={toggleNodeOpen}
            onToggleGroup={toggleGroup}
            onGroupContext={handleGroupContext}
            onContext={handleContext}
            onDragStart={setDragId}
            onDragEnd={handleDragEnd}
            canDrop={canDrop}
            onDropOn={doMove}
            setDropId={setDropId}
            onAssetDrop={onAssetDrop ? handleAssetDrop : undefined}
            onRowRender={onRowRender}
          />
        </div>
      ) : (
        <div className="nav-graph-shell">
          <BrokenFiles files={corrupt} projectPath={projectPath} />
          <RelationshipGraph
            nodes={nodes}
            links={linksQ.data ?? []}
            kinds={kinds}
            selectedId={selectedId}
            filter={filter}
            loading={loading || linksQ.isPending}
            error={error ?? (linksQ.isError ? errorMessage(linksQ.error) : null)}
            onSelect={select}
            readOnly={readOnly}
            onAssetDrop={onAssetDrop}
          />
        </div>
      )}

      {view === 'tree' && (
        <div className="nav-actions">
          <button className="nav-new" onClick={() => onNewNode(null, null)} disabled={readOnly}>
            <Icon name="plus" size="sm" />
            New entity
          </button>
          {onStyleTransfer && (
            <button className="nav-import" onClick={onStyleTransfer} disabled={readOnly}>
              Import style/subtree…
            </button>
          )}
        </div>
      )}

      {ctx && (
        <ContextMenu
          x={ctx.x}
          y={ctx.y}
          onClose={() => setCtx(null)}
          restoreFocus={ctx.opener}
          label={`Actions for ${ctx.node.name}`}
        >
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

/**
 * The unresolved-stale marker, on pinned rows and tree rows alike.
 *
 * `stale` reaches here derived rather than stored: the backend compares what a
 * description was enhanced from against the world as it stands, so an edit to
 * the Style Guide lights up a hundred of these without a hundred files being
 * rewritten (#38). Nothing here re-enhances — the dot is an offer, and
 * regenerating somebody's description because they clicked a row is the one
 * behaviour the whole state machine exists to prevent.
 *
 * The title says *why* rather than saying "stale". "Stale" names the state the
 * developer sees; the user needs to know something upstream moved, or the dot
 * is just a thing that appears.
 */
function StaleDot({ state }: { state: NodeSummary['descriptionState'] }) {
  if (state !== 'stale') return null
  return (
    <span
      className="stale"
      role="img"
      aria-label="Description is out of date"
      title="Notes or an upstream influence changed since this was enhanced"
    />
  )
}

/**
 * Somebody else has this node open.
 *
 * Quiet on purpose, and quieter than the stale dot beside it. This marks a
 * *coincidence*, not a problem: both of you can type, both of you can save, and
 * a save that loses the race is parked as a conflict file rather than lost. A
 * marker that read as a warning would teach people to wait for a row to clear,
 * which is the hard-lock behaviour presence exists to avoid (`docs/07-file-shares.md`).
 *
 * `who` is absent for almost every row, which is also why it is the prop: a
 * boolean would need a second lookup to say a name in the tooltip, and a dot
 * with no name attached is just a mark.
 */
function PeerDot({ who }: { who: string | undefined }) {
  if (!who) return null
  return (
    <span className="peer" role="img" aria-label={editingTitle(who)} title={editingTitle(who)} />
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
      <div className="ctx-label" role="presentation">
        {labelFor(def, node.kind)}
      </div>
      <button
        role="menuitem"
        disabled={readOnly}
        onClick={pick(() => onNewNode(node.kind, node.parentId))}
      >
        <Icon name="plus" size="sm" />
        New {labelFor(def, node.kind).toLowerCase()}
      </button>
      {def?.nests && (
        <button
          role="menuitem"
          disabled={readOnly}
          onClick={pick(() => onNewNode(node.kind, node.id))}
        >
          <Icon name="plus" size="sm" />
          New child of {node.name}
        </button>
      )}
      <div className="ctx-sep" role="separator" />
      <button
        role="menuitem"
        disabled={readOnly || def?.singleton || busy}
        onClick={pick(onDuplicate)}
      >
        <Icon name="copy" size="sm" />
        Duplicate
      </button>
      <button
        role="menuitem"
        className="danger"
        disabled={readOnly || busy}
        onClick={pick(onDelete)}
      >
        <Icon name="trash" size="sm" />
        Delete
      </button>
    </>
  )
}

const NAVIGATOR_ROW_HEIGHT = 28
const NAVIGATOR_OVERSCAN = 8
const NAVIGATOR_FALLBACK_HEIGHT = 560

interface NavigatorRowsProps {
  rows: NavigatorListRow[]
  kinds: KindIndex
  selectedId: string | null
  dragId: string | null
  dropId: string | null
  readOnly: boolean
  editedElsewhere: Map<string, string>
  onSelect: (id: string) => void
  onToggle: (id: string) => void
  onToggleGroup: (kind: NodeKind) => void
  onGroupContext: (kind: NodeKind) => void
  onContext: (x: number, y: number, node: NodeSummary, opener: HTMLButtonElement) => void
  onDragStart: (id: string) => void
  onDragEnd: () => void
  canDrop: (targetId: string | null, kind: NodeKind) => boolean
  onDropOn: (id: string | null) => void
  setDropId: (id: string | null) => void
  onAssetDrop?: (assetId: string, nodeId: string) => void
  onRowRender?: (nodeId: string) => void
}

function VirtualNavigatorRows(props: NavigatorRowsProps) {
  const viewport = useRef<HTMLDivElement>(null)
  const scrollFrame = useRef<number | null>(null)
  const latestScrollTop = useRef(0)
  const [windowState, setWindowState] = useState({
    scrollTop: 0,
    height: NAVIGATOR_FALLBACK_HEIGHT,
  })

  useEffect(() => {
    const element = viewport.current
    if (!element) return
    const measure = () =>
      setWindowState((current) => ({
        ...current,
        height: element.clientHeight || NAVIGATOR_FALLBACK_HEIGHT,
      }))
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  useEffect(
    () => () => {
      if (scrollFrame.current !== null) cancelAnimationFrame(scrollFrame.current)
    },
    [],
  )

  useEffect(() => {
    if (!props.selectedId) return
    const index = props.rows.findIndex(
      (row) => row.type === 'node' && row.tree.node.id === props.selectedId,
    )
    const element = viewport.current
    if (index < 0 || !element) return
    const top = index * NAVIGATOR_ROW_HEIGHT
    const bottom = top + NAVIGATOR_ROW_HEIGHT
    const visibleTop = element.scrollTop
    const visibleBottom = visibleTop + (element.clientHeight || NAVIGATOR_FALLBACK_HEIGHT)
    if (top >= visibleTop && bottom <= visibleBottom) return
    const next = Math.max(0, top - NAVIGATOR_ROW_HEIGHT * 2)
    element.scrollTop = next
    latestScrollTop.current = next
    setWindowState((current) => ({ ...current, scrollTop: next }))
  }, [props.rows, props.selectedId])

  const start = Math.max(
    0,
    Math.floor(windowState.scrollTop / NAVIGATOR_ROW_HEIGHT) - NAVIGATOR_OVERSCAN,
  )
  const end = Math.min(
    props.rows.length,
    Math.ceil((windowState.scrollTop + windowState.height) / NAVIGATOR_ROW_HEIGHT) +
      NAVIGATOR_OVERSCAN,
  )

  return (
    <div
      className="nav-tree-scroll"
      ref={viewport}
      onScroll={(event) => {
        latestScrollTop.current = event.currentTarget.scrollTop
        if (scrollFrame.current !== null) return
        scrollFrame.current = requestAnimationFrame(() => {
          scrollFrame.current = null
          setWindowState((current) => ({
            ...current,
            scrollTop: latestScrollTop.current,
          }))
        })
      }}
    >
      <div
        className="nav-virtual-space"
        style={{ height: props.rows.length * NAVIGATOR_ROW_HEIGHT }}
      >
        <div
          className="nav-virtual-window"
          style={{ transform: `translateY(${start * NAVIGATOR_ROW_HEIGHT}px)` }}
        >
          {props.rows
            .slice(start, end)
            .map((row) =>
              row.type === 'group' ? (
                <NavigatorGroupRow
                  key={row.key}
                  row={row}
                  dropTarget={props.dropId === groupDropId(row.group.kind)}
                  canDrop={props.canDrop}
                  onToggle={props.onToggleGroup}
                  onContext={props.onGroupContext}
                  onDropOn={props.onDropOn}
                  setDropId={props.setDropId}
                />
              ) : (
                <NavigatorNodeRow
                  key={row.key}
                  tree={row.tree}
                  kinds={props.kinds}
                  open={row.open}
                  hasChildren={row.hasChildren}
                  selected={props.selectedId === row.tree.node.id}
                  dragging={props.dragId === row.tree.node.id}
                  dropTarget={props.dropId === row.tree.node.id}
                  onSelect={props.onSelect}
                  onToggle={props.onToggle}
                  onContext={props.onContext}
                  onDragStart={props.onDragStart}
                  onDragEnd={props.onDragEnd}
                  canDrop={props.canDrop}
                  onDropOn={props.onDropOn}
                  setDropId={props.setDropId}
                  readOnly={props.readOnly}
                  who={props.editedElsewhere.get(row.tree.node.id)}
                  onAssetDrop={props.onAssetDrop}
                  onRender={props.onRowRender}
                />
              ),
            )}
        </div>
      </div>
    </div>
  )
}

const NavigatorGroupRow = memo(function NavigatorGroupRow({
  row,
  dropTarget,
  canDrop,
  onToggle,
  onContext,
  onDropOn,
  setDropId,
}: {
  row: Extract<NavigatorListRow, { type: 'group' }>
  dropTarget: boolean
  canDrop: (targetId: string | null, kind: NodeKind) => boolean
  onToggle: (kind: NodeKind) => void
  onContext: (kind: NodeKind) => void
  onDropOn: (id: string | null) => void
  setDropId: (id: string | null) => void
}) {
  const group = row.group
  const target = groupDropId(group.kind)
  return (
    <div className={row.open ? 'group open' : 'group'}>
      <button
        className={`group-h${dropTarget ? ' drop-target' : ''}`}
        onClick={() => onToggle(group.kind)}
        onDragOver={(event) => {
          if (!canDrop(null, group.kind)) return
          event.preventDefault()
          setDropId(target)
        }}
        onDragLeave={() => setDropId(null)}
        onDrop={(event) => {
          if (!canDrop(null, group.kind)) return
          event.preventDefault()
          onDropOn(null)
        }}
        onContextMenu={(event) => {
          event.preventDefault()
          onContext(group.kind)
        }}
      >
        <Icon name="chev" />
        {pluralFor(group.def, group.kind)}
        <span className="gcount">{group.count}</span>
      </button>
    </div>
  )
})

const NavigatorNodeRow = memo(function NavigatorNodeRow({
  tree,
  kinds,
  open,
  hasChildren,
  selected,
  dragging,
  dropTarget,
  onSelect,
  onToggle,
  onContext,
  onDragStart,
  onDragEnd,
  canDrop,
  onDropOn,
  setDropId,
  readOnly,
  who,
  onAssetDrop,
  onRender,
}: {
  tree: TreeNode
  kinds: KindIndex
  open: boolean
  hasChildren: boolean
  selected: boolean
  dragging: boolean
  dropTarget: boolean
  onSelect: (id: string) => void
  onToggle: (id: string) => void
  onContext: (x: number, y: number, node: NodeSummary, opener: HTMLButtonElement) => void
  onDragStart: (id: string) => void
  onDragEnd: () => void
  canDrop: (targetId: string | null, kind: NodeKind) => boolean
  onDropOn: (id: string | null) => void
  setDropId: (id: string | null) => void
  readOnly: boolean
  who: string | undefined
  onAssetDrop?: (assetId: string, nodeId: string) => void
  onRender?: (nodeId: string) => void
}) {
  const n = tree.node
  useEffect(() => {
    onRender?.(n.id)
  })
  const def = kinds.get(n.kind)
  const cls = [
    'node',
    selected ? 'is-sel' : '',
    dragging ? 'is-dragging' : '',
    dropTarget ? 'drop-target' : '',
  ]
    .filter(Boolean)
    .join(' ')

  return (
    <button
      className={cls}
      aria-current={selected ? 'true' : undefined}
      style={{ paddingLeft: 12 + tree.depth * 14 }}
      onClick={() => onSelect(n.id)}
      onContextMenu={(e) => {
        e.preventDefault()
        e.currentTarget.focus()
        onContext(e.clientX, e.clientY, n, e.currentTarget)
      }}
      draggable={!readOnly}
      onDragStart={(e) => {
        e.dataTransfer.setData(DRAG_MIME, n.id)
        e.dataTransfer.effectAllowed = 'move'
        onDragStart(n.id)
      }}
      onDragEnd={onDragEnd}
      onDragOver={(e) => {
        if (!readOnly && onAssetDrop && hasBoardAsset(e.dataTransfer)) {
          e.preventDefault()
          e.dataTransfer.dropEffect = 'link'
          setDropId(n.id)
          return
        }
        if (!canDrop(n.id, n.kind)) return
        e.preventDefault()
        e.dataTransfer.dropEffect = 'move'
        setDropId(n.id)
      }}
      onDragLeave={() => setDropId(null)}
      onDrop={(e) => {
        const assetId = boardAssetFrom(e.dataTransfer)
        if (!readOnly && onAssetDrop && assetId) {
          e.preventDefault()
          onAssetDrop(assetId, n.id)
          setDropId(null)
          return
        }
        if (!canDrop(n.id, n.kind)) return
        e.preventDefault()
        onDropOn(n.id)
      }}
      title={n.summary || n.name}
    >
      {hasChildren ? (
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
      <PeerDot who={who} />
      <StaleDot state={n.descriptionState} />
    </button>
  )
})

function hasBoardAsset(dataTransfer: DataTransfer): boolean {
  return Array.from(dataTransfer.types).includes(BOARD_ASSET_MIME)
}

function boardAssetFrom(dataTransfer: DataTransfer): string | null {
  if (!hasBoardAsset(dataTransfer)) return null
  return dataTransfer.getData(BOARD_ASSET_MIME) || null
}
