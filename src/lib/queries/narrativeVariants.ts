import { useMutation, useQueryClient } from '@tanstack/react-query'
import { narrativeVariantsMaterialize } from '../api/narrativeVariants'
import { assertProjectSession, projectSessionEpoch } from '../projectSession'
import { sceneEditEntry, useUndoStack } from '../undo'
import { invalidateNarrative, qk } from './keys'
/** Structural matrix edits share the workspace's scene undo and authoritative source cache. */
export function useMaterializeVariants() {
  const client = useQueryClient()
  return useMutation({
    mutationFn: ({ report, slot, rows }: { report: string; slot: string; rows: string[] }) =>
      narrativeVariantsMaterialize(report, slot, rows),
    onMutate: () => ({
      epoch: projectSessionEpoch(),
      undoProject: useUndoStack.getState().projectId,
    }),
    onSuccess: ({ before, after }, _, capture) => {
      assertProjectSession(capture.epoch)
      if (capture.undoProject !== useUndoStack.getState().projectId)
        throw new Error('The project changed before the matrix edit completed.')
      const entry = sceneEditEntry(before, after)
      if (entry) useUndoStack.getState().push({ ...entry, coalesce: false })
      client.setQueryData(qk.narrativeScene(after.scene.id), after)
      invalidateNarrative(client)
    },
  })
}
