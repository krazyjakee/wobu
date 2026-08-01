import { useEffect, useMemo, useState } from 'react'
import type { CSSProperties } from 'react'
import * as api from '../lib/api'
import type {
  InfluenceFragment,
  LayerCard,
  NodeSummary,
  ProjectSummary,
  SliderSetting,
} from '../lib/api'
import { layerColor, layerLabel, type KindIndex } from '../lib/kinds'
import {
  useCompiledPrompt,
  useImageReferenceReport,
  useInfluenceStack,
  usePresets,
  useStatusBarBackend,
} from '../lib/queries'
import { report, toast } from '../store/ui'
import { Icon } from './Icon'
import { PromptBox } from './inspector/PromptBox'

/** The live, per-generation influence stack and shot controls. */
export function Inspector({
  project,
  selected,
  kinds: _kinds,
  onJump,
}: {
  project: ProjectSummary
  selected: NodeSummary | null
  kinds: KindIndex
  onJump: (id: string) => void
}) {
  const presets = usePresets(selected?.kind ?? null)
  const backend = useStatusBarBackend(project.id)
  const [presetId, setPresetId] = useState<string>()
  const [aspect, setAspect] = useState('')
  const [model, setModel] = useState('')
  const [seed, setSeed] = useState(() => Math.floor(Math.random() * 0xffffffff))
  const [weights, setWeights] = useState<Record<string, number>>({})
  const [muted, setMuted] = useState<Set<string>>(new Set())
  const [shotWeight, setShotWeight] = useState(1)
  const [shotMuted, setShotMuted] = useState(false)
  const [shotPrompt, setShotPrompt] = useState('')
  const [generating, setGenerating] = useState(false)

  useEffect(() => {
    setPresetId(undefined)
    setWeights({})
    setMuted(new Set())
    setShotWeight(1)
    setShotMuted(false)
    setShotPrompt('')
  }, [selected?.id])

  const chosenPreset =
    presets.data?.find((preset) => preset.id === presetId) ??
    presets.data?.find((preset) => selected && preset.defaultFor.includes(selected.kind)) ??
    presets.data?.[0]
  useEffect(() => {
    if (!chosenPreset) return
    setPresetId(chosenPreset.id)
    setAspect(chosenPreset.aspect)
  }, [chosenPreset?.id])
  useEffect(() => {
    if (backend.data?.image?.model) setModel(backend.data.image.model)
  }, [backend.data?.image?.model])

  const sliders = useMemo<SliderSetting[]>(
    () =>
      Object.entries(weights).map(([nodeId, value]) => ({
        nodeId,
        value,
        muted: muted.has(nodeId),
      })),
    [muted, weights],
  )
  const options = useMemo(
    () => ({
      preset: chosenPreset?.id,
      sliders,
      shot: {
        label: chosenPreset?.label,
        weight: shotMuted ? 0 : shotWeight,
        prompt: shotPrompt.trim() || undefined,
      },
    }),
    [chosenPreset?.id, chosenPreset?.label, shotMuted, shotPrompt, shotWeight, sliders],
  )
  const stack = useInfluenceStack(selected?.id ?? null, options)
  const compiled = useCompiledPrompt(selected?.id ?? null, options)
  const imageReport = useImageReferenceReport(selected?.id ?? null, {
    ...options,
    model: model.trim() || undefined,
  })
  const dropped = useMemo(() => {
    const count = new Map<string, number>()
    for (const item of compiled.data?.dropped ?? []) {
      const key = `${item.fragment.nodeId ?? 'shot'}:${item.fragment.layer}`
      count.set(key, (count.get(key) ?? 0) + 1)
    }
    return count
  }, [compiled.data?.dropped])

  const setLayerWeight = (card: LayerCard, value: number) => {
    if (card.nodeId) setWeights((current) => ({ ...current, [card.nodeId as string]: value }))
    else setShotWeight(value)
  }
  const toggleMute = (card: LayerCard) => {
    if (!card.nodeId) {
      setShotMuted((value) => !value)
      return
    }
    setWeights((current) =>
      card.nodeId && current[card.nodeId] === undefined
        ? { ...current, [card.nodeId]: card.slider }
        : current,
    )
    setMuted((current) => {
      const next = new Set(current)
      if (next.has(card.nodeId as string)) next.delete(card.nodeId as string)
      else next.add(card.nodeId as string)
      return next
    })
  }

  const generate = async () => {
    if (!selected || !chosenPreset) return
    setGenerating(true)
    try {
      const job = await api.generateStart(selected.id, {
        preset: chosenPreset.id,
        sliders,
        shot: options.shot,
        aspect,
        model: model.trim() || undefined,
        seed,
      })
      toast(`Generation queued · ${job.slice(-6)}`)
      setSeed(Math.floor(Math.random() * 0xffffffff))
    } catch (error) {
      report(error, 'Could not start generation')
    } finally {
      setGenerating(false)
    }
  }

  return (
    <>
      <aside className="insp">
        <div className="insp-head">
          <h2>Influence stack</h2>
          <span className="hint">this generation only</span>
        </div>

        {imageReport.data && (
          <div className="ref-budgets" aria-label="Provider reference limits">
            {imageReport.data.buckets.map((bucket) => (
              <span key={bucket.bucket}>
                {bucket.kept}{bucket.limit === null ? '' : `/${bucket.limit}`} {bucket.label}
                {bucket.dropped > 0 ? ` · ${bucket.dropped} dropped` : ''}
              </span>
            ))}
          </div>
        )}

        <div className="stack">
          {!selected ? (
            <div className="insp-empty">
              <b>No node selected.</b>
              <span>The stack is resolved per node, outermost layer first.</span>
            </div>
          ) : stack.isError ? (
            <div className="insp-empty">
              <b>The stack could not be resolved.</b>
              <span>{api.errorMessage(stack.error)}</span>
            </div>
          ) : !stack.data ? (
            <p className="stack-wait">Resolving influences…</p>
          ) : (
            stack.data.layers.map((card, index) => {
              const key = card.nodeId ?? `shot-${index}`
              const isMuted = card.nodeId ? muted.has(card.nodeId) : shotMuted
              const value = card.nodeId ? (weights[card.nodeId] ?? card.slider) : shotWeight
              const textCount = card.fragments.filter((fragment) => fragment.text !== null).length
              const imageCount = card.fragments.filter((fragment) => fragment.assetId !== null).length
              const dropCount = dropped.get(`${card.nodeId ?? 'shot'}:${card.layer}`) ?? 0
              const referenceReport = imageReport.data?.layers.find(
                (report) => report.nodeId === card.nodeId && report.layer === card.layer,
              )
              return (
                <Layer
                  key={key}
                  card={card}
                  value={value}
                  muted={isMuted}
                  textCount={textCount}
                  imageCount={imageCount}
                  dropCount={dropCount + (referenceReport?.dropped ?? 0)}
                  referenceKept={referenceReport?.kept ?? 0}
                  dropReasons={referenceReport?.reasons ?? []}
                  onWeight={(next) => setLayerWeight(card, next)}
                  onMute={() => toggleMute(card)}
                  onJump={onJump}
                />
              )
            })
          )}
        </div>

        <div className="shot-controls" aria-label="Shot controls">
          <label>
            <span>Output preset</span>
            <select
              value={chosenPreset?.id ?? ''}
              onChange={(event) => setPresetId(event.target.value)}
              disabled={!selected || !presets.data?.length}
            >
              {(presets.data ?? []).map((preset) => (
                <option key={preset.id} value={preset.id}>
                  {preset.label} · {preset.images} image{preset.images === 1 ? '' : 's'}
                </option>
              ))}
            </select>
          </label>
          <label>
            <span>Aspect</span>
            <input value={aspect} onChange={(event) => setAspect(event.target.value)} placeholder="3:4" />
          </label>
          <label className="shot-model">
            <span>Model</span>
            <input value={model} onChange={(event) => setModel(event.target.value)} placeholder="Selected model" />
          </label>
          <label className="shot-prompt">
            <span>Extra shot prompt</span>
            <textarea
              rows={2}
              value={shotPrompt}
              onChange={(event) => setShotPrompt(event.target.value)}
              placeholder="Optional framing, action, weather, or camera direction"
            />
          </label>
          <label>
            <span>Seed</span>
            <input
              type="number"
              min={0}
              step={1}
              value={seed}
              onChange={(event) => setSeed(Math.max(0, Number(event.target.value) || 0))}
            />
          </label>
          <button
            className="btn-primary shot-generate"
            disabled={!selected || !chosenPreset || generating || project.readOnly}
            onClick={() => void generate()}
          >
            <Icon name="image" size="sm" />
            {generating ? 'Queueing…' : 'Generate'}
          </button>
        </div>
      </aside>
      <PromptBox project={project} subject={selected} options={options} onJump={onJump} />
    </>
  )
}

