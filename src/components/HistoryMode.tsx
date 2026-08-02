import { useMemo, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import * as api from '../lib/api'
import type { Generation, GenerationSummary, NodeSummary } from '../lib/api'
import { useGenerationHistory } from '../lib/queries'
import { report, useUI } from '../store/ui'
import { GenerationDetail } from './GenerationDetail'
import { useVirtualCardWindow } from './useVirtualCardWindow'

const TILE_MIN = 250
const TILE_HEIGHT = 330
const GAP = 12
const OVERSCAN = 2

export function HistoryMode({
  nodes,
  readOnly,
  onJump,
}: {
  nodes: NodeSummary[]
  readOnly: boolean
  onJump: (id: string) => void
}) {
  const setMode = useUI((state) => state.setMode)
  const [preset, setPreset] = useState('all')
  const [model, setModel] = useState('all')
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')
  const [seed, setSeed] = useState('')
  const [opened, setOpened] = useState<{ generation: Generation; imageSrc: string | null } | null>(
    null,
  )
  const filters = useMemo<api.GenerationFilters>(
    () => ({
      preset: preset === 'all' ? undefined : preset,
      model: model === 'all' ? undefined : model,
      from: from || undefined,
      to: to || undefined,
      seed: seed ? Number(seed) : undefined,
    }),
    [from, model, preset, seed, to],
  )
  const history = useGenerationHistory(filters)
  const names = useMemo(() => new Map(nodes.map((node) => [node.id, node.name])), [nodes])
  const receipts = useMemo(() => history.data ?? [], [history.data])

  async function open(summary: GenerationSummary) {
    try {
      const generation = await api.generationGet(summary.id)
      if (!generation) throw new Error('The immutable generation receipt is no longer indexed.')
      const assetId = generation.outputAssetIds[0] ?? null
      const path = assetId ? await api.assetOriginal(assetId) : null
      setOpened({ generation, imageSrc: path ? convertFileSrc(path) : null })
    } catch (error) {
      report(error, 'Could not open the recorded image')
    }
  }

  return (
    <main className="history-mode" aria-label="Generation history">
      <header className="history-mode-head">
        <div>
          <h2>Generation history</h2>
          <p>{history.total} immutable receipts across this project</p>
        </div>
        <button className="btn" type="button" onClick={() => setMode('library')}>
          Back to Library
        </button>
      </header>

      <div className="history-filters" aria-label="Generation history filters">
        <label>
          <span>Preset</span>
          <select
            aria-label="Filter history by preset"
            value={preset}
            onChange={(event) => setPreset(event.target.value)}
          >
            <option value="all">All presets</option>
            {history.presets.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>Model</span>
          <select
            aria-label="Filter history by model"
            value={model}
            onChange={(event) => setModel(event.target.value)}
          >
            <option value="all">All models</option>
            {history.models.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>From</span>
          <input
            aria-label="Filter history from date"
            type="date"
            value={from}
            onChange={(event) => setFrom(event.target.value)}
          />
        </label>
        <label>
          <span>To</span>
          <input
            aria-label="Filter history to date"
            type="date"
            value={to}
            onChange={(event) => setTo(event.target.value)}
          />
        </label>
        <label>
          <span>Seed</span>
          <input
            aria-label="Filter history by seed"
            inputMode="numeric"
            value={seed}
            onChange={(event) => setSeed(event.target.value.replace(/\D/g, ''))}
            placeholder="Any seed"
          />
        </label>
      </div>

      {history.isError && (
        <p className="history-error inline-error">
          Could not read generation history: {api.errorMessage(history.error)}
        </p>
      )}
      {history.isPending && (
        <p className="history-empty empty-state">Reading generation history…</p>
      )}
      {!history.isPending && !history.isError && receipts.length === 0 && (
        <p className="history-empty empty-state">No generations match these filters.</p>
      )}
      {receipts.length > 0 && (
        <VirtualHistoryGrid
          generations={receipts}
          names={names}
          loadingMore={history.isFetchingNextPage}
          hasMore={history.hasNextPage}
          onLoadMore={() => void history.fetchNextPage()}
          onOpen={(generation) => void open(generation)}
          onJump={onJump}
        />
      )}

      {opened && (
        <GenerationDetail
          generation={opened.generation}
          nodeName={names.get(opened.generation.nodeId) ?? 'Deleted entity'}
          imageSrc={opened.imageSrc}
          readOnly={readOnly}
          onClose={() => setOpened(null)}
        />
      )}
    </main>
  )
}

function VirtualHistoryGrid({
  generations,
  names,
  loadingMore,
  hasMore,
  onLoadMore,
  onOpen,
  onJump,
}: {
  generations: GenerationSummary[]
  names: Map<string, string>
  loadingMore: boolean
  hasMore: boolean
  onLoadMore: () => void
  onOpen: (generation: GenerationSummary) => void
  onJump: (id: string) => void
}) {
  const { viewportRef, start, end, tileWidth, totalHeight, onScroll, position } =
    useVirtualCardWindow({
      count: generations.length,
      tileMin: TILE_MIN,
      tileHeight: TILE_HEIGHT,
      gap: GAP,
      overscan: OVERSCAN,
      initialWidth: 1100,
      initialHeight: 650,
    })
  return (
    <div className="history-grid-viewport" ref={viewportRef} onScroll={onScroll}>
      <div className="history-grid" style={{ height: totalHeight }}>
        {generations.slice(start, end).map((generation, offset) => {
          const card = position(start + offset)
          return (
            <HistoryTile
              key={generation.id}
              generation={generation}
              nodeName={names.get(generation.nodeId) ?? 'Deleted entity'}
              width={tileWidth}
              top={card.top}
              left={card.left}
              onOpen={() => onOpen(generation)}
              onJump={names.has(generation.nodeId) ? () => onJump(generation.nodeId) : null}
            />
          )
        })}
      </div>
      {hasMore && (
        <button className="btn history-more" type="button" disabled={loadingMore} onClick={onLoadMore}>
          {loadingMore ? 'Loading more…' : 'Load more receipts'}
        </button>
      )}
    </div>
  )
}

function HistoryTile({
  generation,
  nodeName,
  width,
  top,
  left,
  onOpen,
  onJump,
}: {
  generation: GenerationSummary
  nodeName: string
  width: number
  top: number
  left: number
  onOpen: () => void
  onJump: (() => void) | null
}) {
  const subject = generation.sceneSubjectNames.length
    ? `Scene · ${generation.sceneSubjectNames.join(' + ')}`
    : nodeName
  return (
    <article
      className="history-tile media-card"
      style={{ width, height: TILE_HEIGHT - GAP, transform: `translate(${left}px, ${top}px)` }}
    >
      <button
        type="button"
        className="history-open"
        onClick={onOpen}
        aria-label={`Open generation ${generation.id}`}
      >
        <div className="history-image asset-media-frame">
          {generation.thumbnailPath ? (
            <img
              src={convertFileSrc(generation.thumbnailPath)}
              alt={generation.promptExcerpt}
              loading="lazy"
              decoding="async"
            />
          ) : (
            <span>No preview</span>
          )}
        </div>
        <div className="history-tile-copy media-card-copy">
          <b>
            {subject}
          </b>
          <span>
            {generation.sceneSubjectNames.length ? 'Multi-entity · ' : ''}
            {generation.preset} · {generation.model}
          </span>
          <span>
            seed {generation.seed} · {new Date(generation.createdAt).toLocaleString()}
          </span>
          <p>{generation.promptExcerpt}</p>
        </div>
      </button>
      {onJump && (
        <button className="btn-mini" type="button" onClick={onJump}>
          Open entity
        </button>
      )}
    </article>
  )
}
