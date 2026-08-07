import { memo, useEffect, useRef, useState } from 'react'
import type { NodeKind, NodeSummary } from '../../lib/api'
import { colorFor, pluralFor, spriteFor, type KindIndex } from '../../lib/kinds'
import { type KindGroup, type TreeNode } from '../../lib/tree'
import { editingTitle } from '../../lib/presence'
import { useNodeThumbs } from '../../lib/nodeThumbs'
import { NodeThumbnail } from '../AssetMedia'
import { Icon } from '../Icon'
import { Tooltip } from '../Tooltip'
import { type MenuTriggerProps } from '../../hooks/useContextMenu'
import { groupDropId, type NavigatorListRow, type NavigatorPlace } from './navigatorRows'
import { DRAG_MIME } from './constants'

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
export function StaleDot({ state }: { state: NodeSummary['descriptionState'] }) {
  if (state !== 'stale') return null
  return (
    <Tooltip tip="Notes, or something this inherits from, changed since this was enhanced">
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
export function PeerDot({ who }: { who: string | undefined }) {
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
export function Star({ on }: { on?: boolean }) {
  return <span className={on ? 'star is-on' : 'star'} aria-hidden />
}

export const NAVIGATOR_ROW_HEIGHT = 28

export const NAVIGATOR_OVERSCAN = 8

export const NAVIGATOR_FALLBACK_HEIGHT = 560

/**
 * Everything a node row can do, named once.
 *
 * The window holds these to hand down and the row holds them to call, so
 * spelling the same nine signatures out twice is how one of them ends up
 * differing from the other by a parameter nobody notices.
 */
export interface NavigatorRowActions {
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

export interface NavigatorRowsProps extends NavigatorRowActions {
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

export function VirtualNavigatorRows(props: NavigatorRowsProps) {
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

export const NavigatorGroupRow = memo(function NavigatorGroupRow({
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
export const NavigatorBandRow = memo(function NavigatorBandRow({
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

export const NavigatorNodeRow = memo(function NavigatorNodeRow({
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
