import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CorruptFile, NodeKind, NodeSummary } from '../../lib/api'
import { useDeleteNode, useDuplicateNode, useMoveNode } from '../../lib/queries'
import { colorFor, labelFor, pluralFor, spriteFor, type KindIndex } from '../../lib/kinds'
import {
  bucketLetter,
  bucketOf,
  bucketRoots,
  descendantsOf,
  type AlphaBucket,
  type KindGroup,
  type TreeNode,
} from '../../lib/tree'
import { canDrop as allow } from '../../lib/drop'
import { useFavourites } from '../../lib/favourites'
import { editingTitle } from '../../lib/presence'
import { useNodeThumbs } from '../../lib/nodeThumbs'
import { useUI, report, toast } from '../../store/ui'
import { NodeThumbnail } from '../AssetMedia'
import { Icon } from '../Icon'
import { IconButton, TipButton, Tooltip } from '../Tooltip'
import { ContextMenu, MenuItem, MenuLabel, MenuSeparator } from '../ContextMenu'
import { useContextMenu, type MenuTriggerProps } from '../../hooks/useContextMenu'
import { ConfirmSheet } from '../ConfirmSheet'
import { BrokenFiles } from './BrokenFiles'
import {
  RECENT_MIN_WORLD,
  bucketBand,
  buildNavigatorRows,
  groupDropId,
  type NavigatorListRow,
  type NavigatorPlace,
} from './navigatorRows'

const DRAG_MIME = 'application/x-wobu-node'

/**
 * Why every writing control in the navigator is refused at once.
 *
 * Said in one place because a user who meets it on one button and then another
 * should be told the same thing, and because it is a *precondition* — it names
 * what would have to change for the button to work — rather than a restatement
 * of the fact that the button does not work.
 */
const READ_ONLY_REASON =
  'This project is open read-only: Wobu cannot write to the folder, so nothing can be created here. Check the folder permissions, or reopen a copy somewhere writable.'

/**
 * A shared empty list, so a project with no favourites and no history hands the
 * row builder the *same* array on every render rather than a new one. Without
 * it, selecting a node would rebuild ten thousand rows to arrive at the list it
 * already had.
 */
