import { useEffect, useMemo, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import * as api from '../lib/api'
import type { GenerationSummary, NodeSummary, ProjectSummary, QueueSnapshot } from '../lib/api'
import { negotiatedAspect } from '../lib/generationCapabilities'
import type { KindIndex } from '../lib/kinds'
import {
  useGenerations,
  useImageGenerationCapabilities,
  useLoraStatus,
  useStatusBarBackend,
  useTrainLora,
} from '../lib/queries'
import { report, useUI } from '../store/ui'
import { GenerationAspectSelect } from './GenerationAspectSelect'
import { Inspector } from './Inspector'
import { Modal } from './Modal'
import { useVirtualCardWindow } from './useVirtualCardWindow'

const TILE_MIN = 230
const TILE_HEIGHT = 286
const GAP = 12
const OVERSCAN = 2
const MAX_COMPARE = 4

export function ForgeMode({
  project,
  nodes,
  selected,
  kinds,
  queue,
  onJump,
}: {
  project: ProjectSummary
  nodes: NodeSummary[]
  selected: NodeSummary | null
  kinds: KindIndex
  queue: QueueSnapshot
  onJump: (id: string) => void
}) {
  const select = useUI((state) => state.select)
  const setMode = useUI((state) => state.setMode)
  const backend = useStatusBarBackend(project.id)
  const sortedNodes = useMemo(
    () =>
      [...nodes].sort((left, right) =>
        left.name.localeCompare(right.name, undefined, { sensitivity: 'base' }),
      ),
    [nodes],
  )

  return (
    <main className="forge-mode" aria-label="Forge">
      <header className="forge-head">
        <div>
          <h2>Forge</h2>
          <p>Iterate on one subject with the complete influence attribution kept in view.</p>
        </div>
        <label>
          <span>Subject</span>
          <select
            aria-label="Forge subject"
            value={selected?.id ?? ''}
            onChange={(event) => select(event.target.value || null)}
          >
            <option value="">Choose an entity</option>
            {sortedNodes.map((node) => (
              <option key={node.id} value={node.id}>
                {node.name}
              </option>
            ))}
          </select>
        </label>
        <button className="btn" type="button" onClick={() => setMode('library')}>
          Back to Library
        </button>
      </header>

      <SceneComposer
        key={`scene:${selected?.id ?? 'none'}`}
        primary={selected}
        nodes={sortedNodes}
        kinds={kinds}
        projectId={project.id}
        model={backend.data?.image?.model}
        readOnly={project.readOnly}
      />

      <LoraCard subject={selected} readOnly={project.readOnly} queue={queue} />

      <Inspector
        project={project}
        selected={selected}
        kinds={kinds}
        onJump={onJump}
        surface="forge"
      />

      {selected ? (
        <ForgeResults key={`results:${selected.id}`} subject={selected} queue={queue} />
      ) : (
        <section className="forge-no-subject empty-state">
          <h3>Choose a subject</h3>
          <p>
            Its influence stack, attributed prompt, batch controls, and generation history will fill
            the Forge.
          </p>
        </section>
      )}
    </main>
  )
}

function LoraCard({
  subject,
  readOnly,
  queue,
}: {
  subject: NodeSummary | null
  readOnly: boolean
  queue: QueueSnapshot
}) {
  const status = useLoraStatus(subject?.id ?? null)
  const train = useTrainLora()
  const active =
    !!subject &&
    queue.jobs.some(
      (job) => job.kind === 'train_lora' && job.subjectId === subject.id && !isTerminal(job.state),
    )

  if (!subject) return null
  if (status.isPending) {
    return <section className="forge-lora">Checking LoRA readiness…</section>
  }
  if (status.isError || !status.data) {
    return (
      <section className="forge-lora" aria-label={`LoRA for ${subject.name}`}>
        <b>Entity LoRA</b>
        <span>Could not inspect LoRA readiness: {api.errorMessage(status.error)}</span>
      </section>
    )
  }

  const value = status.data
  const subjectId = subject.id
  const subjectName = subject.name
  const blocked = readOnly || active || train.isPending || !value.eligible
  const action = value.pin ? 'Re-train' : 'Train'
  const disabledReason = readOnly
    ? 'This project is read-only.'
    : active
      ? 'Training is already active for this entity.'
      : !value.eligible
        ? 'Resolve the reference, trainer, provider, and model requirements first.'
        : undefined

  async function start() {
    try {
      await train.mutateAsync(subjectId)
    } catch (reason) {
      report(reason, `Could not train a LoRA for ${subjectName}`)
    }
  }

  return (
    <section className="forge-lora" aria-label={`LoRA for ${subject.name}`}>
      <div className="forge-lora-title">
        <b>Entity LoRA</b>
        <span data-state={value.applicationState}>
          {value.applicationState.replaceAll('_', ' ')}
        </span>
      </div>
      <div className="forge-lora-counts">
        <b>
          {value.pinnedCount} / {value.requiredCount}
        </b>{' '}
        valid full references
        <span> · {value.invalidPinnedCount} invalid</span>
      </div>
      <dl>
        <div>
          <dt>Trainer</dt>
          <dd>
            <b>{value.trainerState}</b> · {value.trainerDetail}
          </dd>
        </div>
        <div>
          <dt>Model</dt>
          <dd>{value.selectedModel ?? 'No image checkpoint selected'}</dd>
        </div>
        <div>
          <dt>Application</dt>
          <dd>{value.applicationDetail}</dd>
        </div>
        {value.pin && (
          <div>
            <dt>Weights</dt>
            <dd>
              {value.pin.trainer} · {value.pin.baseModel} · strength {value.pin.strength.toFixed(2)}
            </dd>
          </div>
        )}
      </dl>
      <button
        className="btn"
        type="button"
        aria-label={`${action} LoRA for ${subject.name}`}
        disabled={blocked}
        title={disabledReason}
        onClick={() => void start()}
      >
        {active ? 'Training…' : train.isPending ? 'Starting…' : `${action} LoRA`}
      </button>
    </section>
  )
}

function SceneComposer({
  primary,
  nodes,
  kinds,
  projectId,
  model,
  readOnly,
}: {
  primary: NodeSummary | null
  nodes: NodeSummary[]
  kinds: KindIndex
  projectId: string
  model: string | undefined
  readOnly: boolean
}) {
  const [additional, setAdditional] = useState<string[]>([])
  const [prompt, setPrompt] = useState('')
  const [aspect, setAspect] = useState('16:9')
  const [busy, setBusy] = useState(false)
  const [status, setStatus] = useState<string | null>(null)
  const candidates = nodes.filter(
    (node) => node.id !== primary?.id && !kinds.get(node.kind)?.singleton,
  )
  const primaryAllowed = primary ? !kinds.get(primary.kind)?.singleton : false
  const capabilities = useImageGenerationCapabilities(projectId, model, !!model)
  const aspectChoices = capabilities.data?.aspectRatios ?? []
  const aspectNegotiation = useMemo(
    () => negotiatedAspect(capabilities.data, aspect),
    [aspect, capabilities.data],
  )
  const aspectAdjustment =
    aspectNegotiation &&
    !capabilities.isPlaceholderData &&
    aspectNegotiation.actualAspect !== aspect
      ? `${aspectNegotiation.requestedValid ? 'Unsupported saved aspect' : 'Malformed saved aspect'} ${aspect} was replaced with ${aspectNegotiation.actualAspect}.`
      : null
  const normalizedAspect = aspectNegotiation?.actualAspect ?? aspect

  function toggle(id: string) {
    setAdditional((current) => {
      if (current.includes(id)) return current.filter((value) => value !== id)
      if (current.length >= 3) return current
      return [...current, id]
    })
  }

  async function generate() {
    if (!primary || additional.length === 0) return
    setBusy(true)
    setStatus(null)
    try {
      await api.sceneGenerateStart([primary.id, ...additional], {
        prompt: prompt.trim() || undefined,
        aspect: normalizedAspect,
      })
      setStatus(`Scene queued with ${1 + additional.length} entities.`)
    } catch (reason) {
      setStatus(api.errorMessage(reason))
      report(reason, 'Could not queue scene composition')
    } finally {
      setBusy(false)
    }
  }

  return (
    <details className="forge-scene">
      <summary>Compose a multi-entity scene</summary>
      {!primary ? (
        <p>Choose the primary Forge subject first.</p>
      ) : !primaryAllowed ? (
        <p>Style Guides and World Bibles shape a scene, but cannot be participants.</p>
      ) : (
        <div className="forge-scene-body">
          <div className="forge-scene-entities">
            <b>Primary · {primary.name}</b>
            <span>Add one to three entities in prompt order.</span>
            <div role="group" aria-label="Additional scene entities">
              {candidates.map((node) => (
                <label key={node.id}>
                  <input
                    type="checkbox"
                    checked={additional.includes(node.id)}
                    disabled={!additional.includes(node.id) && additional.length >= 3}
                    onChange={() => toggle(node.id)}
                  />
                  {node.name}
                </label>
              ))}
            </div>
          </div>
          <label className="forge-scene-prompt">
            <span>Scene direction</span>
            <textarea
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
              placeholder="Crossing the flooded market at blue hour…"
            />
          </label>
          <label>
            <span>Aspect</span>
            <GenerationAspectSelect
              label="Scene aspect"
              value={normalizedAspect}
              choices={aspectChoices}
              onChange={setAspect}
            />
          </label>
          {capabilities.data && aspectNegotiation && !capabilities.isPlaceholderData && (
            <p className="forge-scene-aspect" role="status">
              Actual output · {aspectNegotiation.actualAspect} · {aspectNegotiation.width}×
              {aspectNegotiation.height}px
              {capabilities.data.flexibleAspect
                ? " · flexible backend, using Wobu's curated validated choices"
                : ''}
            </p>
          )}
          {aspectAdjustment && (
            <p className="forge-scene-aspect is-substituted" role="status">
              {aspectAdjustment}
            </p>
          )}
          <button
            className="btn btn-primary"
            type="button"
            disabled={
              readOnly ||
              busy ||
              additional.length === 0 ||
              !capabilities.data ||
              capabilities.isFetching ||
              capabilities.isPlaceholderData ||
              !aspectNegotiation ||
              !aspectChoices.includes(normalizedAspect)
            }
            onClick={() => void generate()}
          >
            {busy ? 'Queuing scene…' : 'Generate scene'}
          </button>
          {status && <p role="status">{status}</p>}
        </div>
      )}
    </details>
  )
}

function ForgeResults({ subject, queue }: { subject: NodeSummary; queue: QueueSnapshot }) {
  const history = useGenerations(subject.id)
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set())
  const [comparing, setComparing] = useState(false)
  const activeJobs = queue.jobs.filter(
    (job) => job.kind === 'generate' && job.subjectId === subject.id && !isTerminal(job.state),
  )
  const available = new Set((history.data ?? []).map((generation) => generation.id))
  const visibleSelectedIds = new Set([...selectedIds].filter((id) => available.has(id)))
  const selected = (history.data ?? []).filter((generation) =>
    visibleSelectedIds.has(generation.id),
  )

  const toggle = (generation: GenerationSummary) => {
    if (generation.outputCount === 0) return
    setSelectedIds((current) => {
      const next = new Set(current)
      if (next.has(generation.id)) next.delete(generation.id)
      else if (next.size < MAX_COMPARE) next.add(generation.id)
      return next
    })
  }

  return (
    <section className="forge-results" aria-label={`Forge results for ${subject.name}`}>
      <header>
        <div>
          <h3>Results</h3>
          <span>
            {history.total} receipts
            {activeJobs.length ? ` · ${activeJobs.length} active` : ''}
          </span>
        </div>
        <p>Select up to {MAX_COMPARE} completed results for a full-resolution comparison.</p>
        <button
          className="btn"
          type="button"
          disabled={selected.length < 2}
          onClick={() => setComparing(true)}
        >
          Compare selected · {selected.length}
        </button>
        {selected.length > 0 && (
          <button className="btn-mini" type="button" onClick={() => setSelectedIds(new Set())}>
            Clear
          </button>
        )}
      </header>

      {activeJobs.length > 0 && (
        <div className="forge-active" aria-label="Active Forge generations">
          {activeJobs.map((job) => (
            <span key={job.id}>
              {job.label} · {job.state}
            </span>
          ))}
        </div>
      )}
      {history.isError ? (
        <p className="forge-results-empty empty-state">
          Could not read results: {api.errorMessage(history.error)}
        </p>
      ) : (
        <VirtualResultGrid
          generations={history.data ?? []}
          selectedIds={visibleSelectedIds}
          loading={history.isPending}
          hasMore={history.hasNextPage}
          loadingMore={history.isFetchingNextPage}
          onLoadMore={() => void history.fetchNextPage()}
          onToggle={toggle}
        />
      )}

      {comparing && selected.length >= 2 && (
        <CompareViewer generations={selected} onClose={() => setComparing(false)} />
      )}
    </section>
  )
}

