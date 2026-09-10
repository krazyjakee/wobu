import { useProjectMutation } from './useProjectMutation'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  narrativeWorldGet,
  narrativeWorldSave,
  type WorldDocument,
  type WorldFile,
} from '../api/narrativeWorld'
import { preconditionOf } from '../api'
import { useUndoStack } from '../undo'
import { report } from '../../store/ui'
import { invalidateNarrative } from './keys'

export function useNarrativeWorld() {
  return useQuery({ queryKey: ['narrative_world'], queryFn: narrativeWorldGet, retry: false })
}

export function useSaveNarrativeWorld() {
  const qc = useQueryClient()
  return useProjectMutation({
    mutationFn: ({ file, document }: { file: WorldFile; document: WorldDocument }) =>
      narrativeWorldSave(document, preconditionOf(file.stamp)),
    onSuccess: (file, before) => {
      qc.setQueryData(['narrative_world'], file)
      if (JSON.stringify(before.file.document) !== JSON.stringify(file.document)) {
        useUndoStack.getState().push({
          subjectId: 'narrative-world',
          label: 'edit narrative world',
          coalesce: false,
          undo: [{ type: 'worldRestore', document: before.file.document, expected: file.document }],
          redo: [
            {
              type: 'worldRestore',
              document: file.document,
              // Undo is itself an explicit save and retains the current document
              // version. Keep every authored field in the original CAS snapshot.
              expected: { ...before.file.document, schema_version: file.document.schema_version },
            },
          ],
        })
      }
      invalidateNarrative(qc)
    },
    onError: (error) => report(error, 'Could not save narrative world'),
  })
}
