import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { Icon } from '../Icon'
import { Tooltip } from '../Tooltip'
import { NARRATIVE_STATUS, type NarrativeItem, type NarrativeListState } from './narrativeModel'

/**
 * One section of the Narrative Library, in whichever of its five conditions it
 * is actually in.
 *
 * Every list in this workspace goes through here rather than each section
 * writing its own empty paragraph, because "no scenes yet", "still reading",
 * "the read failed", "this build cannot answer" and "you may look but not
 * write" are five different things a writer needs told apart, and a section
 * that only implements the happy one silently becomes a blank rectangle in the
 * other four.
 *
 * A ready-but-empty list is the only one allowed to offer the affordances that
 * would fill it: offering "Create first scene" under a *failed* read would ask
 * somebody to write into a project Wobu has just admitted it cannot read.
 */
export function NarrativeList({
  label,
  state,
  selectedId = null,
  onActivate,
  empty,
  emptyActions,
  readOnly = false,
  reveal = null,
}: {
  /** The accessible name of the list. Never carried by the heading alone. */
  label: string
  state: NarrativeListState
  /** The id to show as chosen, taken from the one shared selection. */
  selectedId?: string | null
  /** Left out while a section's rows cannot be selected yet. */
  onActivate?: (id: string) => void
  empty: string
  emptyActions?: ReactNode
  readOnly?: boolean
  /** A row to bring into view, and the request number that asked for it. */
  reveal?: { id: string; seq: number } | null
}) {
  if (state.kind === 'unavailable') {
    return (
      <p className="nrt-note">
        <Icon name="lock" size="sm" />
        {state.reason}
      </p>
    )
  }

  if (state.kind === 'loading') {
    // `aria-busy` on the region rather than a spinner: the wait is short and a
    // screen reader is better served by the word than by an animation.
    return (
      <p className="nrt-note" aria-busy="true">
        <Icon name="clock" size="sm" />
        Reading {label.toLocaleLowerCase()}…
      </p>
    )
  }

  if (state.kind === 'error') {
    return (
      <p className="nrt-note inline-error" role="alert">
        Could not read {label.toLocaleLowerCase()}: {state.message}
      </p>
    )
  }

  if (state.items.length === 0) {
    return (
      <div className="nrt-empty">
        <p>{empty}</p>
        {readOnly && (
          <p className="nrt-note">
            <Icon name="lock" size="sm" />
            This project folder is read-only, so nothing can be added to it here.
          </p>
        )}
        {emptyActions && <div className="nrt-empty-actions">{emptyActions}</div>}
      </div>
    )
  }

  return (
    <NarrativeRows
      label={label}
      items={state.items}
      selectedId={selectedId}
      onActivate={onActivate}
      reveal={reveal}
    />
  )
}

/**
 * The rows, with the keyboard behaviour a list of them owes.
 *
 * Split out because the hooks below cannot live under the five-way branch
 * above, and because this is the only part with state: which row holds the tab
 * stop. One tab stop for the whole list, moved by the arrow keys — Tab past a
 * scene list should reach the next control, not the ninth scene.
 */
function NarrativeRows({
  label,
  items,
  selectedId,
  onActivate,
  reveal,
}: {
  label: string
  items: NarrativeItem[]
  selectedId: string | null
  onActivate?: (id: string) => void
  reveal: { id: string; seq: number } | null
}) {
  const rows = useRef<(HTMLDivElement | null)[]>([])
  const [focused, setFocused] = useState<number | null>(null)

  /*
   * Bringing a row chosen somewhere else into view.
   *
   * The worked example of the reveal channel, and the shape every other
   * narrative surface should copy. Two things make it behave. The request is
   * honoured once per `seq`, so a re-render does not scroll again — and
   * because the request is latched in the store rather than fired as an event,
   * a list that was unmounted when it was raised still owes the scroll when it
   * comes back. Scrolling on the *selection* instead would drag the reader
   * back every time they scrolled away from the row they were already on.
   */
  const honoured = useRef(0)
  useEffect(() => {
    if (!reveal || reveal.seq === honoured.current) return
    honoured.current = reveal.seq
    const index = items.findIndex((item) => item.id === reveal.id)
    // `scrollIntoView` is not implemented everywhere this renders — jsdom, for
    // one — and a missing scroll must not take the workspace down with it.
    if (index >= 0) rows.current[index]?.scrollIntoView?.({ block: 'nearest' })
  }, [reveal, items])

  // The tab stop follows the shared selection until the reader moves it
  // themselves. Selecting a beat on the Flow canvas therefore also moves where
  // the keyboard re-enters this list, which is the behaviour that makes one
  // selection feel like one selection rather than two that agree.
  const selectedIndex = items.findIndex((item) => item.id === selectedId)
  const preferred = focused ?? (selectedIndex < 0 ? 0 : selectedIndex)
  const active = Math.min(Math.max(preferred, 0), items.length - 1)

  const move = (to: number) => {
    const index = Math.min(Math.max(to, 0), items.length - 1)
    setFocused(index)
    rows.current[index]?.focus()
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault()
        move(active + 1)
        return
      case 'ArrowUp':
        event.preventDefault()
        move(active - 1)
        return
      case 'Home':
        event.preventDefault()
        move(0)
        return
      case 'End':
        event.preventDefault()
        move(items.length - 1)
        return
      case 'Enter':
      case ' ':
        // A `role="option"` is not a button, so it gets no activation for free.
        event.preventDefault()
        onActivate?.(items[active]!.id)
        return
      default:
    }
  }

  return (
    <div
      className="nrt-rows"
      role={onActivate ? 'listbox' : 'list'}
      aria-label={label}
      onKeyDown={onActivate ? onKeyDown : undefined}
    >
      {items.map((item, index) => {
        const status = item.status ? NARRATIVE_STATUS[item.status] : null
        return (
          <div
            key={item.id}
            ref={(node) => {
              rows.current[index] = node
            }}
            className={item.id === selectedId ? 'nrt-row is-selected' : 'nrt-row'}
            role={onActivate ? 'option' : 'listitem'}
            aria-selected={onActivate ? item.id === selectedId : undefined}
            tabIndex={onActivate ? (index === active ? 0 : -1) : undefined}
            onFocus={() => setFocused(index)}
            onClick={onActivate ? () => onActivate(item.id) : undefined}
          >
            <span className="nrt-row-name">{item.name}</span>
            {status && (
              <span className="nrt-badge">
                <Icon name={status.icon} size="sm" />
                {status.label}
              </span>
            )}
            {item.conflict && (
              /* Focusable so the reason reaches a keyboard, exactly as the
                 editor's `stale` badge is. The badge says the word as well as
                 wearing the colour. */
              <Tooltip tip={item.conflict}>
                <span className="nrt-badge is-conflict" tabIndex={0}>
                  <Icon name="layers" size="sm" />
                  Conflict
                </span>
              </Tooltip>
            )}
          </div>
        )
      })}
    </div>
  )
}
