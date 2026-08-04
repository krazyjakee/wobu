import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from 'react'
import { createPortal } from 'react-dom'
import {
  filterPrepared,
  prepareOptions,
  sortPrepared,
  type PreparedOption,
} from '../lib/comboboxOptions'
import { Icon } from './Icon'

/**
 * One searchable dropdown for every picker in the application.
 *
 * A native `<select>` is keyboard- and screen-reader-complete for free, and
 * replacing one is a debt that has to be paid back in full before the trade is
 * worth making. It is paid here — the ARIA combobox pattern, arrow/Home/End/
 * PageUp/PageDown movement, Escape that restores rather than commits, focus
 * that never leaves the control — and what it buys is the three things a
 * `<select>` cannot do: type to filter a few thousand entities, a picture per
 * row, and a stated order that ignores leading articles and accents.
 *
 * Sites that gain none of those keep their `<select>`. A five-item enum is not
 * improved by this and would only be made more fragile by it.
 *
 * Shape: an editable `role="combobox"` input owning a `role="listbox"` popup,
 * with `aria-activedescendant` naming the highlighted row. Focus stays on the
 * input from open to close, which is what makes "return focus to the trigger"
 * trivially true and keeps the control a single tab stop, exactly like the
 * `<select>` it replaced.
 */

export interface ComboboxOption {
  value: string
  label: string
  /** Second line on the row — a kind, a count, a caption. Not searched. */
  hint?: string
  /** Searched but never drawn. */
  keywords?: string
  /** Offered and announced, but refuses selection. */
  disabled?: boolean
  /** Thumbnail or kind icon drawn at the head of the row. */
  icon?: ReactNode
  /** Heading this row sits under; rows with no group come first. */
  group?: string
  /** Held at the head of the list by `sort="title"` — the "no choice" row. */
  pinned?: boolean
}

/** Row height in `combobox.css`. Uniform so the window arithmetic is trivial. */
const ROW_HEIGHT = 30
const MAX_LIST_HEIGHT = 320
const OVERSCAN = 6
/**
 * Below this the whole list is drawn. Windowing costs a scroll listener, a
 * spacer and a class of "the row is not in the DOM" bugs; a list that fits in
 * two screenfuls should not pay for any of it.
 */
const VIRTUALIZE_ABOVE = 200

type Row<T extends ComboboxOption> =
  | { kind: 'group'; label: string; key: string }
  | {
      kind: 'option'
      entry: PreparedOption<T>
      index: number
      key: string
      /** Key of the heading above it, so the row can point at it. */
      groupKey?: string
    }

interface Placement {
  left: number
  width: number
  top?: number
  bottom?: number
  maxHeight: number
}

function nextEnabled(options: ComboboxOption[], from: number, step: number): number {
  for (let i = from; i >= 0 && i < options.length; i += step) {
    if (!options[i]?.disabled) return i
  }
  return -1
}

/**
 * The stored index, clamped to the list and nudged off any unselectable row.
 *
 * Derived rather than stored because the list under it moves: filtering, a
 * changed role, a query that resolved. Highlighting a row Enter would refuse is
 * the one state this control must never be caught in, and deriving it means
 * there is no code path that can leave it there.
 */
function resolveActive(options: ComboboxOption[], stored: number): number {
  if (options.length === 0) return -1
  const clamped = Math.min(Math.max(stored, 0), options.length - 1)
  if (!options[clamped]?.disabled) return clamped
  const forward = nextEnabled(options, clamped, 1)
  return forward >= 0 ? forward : nextEnabled(options, clamped, -1)
}

/**
 * Rows of the same group brought together, groups in first-appearance order.
 *
 * Run after filtering as well as after sorting, because the filter re-ranks by
 * how well each row matched and would otherwise scatter a group across the
 * list — which the flat row builder below would draw as the same heading three
 * times. `sort` is stable, so the order inside each group is whatever the step
 * before this one decided.
 */
