import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { errorMessage } from '../../lib/api'
import { narrativeReviewContext, type ReviewTarget } from '../../lib/api/narrativeReview'
import {
  useNarrativeReviewApply,
  useNarrativeReviewProvenance,
  useNarrativeReviewQueue,
} from '../../lib/queries/narrativeReview'
import { invalidateNarrative } from '../../lib/queries/keys'
import { ReviewQueue } from './review/ReviewQueue'
import { ReviewBulk } from './review/ReviewBulk'
export function NarrativeReview({
  projectKey,
  readOnly,
  sceneName,
  speakerName,
  onSource,
  onClose,
}: {
  projectKey: string
  readOnly: boolean
  sceneName: (id: string) => string
  speakerName: (id: string) => string
  onSource: (target: ReviewTarget) => void
  onClose: () => void
}) {
  const [stateJson, setStateJson] = useState<string | null>(null)
  const queue = useNarrativeReviewQueue(projectKey, stateJson)
  const provenance = useNarrativeReviewProvenance(projectKey)
  const apply = useNarrativeReviewApply(projectKey)
  const client = useQueryClient()
  const pages = queue.data?.pages ?? []
  const errors = pages.flatMap((page) =>
    page.errors.map(
      (error) => `${error.scene_id ? sceneName(error.scene_id) + ': ' : ''}${error.reason}`,
    ),
  )
  const views = pages.flatMap((page) => page.scenes)
  const refresh = async (state: string | null) => {
    if (state !== stateJson) setStateJson(state)
    else await queue.refetch()
    await provenance.refetch()
  }
  return (
    <ReviewQueue
      projectKey={projectKey}
      readOnly={readOnly}
      views={views}
      sceneName={sceneName}
      speakerName={speakerName}
      loading={queue.isFetching}
      error={[
        queue.error ? errorMessage(queue.error) : '',
        provenance.error ? `Provenance unavailable: ${errorMessage(provenance.error)}` : '',
        ...errors,
      ]
        .filter(Boolean)
        .join('\n')}
      generationHistory={provenance.data}
      onRefresh={refresh}
      onApply={async (request) => {
        await apply.mutateAsync(request)
      }}
      onContext={narrativeReviewContext}
      onSource={onSource}
      onClose={onClose}
      coverage={
        <div className="nrt-review-toolbar">
          <p>
            {views.length} scenes loaded of {pages[0]?.total_scenes ?? 0}. Filters search loaded
            scenes.{' '}
            {errors.length > 0 ? 'Some scenes could not be read; errors are listed below.' : ''}
          </p>
          {queue.hasNextPage && (
            <button
              className="btn"
              disabled={queue.isFetching}
              onClick={() => void queue.fetchNextPage()}
            >
              Load more scenes
            </button>
          )}
        </div>
      }
      renderBulk={(rows) => (
        <ReviewBulk
          rows={rows}
          projectKey={projectKey}
          readOnly={readOnly}
          onApplied={() => invalidateNarrative(client)}
        />
      )}
    />
  )
}