function VirtualResultGrid({
  generations,
  selectedIds,
  loading,
  hasMore,
  loadingMore,
  onLoadMore,
  onToggle,
}: {
  generations: GenerationSummary[]
  selectedIds: Set<string>
  loading: boolean
  hasMore: boolean
  loadingMore: boolean
  onLoadMore: () => void
  onToggle: (generation: GenerationSummary) => void
}) {
  const { viewportRef, start, end, tileWidth, totalHeight, onScroll, position } =
    useVirtualCardWindow({
      count: generations.length,
      tileMin: TILE_MIN,
      tileHeight: TILE_HEIGHT,
      gap: GAP,
      overscan: OVERSCAN,
      initialWidth: 1100,
      initialHeight: 500,
    })

  if (generations.length === 0) {
    return (
      <div className="forge-results-empty empty-state">
        <h3>{loading ? 'Reading results…' : 'No results yet'}</h3>
        <p>
          {loading
            ? 'Receipts will appear as the index answers.'
            : 'Queue a generation above; completed images collect here.'}
        </p>
      </div>
    )
  }

  return (
    <div className="forge-grid-viewport" ref={viewportRef} onScroll={onScroll}>
      <div className="forge-grid" style={{ height: totalHeight }}>
        {generations.slice(start, end).map((generation, offset) => {
          const index = start + offset
          const cardPosition = position(index)
          return (
            <ForgeResultTile
              key={generation.id}
              generation={generation}
              selected={selectedIds.has(generation.id)}
              width={tileWidth}
              top={cardPosition.top}
              left={cardPosition.left}
              onToggle={() => onToggle(generation)}
            />
          )
        })}
      </div>
      {hasMore && (
        <button className="btn forge-more" type="button" disabled={loadingMore} onClick={onLoadMore}>
          {loadingMore ? 'Loading more…' : 'Load more results'}
        </button>
      )}
    </div>
  )
}

