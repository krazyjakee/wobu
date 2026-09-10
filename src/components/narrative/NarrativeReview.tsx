import { useMemo, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
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
import { NarrativeLocale } from './NarrativeLocale'
import { NarrativeMedia } from './NarrativeMedia'
import { narrativeMediaGet, mediaKey, mediaStatus } from '../../lib/api/narrativeMedia'
import {
  narrativeLocaleGet,
  translationStatus,
  translationIndex,
  canonicalLocale,
} from '../../lib/api/narrativeLocale'
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
  const [showLocale, setShowLocale] = useState(false)
  const [showMedia, setShowMedia] = useState(false)
  const [recording, setRecording] = useState('')
  const [production, setProduction] = useState('')
  const [locale, setLocale] = useState('fr')
  const localisation = useQuery({
    queryKey: ['narrative_locale', projectKey],
    queryFn: narrativeLocaleGet,
    enabled: Boolean(production),
    retry: false,
  })
  const media = useQuery({
    queryKey: ['narrative_media', projectKey, locale],
    queryFn: () => narrativeMediaGet(locale),
    enabled: Boolean(recording),
    retry: false,
  })
  const mediaMatches = useMemo(() => {
    const invalid = new Set(media.data?.diagnostics.map((d) => d.key))
    return new Set(
      media.data?.rows
        .filter(
          (row) =>
            mediaStatus(
              row,
              media.data?.bindings[mediaKey(row.key)],
              invalid.has(mediaKey(row.key)),
            ) === recording,
        )
        .map((row) => row.key.id),
    )
  }, [media.data, recording])
  const localeIndex = useMemo(() => translationIndex(localisation.data), [localisation.data])
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
    <>
      <ReviewQueue
        projectKey={projectKey}
        readOnly={readOnly}
        views={
          production || recording
            ? views.map((view) => ({
                ...view,
                lines: view.lines.filter(
                  (line) =>
                    line.target.variant &&
                    (!recording || mediaMatches.has(line.target.variant)) &&
                    (!production ||
                      (localisation.data &&
                        translationStatus(
                          localisation.data,
                          locale,
                          line.target.variant,
                          localeIndex,
                        ) === production)),
                ),
              }))
            : views
        }
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
            <button className="btn" onClick={() => setShowLocale(true)}>
              Localisation…
            </button>
            <button className="btn" onClick={() => setShowMedia(true)}>
              Recording…
            </button>
            <label>
              Media readiness{' '}
              <select value={recording} onChange={(e) => setRecording(e.target.value)}>
                <option value="">All media</option>
                <option value="missing">Missing recording</option>
                <option value="out_of_date">Recording out of date</option>
                <option value="unavailable">Recording unavailable</option>
                <option value="current">Recording current</option>
              </select>
            </label>
            {recording && media.isPending && <p role="status">Reading recording readiness…</p>}
            {recording && media.isError && <p role="alert">{errorMessage(media.error)}</p>}
            <label>
              Production locale{' '}
              <input value={locale} onChange={(e) => setLocale(canonicalLocale(e.target.value))} />
            </label>
            <label>
              Translation readiness{' '}
              <select value={production} onChange={(e) => setProduction(e.target.value)}>
                <option value="">All source lines</option>
                <option value="missing">Missing translation</option>
                <option value="draft">Translation draft</option>
                <option value="approved">Translation approved and current</option>
                <option value="out_of_date">Translation out of date</option>
              </select>
            </label>
            {production && localisation.isPending && (
              <p role="status">Reading translation readiness…</p>
            )}
            {production && localisation.isError && (
              <p role="alert">{errorMessage(localisation.error)}</p>
            )}
            <p>
              {views.length} documents loaded of {pages[0]?.total_scenes ?? 0}. Filters search
              loaded documents.{' '}
              {errors.length > 0
                ? 'Some documents could not be read; errors are listed below.'
                : ''}
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
      {showMedia && (
        <NarrativeMedia
          projectKey={projectKey}
          readOnly={readOnly}
          onClose={() => setShowMedia(false)}
        />
      )}
      {showLocale && (
        <NarrativeLocale
          projectKey={projectKey}
          readOnly={readOnly}
          onClose={() => setShowLocale(false)}
        />
      )}
    </>
  )
}
