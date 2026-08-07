import { useCallback, useEffect, useMemo, useState } from 'react'
import type { CorruptFile, NodeKind, NodeSummary } from '../../lib/api'
import { useDeleteNode, useDuplicateNode, useMoveNode } from '../../lib/queries'
import { colorFor, labelFor, spriteFor, type KindIndex } from '../../lib/kinds'
import {
  bucketLetter,
  bucketOf,
  bucketRoots,
  descendantsOf,
  type AlphaBucket,
  type KindGroup,
} from '../../lib/tree'
import { canDrop as allow } from '../../lib/drop'
import { useFavourites } from '../../lib/favourites'
import { useNodeThumbs } from '../../lib/nodeThumbs'
import { useUI, report, toast } from '../../store/ui'
import { NodeThumbnail } from '../AssetMedia'
import { Icon } from '../Icon'
import { IconButton, TipButton, Tooltip } from '../Tooltip'
import { ContextMenu } from '../ContextMenu'
import { useContextMenu } from '../../hooks/useContextMenu'
import { ConfirmSheet } from '../ConfirmSheet'
import { BrokenFiles } from './BrokenFiles'
import { RECENT_MIN_WORLD, bucketBand, buildNavigatorRows } from './navigatorRows'
import { GroupMenu, NodeMenu } from './menus'
import { deleteWarning } from './deleteWarning'
import { PeerDot, StaleDot, VirtualNavigatorRows } from './rows'
import { READ_ONLY_REASON } from './constants'

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
                ? `${src?.name ?? 'Entity'} moved under ${byId.get(targetId)?.name ?? 'another entity'}`
                : `${src?.name ?? 'Entity'} moved to the top level`,
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
        {error && <p className="nav-note">Could not list this world’s entities — {error}</p>}
        {!loading && !error && nodes.length === 0 && (
          <p className="nav-note">
            This project has no entities yet. <b>New entity</b> writes the first Markdown file into{' '}
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
          tip="Write a new entity into this project as a Markdown file"
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
            Import from another project…
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
                toast('Entity deleted')
              },
            })
            setConfirm(null)
          }}
        />
      )}
    </aside>
  )
}
