import { useEffect, useLayoutEffect, useRef } from 'react'
import type { NodeKind } from '../lib/api'
import { useUndoRunner } from '../lib/queries'
import { chordFromEvent, isTypingTarget } from '../lib/keys'
import { modalOpen } from '../lib/modalStack'
import { FAVOURITES_BAND, RECENT_BAND } from '../components/navigator/navigatorRows'
import {
  resolveCommand,
  useKeybindings,
  type CommandDef,
  type CommandId,
} from '../store/keybindings'
import { useUI, type EditorTab } from '../store/ui'

/**
 * The one place a keystroke becomes a command.
 *
 * Nothing here decides what `⌘K` means — `store/keybindings.ts` does, including
 * whatever the user has since changed it to. This hook only owns the part that
 * cannot be declared: what actually happens, and the two gates that keep a
 * shortcut from firing at a moment when it would act on something the user
 * cannot see (a caret in a text field, a dialog on top of the workspace).
 *
 * Enhance and Generate stay with their surfaces, because only those surfaces
 * know whether the action is eligible right now. They resolve their chord
 * through the same registry, so exactly one command runs for any keystroke.
 */

const TAB_COMMANDS: Partial<Record<CommandId, EditorTab>> = {
  'tab.notes': 'notes',
  'tab.refs': 'refs',
  'tab.concepts': 'concepts',
  'tab.three': 'three',
  'tab.relations': 'relations',
}

interface Context {
  onNewNode: () => void
  readOnly: boolean
  /** The navigator's kind groups, for the collapse-everything command. */
  navKinds: NodeKind[]
}

export function useKeyboard({ onNewNode, readOnly, navKinds }: Context) {
  const { undo, redo } = useUndoRunner()

  // One listener for the life of the workspace, reading the latest of
  // everything. Re-registering on each new binding would be correct too, but a
  // window listener that comes and goes is exactly the kind of thing that ends
  // up registered twice, and a shortcut that fires twice is silent damage.
  //
  // The bindings are not held here at all: they are read from the store at the
  // moment a key is pressed. A keystroke is an imperative event, so the answer
  // it needs is the current one rather than the one from the last render.
  const latest = useRef({ onNewNode, readOnly, navKinds, undo, redo })
  useLayoutEffect(() => {
    latest.current = { onNewNode, readOnly, navKinds, undo, redo }
  })

  useEffect(() => {
    const run = (def: CommandDef): void => {
      const ui = useUI.getState()
      const tab = TAB_COMMANDS[def.id]
      if (tab) {
        // The tabs belong to the Library's editor, so the shortcut takes you
        // there rather than setting a tab you are not looking at.
        ui.setMode('library')
        ui.setTab(tab)
        return
      }

      switch (def.id) {
        case 'palette.toggle':
          if (ui.paletteOpen) ui.setPaletteOpen(false)
          else if (!modalOpen()) ui.setPaletteOpen(true)
          return
        case 'shortcuts.show':
          if (ui.shortcutsOpen) ui.setShortcutsOpen(false)
          else if (!modalOpen()) ui.setShortcutsOpen(true)
          return
        case 'nav.filter':
          focusNavigatorFilter()
          return
        case 'mode.library':
          ui.setMode('library')
          return
        case 'mode.forge':
          ui.setMode(ui.mode === 'forge' ? 'library' : 'forge')
          return
        case 'mode.assets':
          ui.setMode('assets')
          return
        case 'mode.settings':
          ui.setMode('settings')
          return
        case 'panel.navigator':
          ui.toggleNav()
          return
        case 'panel.inspector':
          ui.toggleInsp()
          return
        case 'nav.toggleAll':
          toggleEverything(latest.current.navKinds)
          return
        case 'node.new':
          // The read-only refusal lives at the single gate every route to the
          // sheet passes through, not here; this is one of those routes.
          latest.current.onNewNode()
          return
        case 'edit.undo':
        case 'edit.redo':
          // Undo and redo replay writes, so on a read-only folder the shortcut
          // is swallowed rather than left to fail at save time. There is
          // nothing on the stack to reverse either.
          if (latest.current.readOnly) return
          void (def.id === 'edit.undo' ? latest.current.undo() : latest.current.redo())
          return
        default:
          return
      }
    }

    const handler = (event: KeyboardEvent) => {
      const chord = chordFromEvent(event)
      if (!chord) return

      // Escape is not in the registry and is not rebindable: it means "dismiss"
      // everywhere, and `Modal` already handles it in the capture phase for
      // every dialog, this one included. What is left here is the case where a
      // keydown is dispatched straight at `window` and the document listener is
      // therefore not in the propagation path at all.
      if (chord === 'Escape') {
        const ui = useUI.getState()
        if (ui.shortcutsOpen) {
          event.preventDefault()
          ui.setShortcutsOpen(false)
        } else if (ui.paletteOpen) {
          event.preventDefault()
          ui.setPaletteOpen(false)
        }
        return
      }

      const def = resolveCommand(chord, useKeybindings.getState().overrides)
      // A surface-owned command belongs to the component that can say whether
      // it is eligible. Resolving it here and stopping is what stops the same
      // keystroke being handled twice.
      if (!def || def.surface) return
      if (!def.whileTyping && isTypingTarget(event.target)) return
      if (!def.whileModal && modalOpen()) return

      event.preventDefault()
      run(def)
    }

    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])
}

