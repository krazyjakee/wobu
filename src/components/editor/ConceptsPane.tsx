import { useEffect, useMemo, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import * as api from '../../lib/api'
import type {
  AssetRole,
  Generation,
  JobPreview,
  JobProgress,
  JobSnapshot,
  QueueSnapshot,
  WobuNode,
} from '../../lib/api'
import {
  useAssetThumb,
  useGenerations,
  useLinkAsset,
  useNodeLinks,
  useNodes,
  useUnlinkAsset,
} from '../../lib/queries'
import { labelFor, pluralFor, type KindIndex } from '../../lib/kinds'
import { influenceDependentsOf } from '../../lib/tree'

type Signal = { progress?: JobProgress; preview?: JobPreview }

export function ConceptsPane({
  node,
  queue,
  kinds,
  readOnly,
}: {
  node: WobuNode
  queue: QueueSnapshot
  kinds: KindIndex
  readOnly: boolean
}) {
  const history = useGenerations(node.id)
  const nodes = useNodes(true)
  const links = useNodeLinks(true)
  const signals = useGenerationSignals(node.id)
  const [viewer, setViewer] = useState<{ src: string; label: string } | null>(null)
  const dependents = useMemo(
    () => influenceDependentsOf(node.id, nodes.data ?? [], links.data ?? []),
    [links.data, node.id, nodes.data],
  )
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
          <GenerationTile
            key={generation.id}
            generation={generation}
            node={node}
            dependents={dependents}
            kinds={kinds}
            scopeUnknown={
              nodes.isPending || links.isPending || nodes.isError || links.isError
            }
            readOnly={readOnly}
            onOpen={setViewer}
          />
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
  node,
  dependents,
  kinds,
  scopeUnknown,
  readOnly,
  onOpen,
}: {
  generation: Generation
  node: WobuNode
  dependents: ReturnType<typeof influenceDependentsOf>
  kinds: KindIndex
  scopeUnknown: boolean
  readOnly: boolean
  onOpen: (viewer: { src: string; label: string }) => void
}) {
  const assetId = generation.outputAssetIds[0] ?? null
  const thumb = useAssetThumb(assetId)
  const linkAsset = useLinkAsset()
  const unlinkAsset = useUnlinkAsset()
  const [opening, setOpening] = useState(false)
  const pinnedRoles = assetId
    ? node.assetLinks.filter((link) => link.assetId === assetId).map((link) => link.role)
    : []
  const [role, setRole] = useState<AssetRole>(pinnedRoles[0] ?? 'full_ref')
  const [error, setError] = useState<string | null>(null)
  const pinned = pinnedRoles.includes(role)
  const changingPin = linkAsset.isPending || unlinkAsset.isPending

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

  async function togglePin() {
    if (!assetId || readOnly) return
    setError(null)
    try {
      if (pinned) await unlinkAsset.mutateAsync({ nodeId: node.id, assetId, role })
      else await linkAsset.mutateAsync({ nodeId: node.id, assetId, role })
    } catch (reason) {
      setError(`${pinned ? 'Could not unpin' : 'Could not pin'}: ${api.errorMessage(reason)}`)
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
          {pinnedRoles.length > 0 && (
            <b className="concept-pinned">Pinned · {pinnedRoles.map(roleLabel).join(', ')}</b>
          )}
          <span className="concept-hover">
            <span>{generation.compiledPrompt}</span>
            <code>seed {generation.seed}</code>
          </span>
        </div>
      </button>
      <div className="concept-caption">
        <span>{generation.model}</span>
        <span className="concept-seed-source">{seedSourceLabel(generation)}</span>
        <time dateTime={generation.createdAt}>{new Date(generation.createdAt).toLocaleString()}</time>
      </div>
      {assetId && (
        <div className="concept-pin-controls">
          <div className="concept-pin-action">
            <label>
              <span>Reference role</span>
              <select
                aria-label={`Reference role for generation ${generation.id}`}
                value={role}
                disabled={readOnly || changingPin}
                onChange={(event) => setRole(event.target.value as AssetRole)}
              >
                {api.ASSET_ROLES.map((choice) => (
                  <option value={choice} key={choice}>
                    {roleLabel(choice)}
                  </option>
                ))}
              </select>
            </label>
            <button
              className={pinned ? 'btn-mini is-pinned' : 'btn-mini'}
              type="button"
              aria-label={`${pinned ? 'Unpin' : 'Pin'} generation ${generation.id} as ${roleLabel(role)}`}
              aria-pressed={pinned}
              disabled={readOnly || changingPin}
              onClick={() => void togglePin()}
            >
              {changingPin ? 'Saving…' : pinned ? 'Unpin' : 'Pin'}
            </button>
          </div>
          <p className="concept-pin-consequence">
            {pinConsequence(role, node, dependents, kinds, scopeUnknown)}
          </p>
        </div>
      )}
      {error && <p className="concept-failure">{error}</p>}
    </article>
  )
}

function pinConsequence(
  role: AssetRole,
  node: WobuNode,
  dependents: ReturnType<typeof influenceDependentsOf>,
  kinds: KindIndex,
  pending: boolean,
): string {
  if (role === 'mood') {
    return 'Mood is human-only across this entity’s downstream influence stacks; it is never sent to an image model.'
  }
  const nodeKind = labelFor(kinds.get(node.kind), node.kind).toLocaleLowerCase()
  const reach = pending
    ? `this ${nodeKind} and every downstream entity that inherits from it`
    : dependents.length > 0
      ? `this ${nodeKind} and ${dependentSummary(dependents, kinds)} downstream`
      : `this ${nodeKind}`
  return `Future generations for ${reach} can inherit this ${roleConsequence(role)}.`
}

function dependentSummary(
  nodes: ReturnType<typeof influenceDependentsOf>,
  kinds: KindIndex,
): string {
  const counts = new Map<WobuNode['kind'], number>()
  for (const node of nodes) counts.set(node.kind, (counts.get(node.kind) ?? 0) + 1)
  return [...counts.entries()]
    .map(([kind, count]) => {
      const def = kinds.get(kind)
      const name = count === 1 ? labelFor(def, kind) : pluralFor(def, kind)
      return `${count} ${name.toLocaleLowerCase()}`
    })
    .join(', ')
}

function roleConsequence(role: Exclude<AssetRole, 'mood'>): string {
  if (role === 'full_ref') return 'appearance-locking full reference'
  if (role === 'silhouette') return 'structural silhouette reference'
  if (role === 'pose') return 'structural pose reference'
  if (role === 'palette') return 'colour reference'
  if (role === 'material') return 'material style reference'
  return 'costume style reference'
}

function roleLabel(role: AssetRole): string {
  return role === 'full_ref'
    ? 'Full reference'
    : `${role.charAt(0).toUpperCase()}${role.slice(1)}`
}

function seedSourceLabel(generation: Generation): string {
  if (generation.params.usedLockedSeed === true) return 'used locked seed'
  const source = generation.params.seedSource
  switch (source) {
    case 'locked':
      return generation.params.usedLockedSeed === false
        ? 'provider changed locked seed'
        : 'used locked seed'
    case 'locked_derived':
      return 'used locked-seed family'
    case 'rerolled':
      return 'used explicit re-roll'
    case 'rerolled_derived':
      return 'used re-roll family'
    case 'grid':
      return 'variant seed cell'
    case 'random_derived':
      return 'used random-seed family'
    default:
      return 'used unlocked seed'
  }
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
