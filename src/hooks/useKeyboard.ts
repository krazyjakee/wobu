import { useEffect } from 'react'
import { EDITOR_TABS, useUI } from '../store/ui'

/**
 * The keyboard map from docs/03-ui-layout.md, restricted to what M1 actually
 * does. ⌘E (Enhance) and ⌘↵ (Generate) are deliberately unbound — binding a
 * key to a feature that does not exist is a lie the user pays for later.
 */
export function useKeyboard({ onNewNode }: { onNewNode: () => void }) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const ui = useUI.getState()
      const mod = e.metaKey || e.ctrlKey
      const typing = isTyping(e.target)

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
  }, [onNewNode])
}

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null
  if (!el) return false
  const tag = el.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable
}