/**
 * Collapse everything, or open it again if it is already shut.
 *
 * The same toggle the navigator's own button performs. Only the two named
 * sections are closed explicitly — the letter bands default to closed, so
 * `expandAll` clearing the whole record is what reopens the sections and
 * returns the index to its default in one step.
 */
function toggleEverything(kinds: NodeKind[]): void {
  const ui = useUI.getState()
  const allClosed = kinds.length > 0 && kinds.every((kind) => ui.closedGroups[kind])
  if (allClosed || kinds.length === 0) ui.expandAll()
  else ui.collapseAll(kinds, [FAVOURITES_BAND, RECENT_BAND])
}

/**
 * Put the caret in the navigator's filter box, revealing it first if need be.
 *
 * Reached through the DOM rather than a ref because the navigator is mounted by
 * a different branch of the tree and there is nothing to hand a ref through.
 * The frame is not optional: the pane may have been collapsed or in another
 * mode a moment ago, and the input does not exist until React has drawn it.
 */
function focusNavigatorFilter(): void {
  const ui = useUI.getState()
  if (ui.mode !== 'library') ui.setMode('library')
  if (ui.navCollapsed) ui.toggleNav()
  const focus = () => {
    const input = document.querySelector<HTMLInputElement>('.nav-search input')
    input?.focus()
    input?.select()
  }
  focus()
  window.requestAnimationFrame(focus)
}

export type ActionShortcut = Extract<CommandId, 'enhance' | 'generate'>

/**
 * Register an editing-surface action without stealing keys from text controls.
 *
 * The chord comes from the registry, so rebinding Enhance in Settings moves
 * this listener with it, and a chord the user has given to something else
 * resolves to that other command here as well — the surface never wins a
 * conflict just by having registered later.
 */
export function useActionShortcut(shortcut: ActionShortcut, enabled: boolean, action: () => void) {
  const latest = useRef({ enabled, action })
  useLayoutEffect(() => {
    latest.current = { enabled, action }
  })

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const chord = chordFromEvent(event)
      if (!chord) return
      if (resolveCommand(chord, useKeybindings.getState().overrides)?.id !== shortcut) return
      if (isTypingTarget(event.target) || modalOpen()) return

      // These are application commands whenever focus is outside an editable
      // control. Swallow the browser's own Cmd/Ctrl+E even when the visible
      // action is disabled, just as a disabled toolbar button swallows clicks.
      event.preventDefault()
      if (!latest.current.enabled || event.repeat) return
      latest.current.action()
    }

    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [shortcut])
}