const NONE: NodeSummary[] = []

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
  const bands = useUI((s) => s.bands)
  const setBandOpen = useUI((s) => s.setBandOpen)
  const recentIds = useUI((s) => s.recentIds)

  const nodeMenu = useContextMenu<NodeSummary>()
  const groupMenu = useContextMenu<KindGroup>()
  const [confirm, setConfirm] = useState<NodeSummary | null>(null)
  const [dragId, setDragId] = useState<string | null>(null)
  const [dropId, setDropId] = useState<string | null>(null)

  const move = useMoveNode()
  const del = useDeleteNode()
  const dup = useDuplicateNode()
  const moveNode = move.mutate

  const forbidden = useMemo(
    () => (dragId ? descendantsOf(dragId, nodes) : new Set<string>()),
    [dragId, nodes],
  )

  const favouriteIdList = useFavourites((s) => s.byProject[projectPath])
  const toggleFavourite = useFavourites((s) => s.toggle)
  const favouriteIds = useMemo(() => new Set(favouriteIdList ?? []), [favouriteIdList])

  /*
   * Favourites read alphabetically rather than in the order they were starred.
   *
   * Starring is not a ranking — people star what they are working on, over
   * weeks, in no order at all — and a list that reshuffles whenever somebody
   * adds one is a list you have to re-read every time. Sorted, the row a reader
   * has clicked forty times stays where their hand expects it.
   */
  const favourites = useMemo(() => {
    if (!favouriteIdList?.length) return NONE
    const list = favouriteIdList
      .map((id) => byId.get(id))
      .filter((node): node is NodeSummary => node !== undefined)
    list.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }))
    return list.length ? list : NONE
  }, [byId, favouriteIdList])

  // The node you are looking at is not a place you might want to go back to, so
  // the selection is left out. It also keeps a click from adding a row: the
  // section only grows when the reader moves *on*.
  const recents = useMemo(() => {
    if (nodes.length < RECENT_MIN_WORLD) return NONE
    const list = recentIds
      .filter((id) => id !== selectedId)
      .map((id) => byId.get(id))
      .filter((node): node is NodeSummary => node !== undefined)
    return list.length ? list : NONE
  }, [byId, nodes.length, recentIds, selectedId])

  /** The index each oversized kind is drawn with (see lib/tree.ts). */
  const indexed = useMemo(() => {
    const map = new Map<NodeKind, AlphaBucket[]>()
    for (const group of groups) {
      const buckets = bucketRoots(group.roots)
      if (buckets) map.set(group.kind, buckets)
    }
    return map
  }, [groups])

  /*
   * The heading the selection is filed under, or `null` when it is not behind
   * one. Derived rather than searched for: it yields the *same string* while
   * the reader moves around inside one heading, which is what keeps the effect
   * below from firing — and the row list from being rebuilt — on every click.
   */
  const selectedBand = useMemo(() => {
    const node = selectedId ? byId.get(selectedId) : undefined
    const buckets = node ? indexed.get(node.kind) : undefined
    if (!node || !buckets) return null
    // Roots of a kind group are the nodes whose parent is outside the kind, so
    // that is where the walk stops — the same rule `nest()` uses to build them.
    let root = node
    const seen = new Set([root.id])
    while (root.parentId) {
      const parent = byId.get(root.parentId)
      if (!parent || parent.kind !== root.kind || seen.has(parent.id)) break
      seen.add(parent.id)
      root = parent
    }
    const from = bucketOf(buckets, bucketLetter(root.name))
    return from ? bucketBand(node.kind, from) : null
  }, [byId, indexed, selectedId])

  /*
   * Opening a node from somewhere else — the palette, a breadcrumb, a backlink,
   * a freshly created entity — has to land on a row that exists, and inside an
   * indexed group that row can be behind a closed heading. Opening it is a real
   * change to the reader's navigator, kept exactly like the ancestor branches
   * `openAncestors` opens on the same journey: the way back is left open.
   */
  useEffect(() => {
    if (!selectedBand) return
    const ui = useUI.getState()
    if (ui.bands[selectedBand] !== true) ui.setBandOpen(selectedBand, true)
  }, [selectedBand])

  const list = useMemo(
    () =>
      buildNavigatorRows({
        groups,
        filter,
        closedGroups,
        collapsedNodes,
        bands,
        favourites,
        recents,
      }),
    [bands, closedGroups, collapsedNodes, favourites, filter, groups, recents],
  )

  const allClosed = groups.length > 0 && groups.every((group) => closedGroups[group.kind])
  const collapseEverything = useCallback(() => {
    const state = useUI.getState()
    if (allClosed) {
      state.expandAll()
      return
    }
    state.collapseAll(
      groups.map((group) => group.kind),
      list.rows.flatMap((row) => (row.type === 'band' ? [row.key] : [])),
    )
  }, [allClosed, groups, list.rows])

  const handleFavourite = useCallback(
    (id: string) => toggleFavourite(projectPath, id),
    [projectPath, toggleFavourite],
  )
  const handleBand = useCallback(
    (key: string, open: boolean) => setBandOpen(key, open),
    [setBandOpen],
  )

  // The pinned strip is its own short, unvirtualized list, so it asks for its
  // own ids; they join whatever the tree window asks for in the same batch.
  const pinnedThumbs = useNodeThumbs(useMemo(() => pinned.map((n) => n.id), [pinned]))

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

  const handleDragEnd = useCallback(() => {
    setDragId(null)
    setDropId(null)
  }, [])

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
          <IconButton className="clear" label="Clear filter" onClick={() => setFilter('')}>
            <Icon name="x" size="sm" />
          </IconButton>
        )}
      </div>

      {/*
        The size of the world, said out loud.

        A navigator that has been restructured into sections and letters can no
        longer be sized by eye — the reader sees eleven headings and has no idea
        whether that is forty entities or nine hundred. While a filter is
        narrowing, the same line becomes the answer to "did that find anything,
        and how much", which is otherwise only knowable by scrolling.
      */}
      {nodes.length > 0 && (
        <div className="nav-tools">
          <span className="nav-count">
            {filter ? `${list.shown} of ${nodes.length} shown` : `${nodes.length} entities`}
          </span>
          <TipButton
            className="nav-tool"
            onClick={collapseEverything}
            disabledReason={
              groups.length === 0
                ? 'Nothing to collapse — the filter is narrowing this to a flat list.'
                : null
            }
            tip={
              allClosed
                ? 'Open every group and branch'
                : 'Close every group, keeping what is open inside them'
            }
          >
            {allClosed ? 'Expand all' : 'Collapse all'}
          </TipButton>
        </div>
      )}

      {pinned.length > 0 && (
        <div className="nav-pinned">
          {pinned.map((n) => {
            const def = kinds.get(n.kind)
            return (
              <Tooltip key={n.id} tip={n.summary || labelFor(def, n.kind)} placement="right">
                <button
                  className={`node node-pin${selectedId === n.id ? ' is-sel' : ''}${dropId === n.id ? ' drop-target' : ''}`}
                  aria-current={selectedId === n.id ? 'true' : undefined}
                  onClick={() => select(n.id)}
                  {...nodeMenu.trigger(n)}
                >
                  <NodeThumbnail
                    path={pinnedThumbs.get(n.id)}
                    fallback={
                      <Icon
                        name={spriteFor(def, n.kind)}
                        size="sm"
                        style={{ color: colorFor(def, n.kind) }}
                      />
                    }
                  />
                  <span className="nm">{n.name}</span>
                  <PeerDot who={editedElsewhere.get(n.id)} />
                  <StaleDot state={n.descriptionState} />
                </button>
              </Tooltip>
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
          favourites={favouriteIds}
          onSelect={select}
          onToggle={toggleNodeOpen}
          onToggleGroup={toggleGroup}
          onToggleBand={handleBand}
          groupTrigger={groupMenu.trigger}
          trigger={nodeMenu.trigger}
          onFavourite={handleFavourite}
          onDragStart={setDragId}
          onDragEnd={handleDragEnd}
          canDrop={canDrop}
          onDropOn={doMove}
          setDropId={setDropId}
          onRowRender={onRowRender}
        />
      </div>

      {/*
        Read-only is the disabled state a user is most likely to meet and least
        likely to understand: the button is simply grey, in a project that
        opened normally. Both of these now say which of the two causes it is —
        the folder's permissions, or someone else holding the write lock.
      */}
      <div className="nav-actions">
        <TipButton
          className="nav-new"
          onClick={() => onNewNode(null, null)}
          disabledReason={readOnly ? READ_ONLY_REASON : null}
          tip="Write a new Markdown file into nodes/"
        >
          <Icon name="plus" size="sm" />
          New entity
        </TipButton>
        {onStyleTransfer && (
          <TipButton
            className="nav-import"
            onClick={onStyleTransfer}
            disabledReason={readOnly ? READ_ONLY_REASON : null}
            tip="Copy a style, or a whole branch, out of another project"
          >
            Import style/subtree…
          </TipButton>
        )}
      </div>

      {nodeMenu.anchor && (
        <ContextMenu
          x={nodeMenu.anchor.x}
          y={nodeMenu.anchor.y}
          onClose={nodeMenu.close}
          restoreFocus={nodeMenu.anchor.opener}
          label={`Actions for ${nodeMenu.anchor.item.name}`}
        >
          <NodeMenu
            node={nodeMenu.anchor.item}
            kinds={kinds}
            readOnly={readOnly}
            busy={dup.isPending || del.isPending}
            favourite={favouriteIds.has(nodeMenu.anchor.item.id)}
            onFavourite={handleFavourite}
            onNewNode={onNewNode}
            onDuplicate={(id) =>
              dup.mutate(id, {
                onError: (e) => report(e),
                onSuccess: (n) => {
                  select(n.id)
                  toast(`Duplicated as “${n.name}”`)
                },
              })
            }
            onDelete={setConfirm}
          />
        </ContextMenu>
      )}

      {/*
        A heading's menu, rather than the heading's right-click *being* the
        action. Right-clicking a group used to open the new-entity sheet with no
        menu in between, which is the one gesture on this pane that did
        something irreversible-looking without asking — and it left the group's
        other action, the one bound to a chord, reachable only from a button at
        the top of the pane.
      */}
      {groupMenu.anchor && (
        <GroupMenu
          group={groupMenu.anchor.item}
          x={groupMenu.anchor.x}
          y={groupMenu.anchor.y}
          opener={groupMenu.anchor.opener}
          readOnly={readOnly}
          allClosed={allClosed}
          onClose={groupMenu.close}
          onNewNode={onNewNode}
          onToggleAll={collapseEverything}
        />
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
    <Tooltip tip="Notes or an upstream influence changed since this was enhanced">
      <span className="stale" role="img" aria-label="Description is out of date" />
    </Tooltip>
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
    <Tooltip tip={editingTitle(who)}>
      <span className="peer" role="img" aria-label={editingTitle(who)} />
    </Tooltip>
  )
}

/**
 * The favourite mark, drawn in CSS rather than from the sprite sheet.
 *
 * Two reasons, and neither is aesthetic. The sheet has no star, and it lives in
 * `IconSprite.tsx`, which is shared by every surface in the app. And an
 * `<Icon>` is two elements — an `<svg>` and a `<use>` — where this is one,
 * which matters on the navigator row that carries the same mark: a full window
 * of rows pays that cost on every scroll tick.
 */
function Star({ on }: { on?: boolean }) {
  return <span className={on ? 'star is-on' : 'star'} aria-hidden />
}

function deleteWarning(node: NodeSummary, nodes: NodeSummary[]): string {
  const kids = descendantsOf(node.id, nodes).size
  const base = 'Its Markdown file is removed from the project folder.'
  if (kids === 0) return base
  return `${base} ${kids} node${kids === 1 ? '' : 's'} nest inside it — how the backend treats them is its call, so check the tree afterwards.`
}

function GroupMenu({
  group,
  x,
  y,
  opener,
  readOnly,
  allClosed,
  onClose,
  onNewNode,
  onToggleAll,
}: {
  group: KindGroup
  x: number
  y: number
  opener: HTMLElement
  readOnly: boolean
  allClosed: boolean
  onClose: () => void
  onNewNode: (kind: NodeKind | null, parentId: string | null) => void
  onToggleAll: () => void
}) {
  const plural = pluralFor(group.def, group.kind)
  return (
    <ContextMenu
      x={x}
      y={y}
      onClose={onClose}
      restoreFocus={opener}
      label={`Actions for ${plural}`}
    >
      <MenuLabel>{plural}</MenuLabel>
      <MenuItem
        icon={<Icon name="plus" size="sm" />}
        disabledReason={readOnly ? READ_ONLY_REASON : null}
        onSelect={() => onNewNode(group.kind, null)}
      >
        New {labelFor(group.def, group.kind).toLowerCase()}
      </MenuItem>
      <MenuSeparator />
      {/* The chord is printed, not typed: this is the same action as the button
          in `.nav-tools` and the same command the dispatcher runs, so the row
          says whatever the user has bound it to today. */}
      <MenuItem
        icon={<Icon name="chev" size="sm" />}
        command="nav.toggleAll"
        onSelect={onToggleAll}
      >
        {allClosed ? 'Expand everything' : 'Collapse everything'}
      </MenuItem>
    </ContextMenu>
  )
}

function NodeMenu({
  node,
  kinds,
  readOnly,
  busy,
  favourite,
  onFavourite,
  onNewNode,
  onDuplicate,
  onDelete,
}: {
  node: NodeSummary
  kinds: KindIndex
  readOnly: boolean
  busy: boolean
  favourite: boolean
  onFavourite: (id: string) => void
  onNewNode: (kind: NodeKind | null, parentId: string | null) => void
  onDuplicate: (id: string) => void
  onDelete: (node: NodeSummary) => void
}) {
  const def = kinds.get(node.kind)
  const busyReason = busy ? 'Another change to this node is still being written.' : null
  return (
    <>
      <MenuLabel>{labelFor(def, node.kind)}</MenuLabel>
      <MenuItem
        icon={<Icon name="plus" size="sm" />}
        disabledReason={readOnly ? READ_ONLY_REASON : null}
        onSelect={() => onNewNode(node.kind, node.parentId)}
      >
        New {labelFor(def, node.kind).toLowerCase()}
      </MenuItem>
      {def?.nests && (
        <MenuItem
          icon={<Icon name="plus" size="sm" />}
          disabledReason={readOnly ? READ_ONLY_REASON : null}
          onSelect={() => onNewNode(node.kind, node.id)}
        >
          New child of {node.name}
        </MenuItem>
      )}
      <MenuSeparator />
      {/* Never disabled by `readOnly`: a favourite is this reader's shortcut,
          held on this machine, and a project on a read-only share is exactly
          the one you most want to keep your bearings in. */}
      <MenuItem icon={<Star on={favourite} />} onSelect={() => onFavourite(node.id)}>
        {favourite ? 'Remove from favourites' : 'Add to favourites'}
      </MenuItem>
      <MenuSeparator />
      <MenuItem
        icon={<Icon name="copy" size="sm" />}
        disabledReason={
          readOnly
            ? READ_ONLY_REASON
            : def?.singleton
              ? `A world has one ${labelFor(def, node.kind).toLowerCase()}, so there is nothing to duplicate it into.`
              : busyReason
        }
        onSelect={() => onDuplicate(node.id)}
      >
        Duplicate
      </MenuItem>
      <MenuItem
        danger
        icon={<Icon name="trash" size="sm" />}
        disabledReason={readOnly ? READ_ONLY_REASON : busyReason}
        onSelect={() => onDelete(node)}
      >
        Delete
      </MenuItem>
    </>
  )
}

const NAVIGATOR_ROW_HEIGHT = 28
const NAVIGATOR_OVERSCAN = 8
const NAVIGATOR_FALLBACK_HEIGHT = 560

/**
 * Everything a node row can do, named once.
 *
 * The window holds these to hand down and the row holds them to call, so
 * spelling the same nine signatures out twice is how one of them ends up
 * differing from the other by a parameter nobody notices.
 */
interface NavigatorRowActions {
  onSelect: (id: string) => void
  onToggle: (id: string) => void
  /** The right-click and Shift+F10 handlers for one row, from `useContextMenu`. */
  trigger: (node: NodeSummary) => MenuTriggerProps
  onFavourite: (id: string) => void
  onDragStart: (id: string) => void
  onDragEnd: () => void
  canDrop: (targetId: string | null, kind: NodeKind) => boolean
  onDropOn: (id: string | null) => void
  setDropId: (id: string | null) => void
}

interface NavigatorRowsProps extends NavigatorRowActions {
  rows: NavigatorListRow[]
  kinds: KindIndex
  selectedId: string | null
  dragId: string | null
  dropId: string | null
  readOnly: boolean
  editedElsewhere: Map<string, string>
  favourites: Set<string>
  onToggleGroup: (kind: NodeKind) => void
  onToggleBand: (key: string, open: boolean) => void
  groupTrigger: (group: KindGroup) => MenuTriggerProps
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
    // The tree row, not the favourite or the recent copy of it: those are
    // shortcuts at the top of the list, and scrolling to one would leave the
    // reader looking at the shortcut instead of the entity's place in the world.
    const index = props.rows.findIndex(
      (row) => row.type === 'node' && row.place === 'tree' && row.tree.node.id === props.selectedId,
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

  const visible = props.rows.slice(start, end)
  /*
   * Thumbnails are asked for by the window, not by the row.
   *
   * This is the whole reason the navigator can carry pictures at all. The rows
   * on screen are a few dozen out of a possible ten thousand, so one call
   * covers everything visible and nothing else — scrolling past a branch never
   * fetches it. A row is then handed a plain string or `null`, which is what
   * lets `NavigatorNodeRow` stay memoized: a node with no picture is passed
   * `null` before and after the batch resolves, so it never re-renders.
   */
  const thumbs = useNodeThumbs(
    visible.filter((row) => row.type === 'node').map((row) => row.tree.node.id),
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
          {visible.map((row) =>
            row.type === 'group' ? (
              <NavigatorGroupRow
                key={row.key}
                row={row}
                dropTarget={props.dropId === groupDropId(row.group.kind)}
                canDrop={props.canDrop}
                onToggle={props.onToggleGroup}
                trigger={props.groupTrigger}
                onDropOn={props.onDropOn}
                setDropId={props.setDropId}
              />
            ) : row.type === 'band' ? (
              <NavigatorBandRow
                key={row.key}
                bandKey={row.key}
                label={row.label}
                count={row.count}
                open={row.open}
                nested={row.nested}
                onToggle={props.onToggleBand}
              />
            ) : (
              <NavigatorNodeRow
                key={row.key}
                tree={row.tree}
                place={row.place}
                kinds={props.kinds}
                open={row.open}
                hasChildren={row.hasChildren}
                selected={props.selectedId === row.tree.node.id}
                dragging={props.dragId === row.tree.node.id}
                dropTarget={props.dropId === row.tree.node.id && row.place === 'tree'}
                favourite={props.favourites.has(row.tree.node.id)}
                onSelect={props.onSelect}
                onToggle={props.onToggle}
                trigger={props.trigger}
                onFavourite={props.onFavourite}
                onDragStart={props.onDragStart}
                onDragEnd={props.onDragEnd}
                canDrop={props.canDrop}
                onDropOn={props.onDropOn}
                setDropId={props.setDropId}
                readOnly={props.readOnly}
                who={props.editedElsewhere.get(row.tree.node.id)}
                thumb={thumbs.get(row.tree.node.id)}
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
  trigger,
  onDropOn,
  setDropId,
}: {
  row: Extract<NavigatorListRow, { type: 'group' }>
  dropTarget: boolean
  canDrop: (targetId: string | null, kind: NodeKind) => boolean
  onToggle: (kind: NodeKind) => void
  trigger: (group: KindGroup) => MenuTriggerProps
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
        {...trigger(group)}
      >
        <Icon name="chev" />
        {pluralFor(group.def, group.kind)}
        <span className="gcount">{group.count}</span>
      </button>
    </div>
  )
})

/**
 * A heading that is neither a kind nor a node — a section, or a letter.
 *
 * One component for both because they are the same control: a disclosure, a
 * title and a count. What separates them is where the row builder puts them and
 * whether they start open, and neither is the row's business. They are
 * deliberately *not* the group header: that one is a drop target for
 * re-parenting to the top level, and dropping a species onto the letter `V`
 * would have to mean something.
 */
const NavigatorBandRow = memo(function NavigatorBandRow({
  bandKey,
  label,
  count,
  open,
  nested,
  onToggle,
}: {
  bandKey: string
  label: string
  count: number
  open: boolean
  nested: boolean
  onToggle: (key: string, open: boolean) => void
}) {
  return (
    <div className={`group band${nested ? ' band-nested' : ''}${open ? ' open' : ''}`}>
      <button className="group-h" aria-expanded={open} onClick={() => onToggle(bandKey, !open)}>
        <Icon name="chev" />
        {label}
        <span className="gcount">{count}</span>
      </button>
    </div>
  )
})

const NavigatorNodeRow = memo(function NavigatorNodeRow({
  tree,
  place,
  kinds,
  open,
  hasChildren,
  selected,
  dragging,
  dropTarget,
  favourite,
  onSelect,
  onToggle,
  trigger,
  onFavourite,
  onDragStart,
  onDragEnd,
  canDrop,
  onDropOn,
  setDropId,
  readOnly,
  who,
  thumb,
  onRender,
}: NavigatorRowActions & {
  tree: TreeNode
  place: NavigatorPlace
  kinds: KindIndex
  open: boolean
  hasChildren: boolean
  selected: boolean
  dragging: boolean
  dropTarget: boolean
  favourite: boolean
  readOnly: boolean
  who: string | undefined
  /** Resolved by the window above; `null` while unknown and when there is none. */
  thumb: string | null
  onRender?: (nodeId: string) => void
}) {
  const n = tree.node
  useEffect(() => {
    onRender?.(n.id)
  })
  const def = kinds.get(n.kind)
  // A shortcut row is a way back to an entity, not the entity's place in the
  // world: re-parenting by dragging one, or dropping onto one, would move a
  // node using a row that says nothing about where it currently sits.
  const inTree = place === 'tree'
  const cls = [
    'node',
    inTree ? '' : 'node-shortcut',
    selected ? 'is-sel' : '',
    dragging ? 'is-dragging' : '',
    dropTarget ? 'drop-target' : '',
  ]
    .filter(Boolean)
    .join(' ')

  return (
    <Tooltip tip={n.summary || null} placement="right">
      <button
        className={cls}
        aria-current={selected ? 'true' : undefined}
        style={{ paddingLeft: 12 + tree.depth * 14 }}
        onClick={() => onSelect(n.id)}
        {...trigger(n)}
        draggable={!readOnly && inTree}
        onDragStart={(e) => {
          e.dataTransfer.setData(DRAG_MIME, n.id)
          e.dataTransfer.effectAllowed = 'move'
          onDragStart(n.id)
        }}
        onDragEnd={onDragEnd}
        onDragOver={(e) => {
          if (!inTree || !canDrop(n.id, n.kind)) return
          e.preventDefault()
          e.dataTransfer.dropEffect = 'move'
          setDropId(n.id)
        }}
        onDragLeave={() => setDropId(null)}
        onDrop={(e) => {
          if (!inTree || !canDrop(n.id, n.kind)) return
          e.preventDefault()
          onDropOn(n.id)
        }}
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
            {/* Sized by the class, not by a literal: the 11px here was one of the
              hand-tuned compensations for the sprite being drawn at 150% and
              cropped, which the `viewBox` in `Icon.tsx` fixed (#128). */}
            <Icon name="chev" size="sm" />
          </span>
        ) : (
          <span className="twist" />
        )}
        <NodeThumbnail
          path={thumb}
          fallback={
            <Icon
              name={spriteFor(def, n.kind)}
              size="sm"
              style={{ color: colorFor(def, n.kind) }}
            />
          }
        />
        <span className="nm">{n.name}</span>
        {/* Invisible until hovered unless it is on, so a thousand rows do not
          read as a thousand stars, and the way to make one is still findable
          without opening a menu to look for it. Not a `<button>`, for the same
          reason the twist beside it is not one: the row itself is the button,
          and the keyboard route to both is the row's context menu.

          Which is also why this is the one control in the sweep whose tooltip
          is hover-only. A nested button inside the row button would be invalid,
          and giving the star its own tab stop would put two of them on every
          one of a thousand rows. The context menu is the keyboard route, and it
          says the same words. */}
        <Tooltip
          tip={favourite ? `Remove ${n.name} from favourites` : `Add ${n.name} to favourites`}
        >
          <span
            className={favourite ? 'fav star is-on' : 'fav star'}
            aria-hidden
            onClick={(e) => {
              e.stopPropagation()
              onFavourite(n.id)
            }}
          />
        </Tooltip>
        <PeerDot who={who} />
        <StaleDot state={n.descriptionState} />
      </button>
    </Tooltip>
  )
})
