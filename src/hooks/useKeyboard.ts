import { useEffect, useLayoutEffect, useRef } from 'react'
import { useUndoRunner } from '../lib/queries'
import { undoIntent } from '../lib/undo'
import { EDITOR_TABS, useUI } from '../store/ui'

/**
 * The global part of the keyboard map from docs/03-ui-layout.md. Enhance and
 * Generate remain owned by their editing surfaces because only those surfaces
 * know whether their actions are currently eligible; mode navigation lives
 * here so it behaves identically wherever focus was.
 */
export function useKeyboard({ onNewNode, readOnly }: { onNewNode: () => void; readOnly: boolean }) {
  const { undo, redo } = useUndoRunner()

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const ui = useUI.getState()
      const mod = e.metaKey || e.ctrlKey
      const typing = isTyping(e.target)

      // Before the palette check: ⌘Z inside the palette's own input is the
      // input's, and `undoIntent` already refuses it because `typing` is true.
      const intent = undoIntent(e, typing)
      if (intent) {
        e.preventDefault()
        // Undo and redo replay writes, so on a read-only folder the shortcut is
        // swallowed rather than left to fail at save time. There is nothing on
        // the stack to reverse either — nothing here has written anything.
        if (!readOnly) void (intent === 'undo' ? undo() : redo())
        return
      }

      if (mod && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        ui.setPaletteOpen(!ui.paletteOpen)
        return
      }

      if (e.key === 'Escape' && ui.paletteOpen) {
        e.preventDefault()
        ui.setPaletteOpen(false)
        return
      }

      if (mod && e.key.toLowerCase() === 'n') {
        e.preventDefault()
        onNewNode()
        return
      }

      if (mod && e.key === '\\') {
        e.preventDefault()
        ui.setMode(ui.mode === 'forge' ? 'library' : 'forge')
        return
      }

      if (mod && !e.shiftKey && !e.altKey && /^[1-5]$/.test(e.key)) {
        e.preventDefault()
        const tab = EDITOR_TABS[Number(e.key) - 1]
        if (tab) {
          ui.setMode('library')
          ui.setTab(tab)
        }
        return
      }

      if (!mod && !typing && !ui.paletteOpen) {
        if (e.key === '[') {
          e.preventDefault()
          ui.toggleNav()
        } else if (e.key === ']') {
          e.preventDefault()
          ui.toggleInsp()
        }
      }
    }

    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onNewNode, readOnly, undo, redo])
}

export type ActionShortcut = 'enhance' | 'generate'

/** Register an editing-surface action without stealing keys from text controls. */
export function useActionShortcut(shortcut: ActionShortcut, enabled: boolean, action: () => void) {
  const enabledRef = useRef(enabled)
  const actionRef = useRef(action)
  useLayoutEffect(() => {
    enabledRef.current = enabled
    actionRef.current = action
  }, [action, enabled])

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (!matchesActionShortcut(event, shortcut) || isTyping(event.target)) return

      // These are application commands whenever focus is outside an editable
      // control. Swallow the browser's own Cmd/Ctrl+E even when the visible
      // action is disabled, just as a disabled toolbar button swallows clicks.
      event.preventDefault()
      if (!enabledRef.current || event.repeat) return
      actionRef.current()
    }

    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [shortcut])
}

function matchesActionShortcut(event: KeyboardEvent, shortcut: ActionShortcut): boolean {
  if ((!event.metaKey && !event.ctrlKey) || event.shiftKey || event.altKey) return false
  return shortcut === 'enhance' ? event.key.toLowerCase() === 'e' : event.key === 'Enter'
}

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null
  if (!el) return false
  const tag = el.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable
}
