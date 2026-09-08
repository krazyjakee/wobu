import { useQuery } from '@tanstack/react-query'
import type { SceneFile } from '../../lib/api'
import { narrativeReviewGet } from '../../lib/api/narrativeReview'
import { useNarrativeReviewApply } from '../../lib/queries/narrativeReview'

export function useScriptReview(file: SceneFile, projectKey: string) {
  const query = useQuery({
    queryKey: ['narrative_review', projectKey, file.scene.id, file.stamp?.hash],
    queryFn: () => narrativeReviewGet(file.scene.id),
    retry: false,
  })
  const mutation = useNarrativeReviewApply(projectKey)
  return { query, mutation }
}
