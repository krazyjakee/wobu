import { useMemo, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import * as api from '../lib/api'
import type { Generation, NodeSummary } from '../lib/api'
import { useGenerationHistory } from '../lib/queries'
import { report, useUI } from '../store/ui'
import { LazyAssetThumbnail } from './AssetMedia'
import { GenerationDetail } from './GenerationDetail'
import { GenerationPresetModel, GenerationSubject, GenerationTimestamp } from './GenerationMetadata'

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
  const history = useGenerationHistory()
  const [preset, setPreset] = useState('all')
  const [model, setModel] = useState('all')
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')
  const [seed, setSeed] = useState('')
  const [opened, setOpened] = useState<{ generation: Generation; imageSrc: string | null } | null>(
    null,
  )
  const names = useMemo(() => new Map(nodes.map((node) => [node.id, node.name])), [nodes])
  const receipts = history.data ?? []
  const presets = useMemo(
    () => [...new Set(receipts.map((item) => item.preset))].sort(),
    [receipts],
  )
  const models = useMemo(() => [...new Set(receipts.map((item) => item.model))].sort(), [receipts])
  const filtered = useMemo(
    () =>
      receipts.filter((item) => {
        if (preset !== 'all' && item.preset !== preset) return false
        if (model !== 'all' && item.model !== model) return false
        const day = item.createdAt.slice(0, 10)
        if (from && day < from) return false
        if (to && day > to) return false
        if (seed.trim() && String(item.seed) !== seed.trim()) return false
        return true
      }),
    [from, model, preset, receipts, seed, to],
  )

  async function open(generation: Generation) {
    const assetId = generation.outputAssetIds[0] ?? null
    if (!assetId) {
      setOpened({ generation, imageSrc: null })
      return
    }
    try {
      const path = await api.assetOriginal(assetId)
      setOpened({ generation, imageSrc: path ? convertFileSrc(path) : null })
    } catch (error) {
      report(error, 'Could not open the recorded image')
      setOpened({ generation, imageSrc: null })
    }
  }

  return (
    <main className="history-mode" aria-label="Generation history">
      <header className="history-mode-head">
        <div>
          <h2>Generation history</h2>
          <p>{receipts.length} immutable receipts across this project</p>
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
            {presets.map((value) => (
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
            {models.map((value) => (
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
      {!history.isPending && !history.isError && filtered.length === 0 && (
        <p className="history-empty empty-state">No generations match these filters.</p>
      )}
      <div className="history-grid">
        {filtered.map((generation) => (
          <HistoryTile
            key={generation.id}
            generation={generation}
            nodeName={names.get(generation.nodeId) ?? 'Deleted entity'}
            onOpen={() => void open(generation)}
            onJump={names.has(generation.nodeId) ? () => onJump(generation.nodeId) : null}
          />
        ))}
      </div>

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

function HistoryTile({
  generation,
  nodeName,
  onOpen,
  onJump,
}: {
  generation: Generation
  nodeName: string
  onOpen: () => void
  onJump: (() => void) | null
}) {
  const assetId = generation.outputAssetIds[0] ?? null
  return (
    <article className="history-tile media-card">
      <button
        type="button"
        className="history-open"
        onClick={onOpen}
        aria-label={`Open generation ${generation.id}`}
      >
        <div className="history-image asset-media-frame">
          <LazyAssetThumbnail
            assetId={assetId}
            alt={generation.compiledPrompt}
            loadingLabel="No preview"
            missingLabel="No preview"
            errorLabel="No preview"
          />
        </div>
        <div className="history-tile-copy media-card-copy">
          <b>
            <GenerationSubject generation={generation} fallback={nodeName} />
          </b>
          <span>
            <GenerationPresetModel generation={generation} />
          </span>
          <span>
            seed {generation.seed} · <GenerationTimestamp generation={generation} />
          </span>
          <p>{generation.compiledPrompt}</p>
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
