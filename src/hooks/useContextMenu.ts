import { useCallback, useState } from 'react'
import type { KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent } from 'react'
import { isTypingTarget } from '../lib/keys'

/**
 * Opening a menu, from either hand (#130).
 *
 * A context menu that only answers the right mouse button is an accelerator
 * that a keyboard user is simply not offered, and on the surfaces this hook is
 * used for — a reference tile, a concept card, an influence layer — the actions
 * behind it are sometimes three clicks away otherwise. So every surface takes
 * both routes from here rather than each writing its own: right-click, and the
 * two keystrokes the platforms agreed on.
 */

/** How far in from the row's left edge a keyboard-opened menu is drawn. */
const KEY_OFFSET_X = 12

/**
 * Shift+F10 and the Menu key.
 *
 * Both, not either: the Menu key is the one Windows keyboards have and Mac
 * keyboards do not, and Shift+F10 is the fallback every platform honours. A
 * user who knows one of them should not have to discover the other.
 */
export function isMenuKey(event: { key: string; shiftKey: boolean }): boolean {
  return event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)
}

/**
 * Whether the platform's own menu is the better answer here.
 *
 * Inside somewhere text is being composed it always is: cut, copy, paste,
 * spelling and the input method belong to the field, and replacing them with
 * "Mute reference" would take away the menu the user actually wanted. Sliders,
 * checkboxes and radios are excluded from that rule — they carry no text and no
 * clipboard, so their native menu is empty and the row's menu is what is meant.
 */
function keepsNativeMenu(target: EventTarget | null): boolean {
  if (!isTypingTarget(target)) return false
  const type = (target as HTMLInputElement).type
  return type !== 'range' && type !== 'checkbox' && type !== 'radio'
}

/**
 * The pointer and keyboard openers for one row, given somewhere to report to.
 *
 * `open` is handed viewport coordinates and the element the menu should return
 * focus to when it closes. From the pointer that is the row itself, focused on
 * the way so that Escape lands somewhere sensible; from the keyboard it is
 * whatever was already focused inside the row, because a keystroke should never
 * move focus off the control the user was on.
 */
export function menuTriggerProps(open: (x: number, y: number, opener: HTMLElement) => void) {
  return {
    onContextMenu: (event: ReactMouseEvent<HTMLElement>) => {
      if (keepsNativeMenu(event.target)) return
      event.preventDefault()
      event.stopPropagation()
      const opener = event.currentTarget
      opener.focus()
      open(event.clientX, event.clientY, opener)
    },
    onKeyDown: (event: ReactKeyboardEvent<HTMLElement>) => {
      if (!isMenuKey(event) || keepsNativeMenu(event.target)) return
      event.preventDefault()
      event.stopPropagation()
      const active = document.activeElement
      const opener =
        active instanceof HTMLElement && event.currentTarget.contains(active)
          ? active
          : event.currentTarget
      const box = opener.getBoundingClientRect()
      open(box.left + KEY_OFFSET_X, box.bottom, opener)
    },
  }
}

/** What a row spreads onto itself to become right-clickable. */
export type MenuTriggerProps = ReturnType<typeof menuTriggerProps>

export interface MenuAnchor<T> {
  x: number
  y: number
  /** What the menu is about — the row's own subject, not an index into a list. */
  item: T
  opener: HTMLElement
}

/**
 * The open menu, if there is one, and the props that open it.
 *
 * One menu per surface: a second right-click moves the existing menu rather
 * than stacking another on top, which is what a `useState` per row would do.
 */
export function useContextMenu<T>() {
  const [anchor, setAnchor] = useState<MenuAnchor<T> | null>(null)
  const close = useCallback(() => setAnchor(null), [])
  const trigger = useCallback(
    (item: T) =>
      menuTriggerProps((x, y, opener) => {
        setAnchor({ x, y, item, opener })
      }),
    [],
  )
  return { anchor, close, trigger }
}