function ForgeResultTile({
  generation,
  selected,
  width,
  top,
  left,
  onToggle,
}: {
  generation: GenerationSummary
  selected: boolean
  width: number
  top: number
  left: number
  onToggle: () => void
}) {
  const assetId = generation.firstAssetId
  const subject = generation.sceneSubjectNames.length
    ? `Scene · ${generation.sceneSubjectNames.join(' + ')}`
    : generation.preset
  return (
    <button
      className={`forge-result media-card selectable-media-card${selected ? ' is-selected' : ''}`}
      style={{ width, height: TILE_HEIGHT - GAP, transform: `translate(${left}px, ${top}px)` }}
      type="button"
      aria-label={`${selected ? 'Remove' : 'Select'} generation ${generation.id} ${selected ? 'from' : 'for'} comparison`}
      aria-pressed={selected}
      disabled={!assetId}
      onClick={onToggle}
    >
      <span className="forge-result-image asset-media-frame">
        {generation.thumbnailPath ? (
          <img src={convertFileSrc(generation.thumbnailPath)} alt="" loading="lazy" decoding="async" />
        ) : (
          <span>{assetId ? 'Loading preview…' : 'No output'}</span>
        )}
        <b>{selected ? 'Selected' : assetId ? 'Compare' : 'Receipt only'}</b>
      </span>
      <span className="forge-result-meta media-card-copy">
        <b>
          {subject}
        </b>
        <span>
          {generation.model} · seed {generation.seed}
        </span>
        <small>
          {new Date(generation.createdAt).toLocaleString()}
        </small>
        <p>{generation.promptExcerpt}</p>
      </span>
    </button>
  )
}

