import { useEffect, useMemo, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import * as api from '../../lib/api'
import type {
  AssetRole,
  Generation,
  GenerationSummary,
  JobPreview,
  JobProgress,
  JobSnapshot,
  QueueSnapshot,
  WobuNode,
} from '../../lib/api'
import {
  useDeleteGeneration,
  useGenerations,
  useLinkAsset,
  useNodeLinks,
  useNodes,
  useUnlinkAsset,
} from '../../lib/queries'
import { labelFor, pluralFor, type KindIndex } from '../../lib/kinds'
import { influenceDependentsOf } from '../../lib/tree'
import { Combobox } from '../Combobox'
import { GenerationDetail } from '../GenerationDetail'
import { ConfirmSheet } from '../ConfirmSheet'
import { useVirtualCardWindow } from '../useVirtualCardWindow'

type Signal = { progress?: JobProgress; preview?: JobPreview }
const TILE_MIN = 190
const TILE_HEIGHT = 430
const GAP = 12
const OVERSCAN = 2

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
  const [viewer, setViewer] = useState<{ src: string | null; generation: Generation } | null>(null)
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
        <div className="concept-error">
          Could not read generation history: {api.errorMessage(history.error)}
        </div>
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
      </div>
      {(history.data?.length ?? 0) > 0 && (
        <VirtualConceptGrid
          generations={history.data ?? []}
          node={node}
          dependents={dependents}
          kinds={kinds}
          scopeUnknown={nodes.isPending || links.isPending || nodes.isError || links.isError}
          readOnly={readOnly}
          hasMore={history.hasNextPage}
          loadingMore={history.isFetchingNextPage}
          onLoadMore={() => void history.fetchNextPage()}
          onOpen={setViewer}
        />
      )}
      {viewer && (
        <GenerationDetail
          generation={viewer.generation}
          nodeName={node.name}
          imageSrc={viewer.src}
          readOnly={readOnly}
          onClose={() => setViewer(null)}
        />
      )}
    </section>
  )
}

function VirtualConceptGrid({
  generations,
  node,
  dependents,
  kinds,
  scopeUnknown,
  readOnly,
  hasMore,
  loadingMore,
  onLoadMore,
  onOpen,
}: {
  generations: GenerationSummary[]
  node: WobuNode
  dependents: ReturnType<typeof influenceDependentsOf>
  kinds: KindIndex
  scopeUnknown: boolean
  readOnly: boolean
  hasMore: boolean
  loadingMore: boolean
  onLoadMore: () => void
  onOpen: (viewer: { src: string | null; generation: Generation }) => void
}) {
  const { viewportRef, start, end, tileWidth, totalHeight, onScroll, position } =
    useVirtualCardWindow({
      count: generations.length,
      tileMin: TILE_MIN,
      tileHeight: TILE_HEIGHT,
      gap: GAP,
      overscan: OVERSCAN,
      initialWidth: 900,
      initialHeight: 650,
    })
  return (
    <div className="concept-grid-viewport" ref={viewportRef} onScroll={onScroll}>
      <div className="concept-grid concept-grid-virtual" style={{ height: totalHeight }}>
        {generations.slice(start, end).map((generation, offset) => {
          const card = position(start + offset)
          return (
            <GenerationTile
              key={generation.id}
              generation={generation}
              node={node}
              dependents={dependents}
              kinds={kinds}
              scopeUnknown={scopeUnknown}
              readOnly={readOnly}
              width={tileWidth}
              top={card.top}
              left={card.left}
              onOpen={onOpen}
            />
          )
        })}
      </div>
      {hasMore && (
        <button
          className="btn concept-more"
          type="button"
          disabled={loadingMore}
          onClick={onLoadMore}
        >
          {loadingMore ? 'Loading more…' : 'Load more concepts'}
        </button>
      )}
    </div>
  )
}

function useGenerationSignals(nodeId: string): Record<string, Signal> {
  const [signalsByNode, setSignalsByNode] = useState<Record<string, Record<string, Signal>>>({})

  useEffect(() => {
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
      setSignalsByNode((all) => {
        const current = all[nodeId] ?? {}
        return {
          ...all,
          [nodeId]: { ...current, [progress.id]: { ...current[progress.id], progress } },
        }
      })
    })
    attach<JobPreview>(api.JOB_EVENTS.preview, (preview) => {
      setSignalsByNode((all) => {
        const current = all[nodeId] ?? {}
        const previous = current[preview.id]?.preview
        if ((preview.step ?? 0) < (previous?.step ?? 0)) return all
        return {
          ...all,
          [nodeId]: { ...current, [preview.id]: { ...current[preview.id], preview } },
        }
      })
    })

    return () => {
      disposed = true
      for (const unlisten of unlisteners) unlisten()
    }
  }, [nodeId])

  return signalsByNode[nodeId] ?? {}
}

