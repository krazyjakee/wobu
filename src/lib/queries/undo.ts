import { isProjectSession, projectSessionEpoch } from '../projectSession'
import { useCallback } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { applyCommand, useUndoStack } from '../undo'
import { sceneEditKey, useScriptDrafts } from '../../components/narrative/scriptDrafts'
import { useUI } from '../../store/ui'
import type { ProjectSummary } from '../api'
import { qk } from './keys'
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
  const draftUndo = useCallback(
    (redo: boolean) => {
      const project = qc.getQueryData<ProjectSummary | null>(qk.projectCurrent)?.path
      if (!project) return false
      const store = useScriptDrafts.getState()
      const sceneId = useUI.getState().narrative.sceneId
      const key = sceneId && sceneEditKey(project, sceneId)
      if (key && store.drafts[key]) {
        if (redo) store.redo(key)
        else store.undo(key)
        return true
      }
      if (Object.keys(store.drafts).some((key) => key.startsWith(`${project}:`))) {
        toast('Return to the scene draft to undo, or save or discard it first.')
        return true
      }
      return false
    },
    [qc],
  )

  const undo = useCallback(async () => {
    const epoch = projectSessionEpoch()
    if (draftUndo(false)) return
    try {
      const entry = await useUndoStack.getState().undo(applyCommand)
      if (!entry || !isProjectSession(epoch)) return
      toast(entry.caveat ? `Undone: ${entry.label}. ${entry.caveat}` : `Undone: ${entry.label}`)
    } catch (e) {
      if (isProjectSession(epoch)) report(e, 'Undo failed')
    } finally {
      if (isProjectSession(epoch)) invalidateWorld(qc)
    }
  }, [qc, draftUndo])

  const redo = useCallback(async () => {
    const epoch = projectSessionEpoch()
    if (draftUndo(true)) return
    try {
      const entry = await useUndoStack.getState().redo(applyCommand)
      if (entry && isProjectSession(epoch)) toast(`Redone: ${entry.label}`)
    } catch (e) {
      if (isProjectSession(epoch)) report(e, 'Redo failed')
    } finally {
      if (isProjectSession(epoch)) invalidateWorld(qc)
    }
  }, [qc, draftUndo])

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