function groupContiguous<T extends ComboboxOption>(
  entries: PreparedOption<T>[],
): PreparedOption<T>[] {
  if (!entries.some((entry) => entry.option.group)) return entries
  const order = new Map<string, number>()
  for (const entry of entries) {
    const group = entry.option.group ?? ''
    if (!order.has(group)) order.set(group, order.size)
  }
  return [...entries].sort(
    (a, b) => (order.get(a.option.group ?? '') ?? 0) - (order.get(b.option.group ?? '') ?? 0),
  )
}

/**
 * Group headings interleaved with their options, as one flat list.
 *
 * Flat because the window below indexes rows by number, and because a heading
 * and a row are the same height: nesting the options inside `role="group"`
 * elements would be more faithful markup, but it would also make "the row 40
 * screen-pixels down" a tree walk. The heading keeps its `role="presentation"`
 * and each option carries its group in `aria-describedby` instead, so the
 * grouping is still announced.
 */
function buildRows<T extends ComboboxOption>(visible: PreparedOption<T>[]): Row<T>[] {
  const rows: Row<T>[] = []
  let current: string | undefined
  let groupKey: string | undefined
  visible.forEach((entry, index) => {
    const group = entry.option.group
    if (group && group !== current) {
      groupKey = `group:${group}:${index}`
      rows.push({ kind: 'group', label: group, key: groupKey })
    }
    if (!group) groupKey = undefined
    current = group
    rows.push({ kind: 'option', entry, index, key: `option:${entry.option.value}`, groupKey })
  })
  return rows
}

