import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { narrativeReviewApply, narrativeReviewList } from '../api/narrativeReview'
import { narrativeGenerationHistory } from '../api/narrativeGeneration'
import { invalidateNarrative, qk } from './keys'
import type { ProjectSummary } from '../api'

export function useNarrativeReviewQueue(projectKey: string, stateJson: string | null) {
  return useInfiniteQuery({
    queryKey: ['narrative_review', projectKey, 'queue', stateJson],
    initialPageParam: { offset: 0, catalog: null as string | null },
    queryFn: ({ pageParam }) => narrativeReviewList(stateJson, pageParam.offset, pageParam.catalog),
    getNextPageParam: (page) =>
      page.next_offset === null
        ? undefined
        : { offset: page.next_offset, catalog: page.catalog_revision },
    retry: false,
  })
}
export function useNarrativeReviewProvenance(projectKey: string) {
  return useQuery({
    queryKey: ['narrative_review', projectKey, 'provenance'],
    queryFn: narrativeGenerationHistory,
    retry: false,
  })
}
export function useNarrativeReviewApply(projectKey: string) {
  const client = useQueryClient()
  return useMutation({
    mutationFn: narrativeReviewApply,
    onSuccess: ({ file }) => {
      // A reply may arrive after this modal closes and another project opens.
      if (client.getQueryData<ProjectSummary | null>(qk.projectCurrent)?.path === projectKey) {
        client.setQueryData(qk.narrativeScene(file.scene.id), file)
        invalidateNarrative(client)
      } else {
        void client.invalidateQueries({ queryKey: ['narrative_review', projectKey] })
      }
    },
  })
}
