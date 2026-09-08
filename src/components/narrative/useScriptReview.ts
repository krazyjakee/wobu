import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { ProjectSummary, SceneFile } from '../../lib/api'
import {
  narrativeReviewApply,
  narrativeReviewGet,
  type ReviewAction,
  type ReviewLine,
  type ReviewSceneView,
} from '../../lib/api/narrativeReview'
import { invalidateNarrative, qk } from '../../lib/queries/keys'

export function useScriptReview(file: SceneFile, projectKey: string) {
  const qc = useQueryClient()
  const query = useQuery({
    queryKey: ['narrative_review', projectKey, file.scene.id, file.stamp?.hash],
    queryFn: () => narrativeReviewGet(file.scene.id),
    retry: false,
  })
  const mutation = useMutation({
    mutationFn: ({
      view,
      line,
      action,
    }: {
      view: ReviewSceneView
      line: ReviewLine
      action: ReviewAction
    }) =>
      narrativeReviewApply({
        guard: view.guard,
        target: line.target,
        context_revision: line.context_revision,
        state_json: view.state_json,
        action,
      }),
    onSuccess: ({ file }) => {
      if (qc.getQueryData<ProjectSummary | null>(qk.projectCurrent)?.path === projectKey) {
        qc.setQueryData(qk.narrativeScene(file.scene.id), file)
        invalidateNarrative(qc)
      }
      void qc.invalidateQueries({ queryKey: ['narrative_review', projectKey] })
    },
  })
  return { query, mutation }
}
