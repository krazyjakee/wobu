import { useEffect, useMemo, useRef, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import * as api from '../lib/api'
import type { Generation, NodeSummary, ProjectSummary, QueueSnapshot } from '../lib/api'
import type { KindIndex } from '../lib/kinds'
import { useAssetThumb, useGenerations } from '../lib/queries'
import { report, useUI } from '../store/ui'
import { Inspector } from './Inspector'

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
  const sortedNodes = useMemo(
    () => [...nodes].sort((left, right) => left.name.localeCompare(right.name, undefined, { sensitivity: 'base' })),
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
            {sortedNodes.map((node) => <option key={node.id} value={node.id}>{node.name}</option>)}
          </select>
        </label>
        <button className="btn" type="button" onClick={() => setMode('library')}>Back to Library</button>
      </header>

      <Inspector
        project={project}
        selected={selected}
        kinds={kinds}
        onJump={onJump}
        surface="forge"
      />

      {selected ? (
        <ForgeResults subject={selected} queue={queue} />
      ) : (
        <section className="forge-no-subject">
          <h3>Choose a subject</h3>
          <p>Its influence stack, attributed prompt, batch controls, and generation history will fill the Forge.</p>
        </section>
      )}
    </main>
  )
}

function ForgeResults({ subject, queue }: { subject: NodeSummary; queue: QueueSnapshot }) {
  const history = useGenerations(subject.id)
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set())
  const [comparing, setComparing] = useState(false)
  const activeJobs = queue.jobs.filter(
    (job) => job.kind === 'generate' && job.subjectId === subject.id && !isTerminal(job.state),
  )
  const selected = (history.data ?? []).filter((generation) => selectedIds.has(generation.id))

  useEffect(() => {
    setSelectedIds(new Set())
    setComparing(false)
  }, [subject.id])
  useEffect(() => {
    const available = new Set((history.data ?? []).map((generation) => generation.id))
    setSelectedIds((current) => {
      if ([...current].every((id) => available.has(id))) return current
      return new Set([...current].filter((id) => available.has(id)))
    })
  }, [history.data])

  const toggle = (generation: Generation) => {
    if (generation.outputAssetIds.length === 0) return
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
          <span>{history.data?.length ?? 0} receipts{activeJobs.length ? ` · ${activeJobs.length} active` : ''}</span>
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
          <button className="btn-mini" type="button" onClick={() => setSelectedIds(new Set())}>Clear</button>
        )}
      </header>

      {activeJobs.length > 0 && (
        <div className="forge-active" aria-label="Active Forge generations">
          {activeJobs.map((job) => <span key={job.id}>{job.label} · {job.state}</span>)}
        </div>
      )}
      {history.isError ? (
        <p className="forge-results-empty">Could not read results: {api.errorMessage(history.error)}</p>
      ) : (
        <VirtualResultGrid
          generations={history.data ?? []}
          selectedIds={selectedIds}
          loading={history.isPending}
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
  onToggle,
}: {
  generations: Generation[]
  selectedIds: Set<string>
  loading: boolean
  onToggle: (generation: Generation) => void
}) {
  const viewport = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ width: 1100, height: 500 })
  const [scrollTop, setScrollTop] = useState(0)
  const hasResults = generations.length > 0

  useEffect(() => {
    const element = viewport.current
    if (!element) return
    const measure = () =>
      setSize({ width: Math.max(1, element.clientWidth - 24), height: element.clientHeight })
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [hasResults])

  if (generations.length === 0) {
    return (
      <div className="forge-results-empty">
        <h3>{loading ? 'Reading results…' : 'No results yet'}</h3>
        <p>{loading ? 'Receipts will appear as the index answers.' : 'Queue a generation above; completed images collect here.'}</p>
      </div>
    )
  }

  const columns = Math.max(1, Math.floor((size.width + GAP) / (TILE_MIN + GAP)))
  const tileWidth = (size.width - GAP * (columns - 1)) / columns
  const rows = Math.ceil(generations.length / columns)
  const startRow = Math.max(0, Math.floor(scrollTop / TILE_HEIGHT) - OVERSCAN)
  const endRow = Math.min(rows, Math.ceil((scrollTop + size.height) / TILE_HEIGHT) + OVERSCAN)
  const start = startRow * columns
  const end = Math.min(generations.length, endRow * columns)

  return (
    <div
      className="forge-grid-viewport"
      ref={viewport}
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
    >
      <div className="forge-grid" style={{ height: rows * TILE_HEIGHT }}>
        {generations.slice(start, end).map((generation, offset) => {
          const index = start + offset
          const row = Math.floor(index / columns)
          const column = index % columns
          return (
            <ForgeResultTile
              key={generation.id}
              generation={generation}
              selected={selectedIds.has(generation.id)}
              width={tileWidth}
              top={row * TILE_HEIGHT}
              left={column * (tileWidth + GAP)}
              onToggle={() => onToggle(generation)}
            />
          )
        })}
      </div>
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
  generation: Generation
  selected: boolean
  width: number
  top: number
  left: number
  onToggle: () => void
}) {
  const assetId = generation.outputAssetIds[0] ?? null
  const thumb = useAssetThumb(assetId)
  const variation = variationLabel(generation)
  return (
    <button
      className={`forge-result${selected ? ' is-selected' : ''}`}
      style={{ width, height: TILE_HEIGHT - GAP, transform: `translate(${left}px, ${top}px)` }}
      type="button"
      aria-label={`${selected ? 'Remove' : 'Select'} generation ${generation.id} ${selected ? 'from' : 'for'} comparison`}
      aria-pressed={selected}
      disabled={!assetId}
      onClick={onToggle}
    >
      <span className="forge-result-image">
        {thumb.data ? <img src={convertFileSrc(thumb.data)} alt="" /> : <span>{assetId ? 'Loading preview…' : 'No output'}</span>}
        <b>{selected ? 'Selected' : assetId ? 'Compare' : 'Receipt only'}</b>
      </span>
      <span className="forge-result-meta">
        <b>{generation.preset}{variation ? ` · ${variation}` : ''}</b>
        <span>{generation.model} · seed {generation.seed}</span>
        <small>{new Date(generation.createdAt).toLocaleString()}</small>
        <p>{generation.compiledPrompt}</p>
      </span>
    </button>
  )
}