function Layer({
  card,
  value,
  muted,
  textCount,
  imageCount,
  dropCount,
  referenceKept,
  dropReasons,
  onWeight,
  onMute,
  onJump,
}: {
  card: LayerCard
  value: number
  muted: boolean
  textCount: number
  imageCount: number
  dropCount: number
  referenceKept: number
  dropReasons: string[]
  onWeight: (value: number) => void
  onMute: () => void
  onJump: (id: string) => void
}) {
  const color = layerColor(card.layer)
  return (
    <details className={muted ? 'layer is-muted' : 'layer'} style={{ '--lc': color } as CSSProperties}>
      <summary className="layer-h">
        <span className="layer-dot" aria-hidden="true" />
        <span className="txt">
          <span className="layer-l">{layerLabel(card.layer)}</span>
          <span className="layer-n">{card.name}</span>
          <span className="layer-count">
            {textCount} text · {imageCount} image{imageCount === 1 ? '' : 's'}
            {imageCount > 0 ? ` · ${referenceKept} sent` : ''}
            {dropCount > 0 ? ` · ${dropCount} dropped` : ''}
          </span>
        </span>
      </summary>
      <div className="layer-controls">
        <label>
          <span>Weight {muted ? 'muted' : value.toFixed(2)}</span>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={value}
            disabled={muted}
            onChange={(event) => onWeight(Number(event.target.value))}
          />
        </label>
        <button className={muted ? 'btn-mini is-on' : 'btn-mini'} onClick={onMute}>
          {muted ? 'Unmute' : 'Mute'}
        </button>
      </div>
      <div className="layer-fragments">
        {dropReasons.map((reason, index) => (
          <p className="layer-drop" key={`${reason}-${index}`}>
            Dropped: {reason}
          </p>
        ))}
        {card.fragments.length === 0 ? (
          <p>Nothing described on this source yet.</p>
        ) : (
          card.fragments.map((fragment, index) => (
            <FragmentRow key={`${fragment.section}-${fragment.assetId ?? index}`} fragment={fragment} />
          ))
        )}
        {card.nodeId && (
          <button className="layer-source" onClick={() => onJump(card.nodeId as string)}>
            Open source
          </button>
        )}
      </div>
    </details>
  )
}

function FragmentRow({ fragment }: { fragment: InfluenceFragment }) {
  return (
    <div className="layer-fragment">
      <b>{fragment.section.replaceAll('_', ' ')}</b>
      {fragment.text !== null ? (
        <span>{fragment.text}</span>
      ) : (
        <span>
          Image {fragment.assetId} · {fragment.target.replaceAll('_', ' ')}
          {!fragment.sendable ? ' · private' : ''}
        </span>
      )}
      <small>weight {fragment.weight.toFixed(2)}</small>
    </div>
  )
}
