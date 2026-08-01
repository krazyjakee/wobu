import { useEffect } from 'react'
import { useUndoRunner } from '../lib/queries'
import { undoIntent } from '../lib/undo'
import { EDITOR_TABS, useUI } from '../store/ui'

/**
 * The keyboard map from docs/03-ui-layout.md, restricted to what M1 actually
 * does. ⌘E (Enhance) and ⌘↵ (Generate) are deliberately unbound — binding a
 * key to a feature that does not exist is a lie the user pays for later.
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

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null
  if (!el) return false
  const tag = el.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable
}