export function Combobox({
  value,
  onChange,
  options,
  id,
  label,
  placeholder = 'Choose…',
  disabled = false,
  className,
  sort = 'none',
  emptyMessage = 'No match.',
  onDrawnRows,
}: {
  value: string
  onChange: (value: string) => void
  options: ComboboxOption[]
  /** Set when a visible `<label for>` points at the control. */
  id?: string
  /** Accessible name, when no visible `<label>` supplies one. */
  label?: string
  placeholder?: string
  disabled?: boolean
  className?: string
  /**
   * `'title'` re-sorts alphabetically, ignoring leading articles and accents.
   * `'none'` — the default — keeps the caller's order, because plenty of these
   * lists are already in an order that means something: aspect ratios ascend,
   * reference roles have a canonical sequence, presets arrive ranked.
   */
  sort?: 'title' | 'none'
  emptyMessage?: string
  /**
   * The values currently drawn, for a caller that has to fetch something per
   * row — a thumbnail, say. Only the rows in the scrolled window are reported,
   * so a picker over three thousand entities asks for twenty pictures rather
   * than three thousand. Must be referentially stable (`useCallback`).
   */
  onDrawnRows?: (values: string[]) => void
}) {
  const reactId = useId()
  const listId = `${reactId}-listbox`
  const optionId = (index: number) => `${reactId}-option-${index}`

  /*
   * The field is held in state, not a ref, so that the portal can be aimed
   * during render. `Modal` marks every body child but its own host `inert`, so
   * a list appended to the document from inside a sheet would be made
   * unclickable by the dialog it belongs to; the host has to be read off the
   * live element, and a ref cannot be read while rendering.
   */
  const [field, setField] = useState<HTMLDivElement | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)

  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const [placement, setPlacement] = useState<Placement | null>(null)
  const [scrollTop, setScrollTop] = useState(0)

  const prepared = useMemo(() => prepareOptions(options), [options])
  const ordered = useMemo(
    () => (sort === 'title' ? sortPrepared(prepared) : prepared),
    [prepared, sort],
  )
  const visible = useMemo(
    () => groupContiguous(open ? filterPrepared(ordered, query) : ordered),
    [ordered, open, query],
  )
  const visibleOptions = useMemo(() => visible.map((entry) => entry.option), [visible])
  const rows = useMemo(() => buildRows(visible), [visible])

  const selected = options.find((option) => option.value === value)
  const activeIndex = resolveActive(visibleOptions, active)
  const activeOption = activeIndex >= 0 ? visibleOptions[activeIndex] : undefined

  /*
   * The popup is a portal, positioned from the field's own box.
   *
   * Drawn in place it would be clipped by the first scrolling ancestor, and
   * every one of these pickers has one: the inspector column, the asset filter
   * bar, the editor panes. Inside a sheet it portals into that sheet's host
   * rather than into `<body>`, because `Modal` marks every other body child
   * `inert` — a list appended to the document would be made unclickable by the
   * very dialog it belongs to.
   *
   * Measured from the act that opens the list and from the scroll and resize
   * that move it, never from an effect: the box is known the moment the user
   * presses, and taking it there costs one render instead of two.
   */
  const measure = useCallback(() => {
    if (!field) return
    const box = field.getBoundingClientRect()
    const below = window.innerHeight - box.bottom
    const above = box.top
    const flip = below < 180 && above > below
    setPlacement({
      left: box.left,
      width: box.width,
      top: flip ? undefined : box.bottom + 4,
      bottom: flip ? window.innerHeight - box.top + 4 : undefined,
      maxHeight: Math.max(120, Math.min(MAX_LIST_HEIGHT, (flip ? above : below) - 12)),
    })
  }, [field])

  const close = useCallback(() => {
    setOpen(false)
    setQuery('')
    // The one place focus is ever moved: back to the control the user was on,
    // never to the document. Escape from a list must not also cost them their
    // place in the form.
    inputRef.current?.focus()
  }, [])

  const openList = useCallback(() => {
    if (disabled || open) return
    setQuery('')
    // Indexed against the *ordered* list, which is what the unfiltered popup
    // draws — the caller's array order is not what the user is about to see.
    const index = ordered.findIndex((entry) => entry.option.value === value)
    setActive(index >= 0 ? index : 0)
    setScrollTop(0)
    measure()
    setOpen(true)
  }, [disabled, measure, open, ordered, value])

  const commit = useCallback(
    (option: ComboboxOption | undefined) => {
      if (!option || option.disabled) return
      onChange(option.value)
      setOpen(false)
      setQuery('')
      inputRef.current?.focus()
    },
    [onChange],
  )

  /*
   * Escape is intercepted on `window`, in the capture phase, and only while the
   * list is open.
   *
   * `Modal` listens for Escape on `document` in capture and stops the event
   * there, so a combobox inside a sheet would never see its own Escape: the
   * first press would close the entire dialog and lose whatever else the user
   * had typed into it. `window` is earlier in the capture path than `document`,
   * which is the only reason this is reachable at all. When the list is closed
   * the listener is gone, so Escape goes back to dismissing the sheet.
   */
  useEffect(() => {
    if (!open) return
    const onKeyDownCapture = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      event.stopPropagation()
      close()
    }
    window.addEventListener('keydown', onKeyDownCapture, true)
    return () => window.removeEventListener('keydown', onKeyDownCapture, true)
  }, [open, close])

  /* A press that lands outside both halves of the control dismisses it. */
  useEffect(() => {
    if (!open) return
    const onDown = (event: MouseEvent) => {
      const target = event.target as Node | null
      if (!target) return
      if (field?.contains(target) || listRef.current?.contains(target)) return
      setOpen(false)
      setQuery('')
    }
    document.addEventListener('mousedown', onDown)
    return () => document.removeEventListener('mousedown', onDown)
  }, [field, open])

  /* Follow the field if the surface under it scrolls or the window resizes. */
  useEffect(() => {
    if (!open) return
    window.addEventListener('resize', measure)
    window.addEventListener('scroll', measure, true)
    return () => {
      window.removeEventListener('resize', measure)
      window.removeEventListener('scroll', measure, true)
    }
  }, [open, measure])

  const virtualized = rows.length > VIRTUALIZE_ABOVE
  const viewportHeight = placement?.maxHeight ?? MAX_LIST_HEIGHT
  const activeRow = rows.findIndex((row) => row.kind === 'option' && row.index === activeIndex)
  let first = 0
  let last = rows.length
  if (virtualized) {
    const span = Math.ceil(viewportHeight / ROW_HEIGHT) + OVERSCAN * 2
    first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN)
    last = Math.min(rows.length, first + span)
    /*
     * `aria-activedescendant` names an element by id, so the highlighted row
     * has to exist. When the keyboard has jumped outside the scrolled window —
     * End on three thousand rows — the window *moves* to it rather than
     * stretching to cover everything in between, which would put the whole list
     * in the document and undo the point of the exercise.
     */
    if (activeRow >= 0 && (activeRow < first || activeRow >= last)) {
      first = Math.max(0, Math.min(activeRow - OVERSCAN, rows.length - span))
      last = Math.min(rows.length, first + span)
    }
  }
  const windowed = rows.slice(first, last)

  /*
   * Which rows are on screen, published for callers that fetch per row.
   *
   * Joined into a string first: callers rebuild their option array on every
   * render, so an array identity would re-report an unchanged window and, since
   * the caller usually puts the answer into state, spin. A space cannot occur
   * in a value that came from an id.
   */
  const drawnKey = windowed
    .filter((row) => row.kind === 'option')
    .map((row) => (row.kind === 'option' ? row.entry.option.value : ''))
    .join(' ')
  useEffect(() => {
    onDrawnRows?.(drawnKey === '' ? [] : drawnKey.split(' '))
  }, [drawnKey, onDrawnRows])

  /* Keep the highlighted row on screen as the arrows move it. */
  useEffect(() => {
    if (!open || activeRow < 0) return
    const list = listRef.current
    if (!list) return
    const row = list.querySelector<HTMLElement>('[data-active="true"]')
    row?.scrollIntoView?.({ block: 'nearest' })
  }, [open, activeRow])

  const move = (step: number) => {
    if (visibleOptions.length === 0) return
    const from = Math.min(Math.max(activeIndex + step, 0), visibleOptions.length - 1)
    const found = nextEnabled(visibleOptions, from, step > 0 ? 1 : -1)
    // Stepping past the last enabled row in one direction settles on the last
    // enabled row in the other, rather than leaving nothing highlighted.
    setActive(found >= 0 ? found : nextEnabled(visibleOptions, from, step > 0 ? -1 : 1))
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (disabled) return
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault()
        if (!open) {
          openList()
          return
        }
        if (event.altKey) return
        move(1)
        return
      case 'ArrowUp':
        event.preventDefault()
        if (event.altKey && open) {
          close()
          return
        }
        if (!open) {
          openList()
          return
        }
        move(-1)
        return
      case 'Home':
        if (!open) return
        event.preventDefault()
        setActive(Math.max(nextEnabled(visibleOptions, 0, 1), 0))
        return
      case 'End':
        if (!open) return
        event.preventDefault()
        setActive(Math.max(nextEnabled(visibleOptions, visibleOptions.length - 1, -1), 0))
        return
      case 'PageDown':
        if (!open) return
        event.preventDefault()
        move(10)
        return
      case 'PageUp':
        if (!open) return
        event.preventDefault()
        move(-10)
        return
      case 'Enter':
        if (!open) return
        // Only swallowed when there is a list to choose from, so Enter still
        // submits the sheet this control sits in the rest of the time.
        event.preventDefault()
        commit(activeOption)
        return
      case 'Tab':
        // Native behaviour: moving on abandons the search and keeps the value.
        if (open) {
          setOpen(false)
          setQuery('')
        }
        return
      default:
        return
    }
  }

  const count = visibleOptions.length
  const list = open && (
    <div
      className="cbx-pop"
      ref={listRef}
      style={{
        position: 'fixed',
        left: placement?.left,
        width: placement?.width,
        top: placement?.top,
        bottom: placement?.bottom,
        maxHeight: placement?.maxHeight,
      }}
      // Keeps focus on the input while a row is being clicked, so the control
      // never blurs and re-focuses in the middle of a selection.
      onMouseDown={(event) => event.preventDefault()}
      onScroll={(event) => {
        if (virtualized) setScrollTop(event.currentTarget.scrollTop)
      }}
    >
      <div id={listId} role="listbox" aria-label={label}>
        {rows.length === 0 ? (
          <div className="cbx-empty">{emptyMessage}</div>
        ) : (
          <div
            style={
              virtualized ? { height: rows.length * ROW_HEIGHT, position: 'relative' } : undefined
            }
          >
            {windowed.map((row, offset) =>
              row.kind === 'group' ? (
                <div
                  key={row.key}
                  className="cbx-group"
                  role="presentation"
                  id={`${reactId}-group-${row.key}`}
                  style={virtualized ? rowStyle(first + offset) : undefined}
                >
                  {row.label}
                </div>
              ) : (
                <div
                  key={row.key}
                  id={optionId(row.index)}
                  role="option"
                  className={`cbx-option${row.index === activeIndex ? ' is-active' : ''}${
                    row.entry.option.disabled ? ' is-disabled' : ''
                  }`}
                  aria-selected={row.entry.option.value === value}
                  aria-disabled={row.entry.option.disabled || undefined}
                  aria-setsize={count}
                  aria-posinset={row.index + 1}
                  aria-describedby={row.groupKey ? `${reactId}-group-${row.groupKey}` : undefined}
                  data-active={row.index === activeIndex}
                  data-value={row.entry.option.value}
                  style={virtualized ? rowStyle(first + offset) : undefined}
                  onMouseMove={() => setActive(row.index)}
                  onClick={() => commit(row.entry.option)}
                >
                  {row.entry.option.icon && (
                    <span className="cbx-icon" aria-hidden>
                      {row.entry.option.icon}
                    </span>
                  )}
                  <span className="cbx-label">{row.entry.option.label}</span>
                  {row.entry.option.hint && (
                    <span className="cbx-hint">{row.entry.option.hint}</span>
                  )}
                </div>
              ),
            )}
          </div>
        )}
      </div>
    </div>
  )

  const host =
    open && typeof document !== 'undefined'
      ? (field?.closest('[data-modal-host]') ?? document.body)
      : null

  return (
    <div className={className ? `cbx ${className}` : 'cbx'} ref={setField}>
      <input
        ref={inputRef}
        id={id}
        className="cbx-input"
        type="text"
        role="combobox"
        aria-label={label}
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        aria-activedescendant={open && activeIndex >= 0 ? optionId(activeIndex) : undefined}
        aria-autocomplete="list"
        autoComplete="off"
        spellCheck={false}
        disabled={disabled}
        // Closed, the box states the current value the way a `<select>` does.
        // Open, it is the search field, and the value moves to the placeholder
        // so the user can still see what they are about to replace.
        value={open ? query : (selected?.label ?? '')}
        placeholder={open ? (selected?.label ?? placeholder) : placeholder}
        onChange={(event) => {
          if (!open) openList()
          setQuery(event.target.value)
          setActive(0)
        }}
        onKeyDown={onKeyDown}
        onMouseDown={() => {
          if (open) {
            setOpen(false)
            setQuery('')
          } else openList()
        }}
        // A click into the box selects what is in it, so the first character
        // typed replaces the current value instead of appending to it.
        onFocus={(event) => event.target.select()}
        onBlur={() => {
          setOpen(false)
          setQuery('')
        }}
      />
      <span className="cbx-chev" aria-hidden>
        <Icon name="chev" size="sm" />
      </span>
      {/*
       * The count, for anyone who cannot see the list shrink as they type.
       *
       * Polite rather than assertive: it follows the keystroke, it does not
       * interrupt it. `aria-live` alone rather than `role="status"`, because a
       * status role would put a landmark inside every picker in the
       * application — and every surface that already has one status message of
       * its own would then have several. It is present whether the list is open
       * or not, so the live region is registered before it has anything to say.
       */}
      <span className="cbx-status" aria-live="polite" aria-atomic="true">
        {open ? `${count} ${count === 1 ? 'result' : 'results'}` : ''}
      </span>
      {host && list ? createPortal(list, host) : null}
    </div>
  )
}

function rowStyle(index: number) {
  return {
    position: 'absolute' as const,
    top: index * ROW_HEIGHT,
    left: 0,
    right: 0,
    height: ROW_HEIGHT,
  }
}