function CompareViewer({ generations, onClose }: { generations: Generation[]; onClose: () => void }) {
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', close)
    return () => window.removeEventListener('keydown', close)
  }, [onClose])

  return (
    <div className="scrim forge-compare-scrim" role="dialog" aria-label="Compare Forge results">
      <section className="forge-compare">
        <header>
          <div><h2>Full-resolution comparison</h2><p>{generations.length} immutable results</p></div>
          <button className="ibtn" type="button" onClick={onClose} aria-label="Close Forge comparison">×</button>
        </header>
        <div className="forge-compare-images">
          {generations.map((generation) => <CompareImage key={generation.id} generation={generation} />)}
        </div>
      </section>
    </div>
  )
}

function CompareImage({ generation }: { generation: Generation }) {
  const [src, setSrc] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const assetId = generation.outputAssetIds[0] ?? null
  useEffect(() => {
    let disposed = false
    setSrc(null)
    setError(null)
    if (!assetId) return
    void api.assetOriginal(assetId)
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
    return () => { disposed = true }
  }, [assetId])
  return (
    <figure>
      <div>{src ? <img src={src} alt={generation.compiledPrompt} /> : <span>{error ?? 'Loading original…'}</span>}</div>
      <figcaption><b>{generation.preset}</b><span>{generation.model} · seed {generation.seed}</span></figcaption>
    </figure>
  )
}

function variationLabel(generation: Generation): string | null {
  const variation = generation.params.variation
  if (!variation || typeof variation !== 'object' || Array.isArray(variation)) return null
  const axis = (variation as Record<string, unknown>).axis
  return typeof axis === 'string' ? axis.replaceAll('_', ' ') : null
}

function isTerminal(state: QueueSnapshot['jobs'][number]['state']): boolean {
  return state === 'done' || state === 'failed' || state === 'cancelled'
}
