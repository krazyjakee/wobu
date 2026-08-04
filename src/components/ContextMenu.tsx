import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode, RefObject } from 'react'
import { ariaChord, chordParts } from '../lib/keys'
import { bindingOf, useKeybindings, type CommandId } from '../store/keybindings'
import { isMenuKey } from '../hooks/useContextMenu'
import { TipButton } from './Tooltip'

/**
 * The one menu in Wobu (#130).
 *
 * It is used two ways and knows the difference. Given `x`/`y` it is a *context*
 * menu: fixed to the pointer, and portalled to `<body>` so that the tile it
 * belongs to cannot capture it — a card positioned with `transform`, which
 * every virtualized grid in this app uses, becomes the containing block for its
 * `position: fixed` descendants, and a menu rendered inside one would be drawn
 * at the pointer's coordinates *plus* the card's offset. Given no coordinates
 * it stays where it was rendered and is placed by its own class, which is what
 * the title bar's project menu and the launcher's card menu want.
 *
 * Because a portal still propagates React events to the tree it was written in,
 * the menu stops click and contextmenu at its own root: without that, choosing
 * an item would also fire the click handler of the tile the menu was declared
 * inside, and right-clicking the menu would open a second one on top of it.
 */
export function ContextMenu({
  x,
  y,
  onClose,
  children,
  className,
  restoreFocus,
  restoreFocusRef,
  label,
}: {
  x?: number
  y?: number
  onClose: () => void
  children: ReactNode
  className?: string
  restoreFocus?: HTMLElement | null
  restoreFocusRef?: RefObject<HTMLElement | null>
  label?: string
}) {
  const ref = useRef<HTMLDivElement>(null)
  const opener = useRef<HTMLElement | null>(
    restoreFocus ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null),
  )
  const [pos, setPos] = useState(() =>
    x === undefined || y === undefined ? undefined : { left: x, top: y },
  )

  useLayoutEffect(() => {
    const el = ref.current
    if (!el || x === undefined || y === undefined) return
    const r = el.getBoundingClientRect()
    setPos({
      left: Math.max(6, Math.min(x, window.innerWidth - r.width - 6)),
      top: Math.max(6, Math.min(y, window.innerHeight - r.height - 6)),
    })
  }, [x, y])

  useLayoutEffect(() => {
    const menu = ref.current
    const items = menuItems(menu)
    for (const item of items) item.tabIndex = -1
    const first = items[0]
    const restoreTarget = restoreFocusRef?.current ?? opener.current
    // `preventScroll` because the menu is already where the user is looking:
    // scrolling the grid underneath it to reveal a row that is not moving would
    // shift the content the menu is about, and the scroll below closes it.
    ;(first ?? menu)?.focus({ preventScroll: true })

    return () => {
      if (restoreTarget?.isConnected) restoreTarget.focus()
    }
  }, [restoreFocusRef])

  useEffect(() => {
    const down = (e: MouseEvent) => {
      const target = e.target as Node
      const trigger = restoreFocusRef?.current ?? opener.current
      if (!ref.current?.contains(target) && !trigger?.contains(target)) onClose()
    }
    window.addEventListener('mousedown', down)
    window.addEventListener('resize', onClose)
    // Capture, because a scrolling grid does not bubble its scroll to the
    // window. A menu is about the row it was opened on; once that row has moved
    // out from under it the menu is pointing at something else, which is why
    // every platform's own menus close here too.
    window.addEventListener('scroll', onClose, true)
    return () => {
      window.removeEventListener('mousedown', down)
      window.removeEventListener('resize', onClose)
      window.removeEventListener('scroll', onClose, true)
    }
  }, [onClose, restoreFocusRef])

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const items = menuItems(ref.current)
    const active = document.activeElement
    const index = items.findIndex((item) => item === active)
    let next: HTMLElement | undefined

    switch (event.key) {
      case 'ArrowDown':
        next = items[(index + 1 + items.length) % items.length]
        break
      case 'ArrowUp':
        next = items[(index - 1 + items.length) % items.length]
        break
      case 'Home':
        next = items[0]
        break
      case 'End':
        next = items.at(-1)
        break
      case 'Escape':
        event.preventDefault()
        event.stopPropagation()
        onClose()
        return
      default:
        // The key that opens a menu must not open another one from inside it.
        if (isMenuKey(event)) {
          event.preventDefault()
          event.stopPropagation()
        }
        return
    }

    if (next) {
      event.preventDefault()
      event.stopPropagation()
      next.focus()
    }
  }

  const menu = (
    <div
      ref={ref}
      className={className ? `ctx ${className}` : 'ctx'}
      style={pos}
      role="menu"
      aria-label={label}
      tabIndex={-1}
      onKeyDown={onKeyDown}
      onClick={(event) => event.stopPropagation()}
      onContextMenu={(event) => {
        event.preventDefault()
        event.stopPropagation()
      }}
    >
      <CloseMenu.Provider value={onClose}>{children}</CloseMenu.Provider>
    </div>
  )

  return x === undefined || y === undefined ? menu : createPortal(menu, document.body)
}