function LiveTile({ job, signal }: { job: JobSnapshot; signal: Signal | undefined }) {
  const [cancelling, setCancelling] = useState(false)
  const [cancelError, setCancelError] = useState<string | null>(null)
  const progress = signal?.progress
  const pct =
    progress && progress.total > 0 ? Math.min(100, (progress.done / progress.total) * 100) : null
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
  width,
  top,
  left,
  onOpen,
}: {
  generation: GenerationSummary
  node: WobuNode
  dependents: ReturnType<typeof influenceDependentsOf>
  kinds: KindIndex
  scopeUnknown: boolean
  readOnly: boolean
  width: number
  top: number
  left: number
  onOpen: (viewer: { src: string | null; generation: Generation }) => void
}) {
  const assetId = generation.firstAssetId
  const linkAsset = useLinkAsset()
  const unlinkAsset = useUnlinkAsset()
  const deleteGeneration = useDeleteGeneration()
  const [opening, setOpening] = useState(false)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const pinnedRoles = assetId
    ? node.assetLinks.filter((link) => link.assetId === assetId).map((link) => link.role)
    : []
  const [role, setRole] = useState<AssetRole>(pinnedRoles[0] ?? 'full_ref')
  const [error, setError] = useState<string | null>(null)
  const pinned = pinnedRoles.includes(role)
  const changingPin = linkAsset.isPending || unlinkAsset.isPending

  async function open() {
    setOpening(true)
    setError(null)
    try {
      const receipt = await api.generationGet(generation.id)
      if (!receipt) throw new Error('The immutable generation receipt is no longer indexed.')
      const outputId = receipt.outputAssetIds[0] ?? null
      const path = outputId ? await api.assetOriginal(outputId) : null
      if (!path) {
        if (outputId) {
          setError(
            'The full-resolution image is not available; the immutable receipt can still be inspected.',
          )
        }
        onOpen({ src: null, generation: receipt })
      } else onOpen({ src: convertFileSrc(path), generation: receipt })
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
    <article
      className="concept-tile historical"
      style={{ width, height: TILE_HEIGHT - GAP, transform: `translate(${left}px, ${top}px)` }}
    >
      <button
        className="concept-open"
        onClick={() => void open()}
        disabled={opening}
        aria-label={`Open generation from ${generation.createdAt}`}
      >
        <div className="concept-image">
          {generation.thumbnailPath ? (
            <img src={convertFileSrc(generation.thumbnailPath)} alt={generation.promptExcerpt} />
          ) : (
            <span>{assetId ? 'Loading preview…' : 'No output image'}</span>
          )}
          {generation.outputCount > 1 && (
            <b className="concept-count">+{generation.outputCount - 1}</b>
          )}
          {pinnedRoles.length > 0 && (
            <b className="concept-pinned">Pinned · {pinnedRoles.map(roleLabel).join(', ')}</b>
          )}
          <span className="concept-hover">
            <span>{generation.promptExcerpt}</span>
            <code>seed {generation.seed}</code>
          </span>
        </div>
      </button>
      <div className="concept-caption">
        <span>{generation.model}</span>
        <span className="concept-seed-source">{seedSourceLabel(generation)}</span>
        <time dateTime={generation.createdAt}>
          {new Date(generation.createdAt).toLocaleString()}
        </time>
      </div>
      {assetId && (
        <div className="concept-pin-controls">
          <div className="concept-pin-action">
            <label>
              <span>Reference role</span>
              <Combobox
                label={`Reference role for generation ${generation.id}`}
                value={role}
                options={ROLE_OPTIONS}
                disabled={readOnly || changingPin}
                onChange={(next) => setRole(next as AssetRole)}
              />
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
      <button
        className="btn-mini concept-delete"
        type="button"
        disabled={readOnly || deleteGeneration.isPending}
        onClick={() => setConfirmDelete(true)}
        aria-label={`Delete generation ${generation.id}`}
      >
        Delete concept…
      </button>
      {error && <p className="concept-failure">{error}</p>}
      {confirmDelete && (
        <ConfirmSheet
          title="Delete this concept?"
          body="It will disappear from Concepts and the Asset Library, and its images will be deleted. Any image you pinned as a reference or set as a cover is kept, as is its archived receipt in spend accounting."
          confirmLabel="Delete concept"
          danger
          busy={deleteGeneration.isPending}
          onCancel={() => setConfirmDelete(false)}
          onConfirm={() =>
            deleteGeneration.mutate(
              { generationId: generation.id, nodeId: generation.nodeId },
              { onSuccess: () => setConfirmDelete(false) },
            )
          }
        />
      )}
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
  return role === 'full_ref' ? 'Full reference' : `${role.charAt(0).toUpperCase()}${role.slice(1)}`
}

/** Built once at module scope: the same five rows on every concept card. */
const ROLE_OPTIONS = api.ASSET_ROLES.map((role) => ({ value: role, label: roleLabel(role) }))

function seedSourceLabel(generation: GenerationSummary): string {
  if (generation.seedSource === 'replay') return 'replayed snapshot'
  if (generation.usedLockedSeed === true) return 'used locked seed'
  const source = generation.seedSource
  switch (source) {
    case 'locked':
      return generation.usedLockedSeed === false
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
