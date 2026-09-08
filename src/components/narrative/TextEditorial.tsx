import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { TextAsset } from '../../lib/api/narrativeText'
import {
  narrativeReviewApply,
  narrativeReviewContext,
  narrativeReviewGet,
  type ReviewTarget,
} from '../../lib/api/narrativeReview'
import { errorMessage, type Scene } from '../../lib/api'
import { NarrativeGeneration } from './NarrativeGeneration'
import { NarrativeContext } from './NarrativeContext'
import { ReviewQueue } from './review/ReviewQueue'
import { WhyAffected } from './WhyAffected'
import { invalidateNarrative } from '../../lib/queries/keys'

/** Supporting text reuses the scene editors' guarded commands and comparisons.
 * The adapter only carries stable asset/entry/slot identities; no scene is saved. */
export function TextEditorial({
  asset,
  projectKey,
  readOnly,
  dirty,
  nameOf,
  onSource,
}: {
  asset: TextAsset
  projectKey: string
  readOnly: boolean
  dirty: boolean
  nameOf: (id: string) => string | undefined
  onSource: (target: ReviewTarget) => void
}) {
  const client = useQueryClient()
  const [mode, setMode] = useState<'review' | 'generate' | 'context' | null>(null)
  const [stateJson, setStateJson] = useState<string | null>(null)
  const [slotId, setSlotId] = useState('')
  const review = useQuery({
    queryKey: ['narrative-review-text', projectKey, asset.id, stateJson],
    queryFn: () => narrativeReviewGet(asset.id, stateJson),
    enabled: mode === 'review',
    retry: false,
  })
  const scene: Scene = {
    id: asset.id,
    name: asset.name,
    participants: asset.participants,
    beats: asset.entries?.map((entry) => ({
      id: entry.id,
      title: entry.label ?? 'Entry',
      dialogue: entry.lines,
    })),
  }
  const lines = (asset.entries ?? []).flatMap((entry) =>
    (entry.lines ?? []).map((slot) => ({ entry, slot })),
  )
  const chosen = lines.find(({ slot }) => slot.id === slotId) ?? lines[0]
  const close = () => {
    setMode(null)
    invalidateNarrative(client)
  }
  return (
    <section aria-label="Text generation and review">
      <div className="ntl-actions">
        <button className="btn" disabled={dirty} onClick={() => setMode('context')}>
          Context
        </button>
        <button className="btn" disabled={dirty || readOnly} onClick={() => setMode('generate')}>
          Generate
        </button>
        <button className="btn" disabled={dirty} onClick={() => setMode('review')}>
          Review
        </button>
      </div>
      {dirty && (
        <p>Save the authored changes before inspecting context, generating or reviewing.</p>
      )}
      {mode === 'generate' && (
        <NarrativeGeneration
          scene={scene}
          projectKey={projectKey}
          readOnly={readOnly}
          onClose={close}
        />
      )}
      {mode === 'context' && (
        <>
          <label>
            Context line
            <select
              value={chosen?.slot.id ?? ''}
              onChange={(event) => setSlotId(event.target.value)}
            >
              {lines.map(({ entry, slot }, index) => (
                <option key={slot.id} value={slot.id}>
                  {entry.label || 'Entry'} · line {index + 1}
                </option>
              ))}
            </select>
          </label>
          {chosen ? (
            <>
              <NarrativeContext
                key={chosen.slot.id}
                selection={{ scene: asset.id, beat: chosen.entry.id, slot: chosen.slot.id }}
                slot={chosen.slot}
              />
              <WhyAffected slotId={chosen.slot.id} />
            </>
          ) : (
            <p>Add and save a line to inspect its context.</p>
          )}
        </>
      )}
      {mode === 'review' && (
        <ReviewQueue
          projectKey={projectKey}
          readOnly={readOnly}
          views={review.data ? [review.data] : []}
          sceneName={() => asset.name}
          speakerName={(id) => nameOf(id) ?? id}
          loading={review.isPending}
          error={review.isError ? errorMessage(review.error) : ''}
          onRefresh={async (state) => {
            setStateJson(state)
            await review.refetch()
          }}
          onApply={async (request) => {
            await narrativeReviewApply(request)
            invalidateNarrative(client)
            await review.refetch()
          }}
          onContext={narrativeReviewContext}
          onSource={(target) => {
            close()
            onSource(target)
          }}
          onClose={close}
        />
      )}
    </section>
  )
}