/**
 * How a row closes the menu it is in.
 *
 * Through context rather than a prop on every item, because the alternative —
 * which the navigator carried on its own — is a `pick(fn)` helper repeated in
 * each menu, and one of those eventually forgets to close.
 */
const CloseMenu = createContext<() => void>(() => {})

/**
 * A row in a menu.
 *
 * Built on `<TipButton>`, so a row that is unavailable is `aria-disabled` and
 * says *why* rather than being a grey rectangle: it keeps its focus stop, is
 * reachable with the arrow keys, and answers when a keyboard user lands on it
 * (#129). That is also why `menuItems()` below skips `[disabled]` and not
 * `[aria-disabled]`.
 *
 * `command` is the only way a row may print a chord. It reads the binding in
 * force *now*, so a rebound key is right here without this file knowing what it
 * was rebound to, and a command the user unbound prints nothing at all.
 */
export function MenuItem({
  icon,
  danger,
  command,
  disabledReason,
  onSelect,
  children,
}: {
  icon?: ReactNode
  danger?: boolean
  /** The registry command this row is an accelerator for, if it is one. */
  command?: CommandId
  /** Set to refuse the row, and to say what would un-refuse it. */
  disabledReason?: string | null
  onSelect: () => void
  children: ReactNode
}) {
  const close = useContext(CloseMenu)
  const overrides = useKeybindings((state) => state.overrides)
  const chord = command ? bindingOf(overrides, command) : null
  return (
    <TipButton
      role="menuitem"
      className={danger ? 'danger' : undefined}
      placement="right"
      aria-keyshortcuts={ariaChord(chord)}
      disabledReason={disabledReason ?? null}
      onClick={() => {
        close()
        onSelect()
      }}
    >
      {icon}
      {children}
      {chord && (
        <span className="keys-chord ctx-chord">
          {chordParts(chord).map((part, index) => (
            <kbd key={index}>{part}</kbd>
          ))}
        </span>
      )}
    </TipButton>
  )
}

/** What the menu is about, said once at the top of it. */
export function MenuLabel({ children }: { children: ReactNode }) {
  return (
    <div className="ctx-label" role="presentation">
      {children}
    </div>
  )
}

export function MenuSeparator() {
  return <div className="ctx-sep" role="separator" />
}

/**
 * The rows the arrow keys move between.
 *
 * `[disabled]` is skipped because the platform has already taken it out of the
 * tab order and it can be neither hovered nor read. `aria-disabled` is *not*
 * skipped, which is deliberate and is what APG asks for: a refused menu item
 * that keeps its focus stop is one a keyboard user can land on and be told why
 * it is refused (#129). Skipping it would hide the explanation from exactly the
 * people who cannot hover to find it.
 */
function menuItems(menu: HTMLElement | null): HTMLElement[] {
  if (!menu) return []
  return Array.from(menu.querySelectorAll<HTMLElement>('[role="menuitem"]:not([disabled])'))
}
