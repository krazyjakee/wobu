import { useCallback } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { applyCommand, useUndoStack } from '../undo'
import { report, toast } from '../../store/ui'
import { invalidateWorld } from './keys'
/* ── keys ─────────────────────────────────────────────────────────────────── */

/**
 * Drive the undo stack from the UI.
 *
 * The commands run through `applyCommand`, which calls the backend directly
 * rather than going back through the mutation hooks above — those record what
 * they do, and an undo that recorded itself would put its own inverse on the
 * stack and make ⌘Z a toggle.
 *
 * Invalidation happens in `finally` on purpose. A sequence that failed halfway
 * has still changed the world, and the conflict path pulls the winner's version
 * into the index before it reports, so the cache is stale either way.
 */
export function useUndoRunner() {
  const qc = useQueryClient()

  const undo = useCallback(async () => {
    try {
      const entry = await useUndoStack.getState().undo(applyCommand)
      if (!entry) return
      toast(entry.caveat ? `Undone: ${entry.label}. ${entry.caveat}` : `Undone: ${entry.label}`)
    } catch (e) {
      report(e, 'Undo failed')
    } finally {
      invalidateWorld(qc)
    }
  }, [qc])

  const redo = useCallback(async () => {
    try {
      const entry = await useUndoStack.getState().redo(applyCommand)
      if (entry) toast(`Redone: ${entry.label}`)
    } catch (e) {
      report(e, 'Redo failed')
    } finally {
      invalidateWorld(qc)
    }
  }, [qc])

  return { undo, redo }
}

/**
 * The same two actions, plus what they would do.
 *
 * Separate from `useUndoRunner` because reading the stack subscribes to it, and
 * the keyboard hook lives in the Workspace — re-rendering the entire workspace
 * every time an entry is pushed, to run a callback that does not depend on it,
 * is a cost with nothing to show for it. Only a surface that *names* the next
 * entry needs this one.
 */
export function useUndo() {
  const { undo, redo } = useUndoRunner()
  const past = useUndoStack((s) => s.past)
  const future = useUndoStack((s) => s.future)

  return {
    undo,
    redo,
    /** The entry ⌘Z would reverse, for naming it on the surface that offers it. */
    nextUndo: past[past.length - 1] ?? null,
    nextRedo: future[future.length - 1] ?? null,
  }
}

/* ── file-watcher bridge ──────────────────────────────────────────────────── */
