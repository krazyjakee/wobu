import { useEffect, useMemo, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import * as api from '../../lib/api'
import type {
  Generation,
  JobPreview,
  JobProgress,
  JobSnapshot,
  QueueSnapshot,
  WobuNode,
} from '../../lib/api'
import { useAssetThumb, useGenerations } from '../../lib/queries'

type Signal = { progress?: JobProgress; preview?: JobPreview }

export function ConceptsPane({ node, queue }: { node: WobuNode; queue: QueueSnapshot }) {
  const history = useGenerations(node.id)
  const signals = useGenerationSignals(node.id)
  const [viewer, setViewer] = useState<{ src: string; label: string } | null>(null)
  const jobs = useMemo(
    () =>
      queue.jobs.filter(
        (job) =>
          job.kind === 'generate' &&
          job.subjectId === node.id &&
          (job.state === 'queued' ||
            job.state === 'running' ||
            job.state === 'retrying' ||
            job.state === 'failed'),
      ),
    [node.id, queue.jobs],
  )

  return (
    <section className="concepts" aria-label={`Concepts for ${node.name}`}>
      {history.isError && (
        <div className="concept-error">Could not read generation history: {api.errorMessage(history.error)}</div>
      )}
      {jobs.length === 0 && (history.data?.length ?? 0) === 0 && !history.isPending && (
        <div className="concept-empty">
          <h3>No concepts yet</h3>
          <p>Generated images for {node.name} will collect here with their prompt and seed.</p>
        </div>
      )}
      <div className="concept-grid">
        {jobs.map((job) => (
          <LiveTile key={job.id} job={job} signal={signals[job.id]} />
        ))}
        {(history.data ?? []).map((generation) => (
          <GenerationTile key={generation.id} generation={generation} onOpen={setViewer} />
        ))}
      </div>
      {viewer && <FullImage viewer={viewer} onClose={() => setViewer(null)} />}
    </section>
  )
}

function useGenerationSignals(nodeId: string): Record<string, Signal> {
  const [signals, setSignals] = useState<Record<string, Signal>>({})

  useEffect(() => {
    setSignals({})
    if (!api.isTauri()) return
    let disposed = false
    const unlisteners: Array<() => void> = []

    const attach = <T,>(name: string, receive: (payload: T) => void) => {
      void listen<T>(name, (event) => receive(event.payload))
        .then((unlisten) => {
          if (disposed) unlisten()
          else unlisteners.push(unlisten)
        })
        .catch(() => {
          /* queue state still gives a truthful card without rich progress */
        })
    }

    attach<JobProgress>(api.JOB_EVENTS.progress, (progress) => {
      setSignals((current) => ({
        ...current,
        [progress.id]: { ...current[progress.id], progress },
      }))
    })
    attach<JobPreview>(api.JOB_EVENTS.preview, (preview) => {
      setSignals((current) => {
        const previous = current[preview.id]?.preview
        if ((preview.step ?? 0) < (previous?.step ?? 0)) return current
        return { ...current, [preview.id]: { ...current[preview.id], preview } }
      })
    })

    return () => {
      disposed = true
      for (const unlisten of unlisteners) unlisten()
    }
  }, [nodeId])

  return signals
}

function LiveTile({ job, signal }: { job: JobSnapshot; signal: Signal | undefined }) {
  const [cancelling, setCancelling] = useState(false)
  const [cancelError, setCancelError] = useState<string | null>(null)
  const progress = signal?.progress
  const pct = progress && progress.total > 0 ? Math.min(100, (progress.done / progress.total) * 100) : null
  const failed = job.state === 'failed' ? job.failure.message : null

  async function cancel() {
    setCancelling(true)
    setCancelError(null)
    try {
      await api.jobCancel(job.id)
    } catch (error) {
      setCancelError(api.errorMessage(error))
    } finally {
      setCancelling(false)
    }
  }

  return (
    <article className={`concept-tile live is-${job.state}`} aria-label={job.label}>
      <div className="concept-image">
        {signal?.preview ? (
          <img src={signal.preview.image} alt={`Live preview for ${job.label}`} />
        ) : (
          <span>{job.state === 'failed' ? 'Generation failed' : 'Waiting for preview…'}</span>
        )}
      </div>
      {pct !== null && (
        <div className="concept-progress" aria-label={`${Math.round(pct)}% complete`}>
          <span style={{ width: `${pct}%` }} />
        </div>
      )}
      <div className="concept-live-meta">
        <span>{progress?.note ?? liveLabel(job)}</span>
        {job.state !== 'failed' && (
          <button className="btn-mini" disabled={cancelling} onClick={() => void cancel()}>
            {cancelling ? 'Stopping…' : 'Cancel'}
          </button>
        )}
      </div>
      {failed && <p className="concept-failure">{failed}</p>}
      {cancelError && <p className="concept-failure">Could not cancel: {cancelError}</p>}
    </article>
  )
}

function liveLabel(job: JobSnapshot): string {
  if (job.state === 'queued') return 'Queued'
  if (job.state === 'running') return 'Generating…'
  if (job.state === 'retrying') return `Retrying in ${Math.ceil(job.inMs / 1_000)}s`
  if (job.state === 'failed') return 'Failed'
  return job.state
}

function GenerationTile({
  generation,
  onOpen,
}: {
  generation: Generation
  onOpen: (viewer: { src: string; label: string }) => void
}) {
  const assetId = generation.outputAssetIds[0] ?? null
  const thumb = useAssetThumb(assetId)
  const [opening, setOpening] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function open() {
    if (!assetId) return
    setOpening(true)
    setError(null)
    try {
      const path = await api.assetOriginal(assetId)
      if (!path) setError('The full-resolution image is not available.')
      else onOpen({ src: convertFileSrc(path), label: generation.compiledPrompt })
    } catch (reason) {
      setError(api.errorMessage(reason))
    } finally {
      setOpening(false)
    }
  }

  return (
    <article className="concept-tile historical">
      <button
        className="concept-open"
        onClick={() => void open()}
        disabled={!assetId || opening}
        aria-label={`Open generation from ${generation.createdAt}`}
      >
        <div className="concept-image">
          {thumb.data ? (
            <img src={convertFileSrc(thumb.data)} alt={generation.compiledPrompt} />
          ) : (
            <span>{assetId ? 'Loading preview…' : 'No output image'}</span>
          )}
          {generation.outputAssetIds.length > 1 && (
            <b className="concept-count">+{generation.outputAssetIds.length - 1}</b>
          )}
          <span className="concept-hover">
            <span>{generation.compiledPrompt}</span>
            <code>seed {generation.seed}</code>
          </span>
        </div>
      </button>
      <div className="concept-caption">
        <span>{generation.model}</span>
        <time dateTime={generation.createdAt}>{new Date(generation.createdAt).toLocaleString()}</time>
      </div>
      {error && <p className="concept-failure">{error}</p>}
    </article>
  )
}

function FullImage({
  viewer,
  onClose,
}: {
  viewer: { src: string; label: string }
  onClose: () => void
}) {
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', close)
    return () => window.removeEventListener('keydown', close)
  }, [onClose])

  return (
    <div className="scrim concept-viewer" role="dialog" aria-label="Full-resolution concept">
      <button className="concept-viewer-close" onClick={onClose} aria-label="Close full-resolution concept">×</button>
      <img src={viewer.src} alt={viewer.label} />
    </div>
  )
}