function CompareViewer({
  generations,
  onClose,
}: {
  generations: GenerationSummary[]
  onClose: () => void
}) {
  return (
    <Modal
      className="forge-compare"
      scrimClassName="forge-compare-scrim"
      titleId="forge-compare-title"
      descriptionId="forge-compare-description"
      onClose={onClose}
    >
      <header>
        <div>
          <h2 id="forge-compare-title">Compare Forge results</h2>
          <p id="forge-compare-description">
            Full-resolution comparison · {generations.length} immutable results
          </p>
        </div>
        <button
          className="ibtn"
          type="button"
          onClick={onClose}
          aria-label="Close Forge comparison"
          data-modal-initial-focus
        >
          ×
        </button>
      </header>
      <div className="forge-compare-images">
        {generations.map((generation) => (
          <CompareImage
            key={`${generation.id}:${generation.firstAssetId ?? 'none'}`}
            generation={generation}
          />
        ))}
      </div>
    </Modal>
  )
}

function CompareImage({ generation }: { generation: GenerationSummary }) {
  const [src, setSrc] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const assetId = generation.firstAssetId
  useEffect(() => {
    let disposed = false
    if (!assetId) return
    void api
      .assetOriginal(assetId)
      .then((path) => {
        if (!disposed) {
          if (path) setSrc(convertFileSrc(path))
          else setError('Original missing')
        }
      })
      .catch((reason) => {
        if (!disposed) {
          setError(api.errorMessage(reason))
          report(reason, 'Could not load a comparison original')
        }
      })
    return () => {
      disposed = true
    }
  }, [assetId])
  return (
    <figure>
      <div>
        {src ? (
          <img src={src} alt={generation.promptExcerpt} />
        ) : (
          <span>{error ?? 'Loading original…'}</span>
        )}
      </div>
      <figcaption>
        <b>
          {generation.sceneSubjectNames.length
            ? `Scene · ${generation.sceneSubjectNames.join(' + ')}`
            : generation.preset}
        </b>
        <span>
          {generation.model} · seed {generation.seed}
        </span>
      </figcaption>
    </figure>
  )
}

function isTerminal(state: QueueSnapshot['jobs'][number]['state']): boolean {
  return state === 'done' || state === 'failed' || state === 'cancelled'
}
