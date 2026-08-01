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
  useRecoverSpendLedger,
  useSetSpendCeiling,
  useSetLockedSeed,
  useSpendStatus,
  useStatusBarBackend,
} from '../lib/queries'
import { report, toast } from '../store/ui'
import { useActionShortcut } from '../hooks/useKeyboard'
import { Icon } from './Icon'
import { PromptBox } from './inspector/PromptBox'

/** The live, per-generation influence stack and shot controls. */
export function Inspector({
  project,
  selected,
  kinds: _kinds,
  onJump,
  surface = 'sidebar',
}: {
  project: ProjectSummary
  selected: NodeSummary | null
  kinds: KindIndex
  onJump: (id: string) => void
  surface?: 'sidebar' | 'forge'
}) {
  const presets = usePresets(selected?.kind ?? null)
  const backend = useStatusBarBackend(project.id)
  const [presetId, setPresetId] = useState<string>()
  const [aspect, setAspect] = useState('')
  const [model, setModel] = useState('')
  const [seed, setSeed] = useState(randomSeed)
  const [seedOverride, setSeedOverride] = useState(false)
  const [gridAxis, setGridAxis] = useState<'none' | api.VariantGrid['axis']>('none')
  const [gridValues, setGridValues] = useState('')
  const [gridNodeId, setGridNodeId] = useState('')
  const [weights, setWeights] = useState<Record<string, number>>({})
  const [muted, setMuted] = useState<Set<string>>(new Set())
  const [shotWeight, setShotWeight] = useState(1)
  const [shotMuted, setShotMuted] = useState(false)
  const [shotPrompt, setShotPrompt] = useState('')
  const [generating, setGenerating] = useState(false)
  const [ceilingDollars, setCeilingDollars] = useState('')

  useEffect(() => {
    setPresetId(undefined)
    setWeights({})
    setMuted(new Set())
    setShotWeight(1)
    setShotMuted(false)
    setShotPrompt('')
    setSeed(randomSeed())
    setSeedOverride(false)
    setGridAxis('none')
    setGridValues('')
    setGridNodeId('')
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
  const grid = useMemo(
    () => parseVariantGrid(gridAxis, gridValues, gridNodeId),
    [gridAxis, gridNodeId, gridValues],
  )
  const imageReport = useImageReferenceReport(selected?.id ?? null, {
    ...options,
    aspect,
    model: model.trim() || undefined,
    grid: grid.value,
  })
  const lockedSeed = imageReport.isPlaceholderData ? null : (imageReport.data?.lockedSeed ?? null)
  const usesLockedSeed = lockedSeed !== null && !seedOverride && gridAxis !== 'seed'
  useEffect(() => {
    if (!imageReport.data || imageReport.isPlaceholderData || seedOverride) return
    if (lockedSeed !== null) setSeed(lockedSeed)
    else setSeedOverride(true)
  }, [imageReport.data, imageReport.isPlaceholderData, lockedSeed, seedOverride])
  const paidEstimate = imageReport.data?.cost ?? null
  const spendQuery = useSpendStatus(project.id, paidEstimate !== null)
  const setSpendCeiling = useSetSpendCeiling(project.id)
  const recoverSpendLedger = useRecoverSpendLedger(project.id)
  const setLockedSeed = useSetLockedSeed()
  const spend = spendQuery.data ?? imageReport.data?.spend
  useEffect(() => {
    const ceiling = spend?.ceilingUsdMicros
    setCeilingDollars(ceiling === null || ceiling === undefined ? '' : microsAsInput(ceiling))
  }, [spend?.ceilingUsdMicros])
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
        seed: seedOverride ? seed : undefined,
        grid: grid.value,
      })
      toast(`Generation queued · ${job.slice(-6)}`)
      if (lockedSeed === null && gridAxis === 'none') setSeed(randomSeed())
      void spendQuery.refetch()
    } catch (error) {
      report(error, 'Could not start generation')
    } finally {
      setGenerating(false)
    }
  }

  const changeGridAxis = (axis: typeof gridAxis) => {
    setGridAxis(axis)
    if (axis === 'seed') setGridValues([seed, seed + 1, seed + 2, seed + 3].join(', '))
    if (axis === 'fragment_weight') {
      setGridValues('0.4, 0.7, 1')
      setGridNodeId(
        (current) => current || stack.data?.layers.find((layer) => layer.nodeId)?.nodeId || '',
      )
    }
    if (axis === 'preset') {
      setGridValues(
        (presets.data ?? [])
          .filter((preset) => preset.views.length === 0)
          .slice(0, 4)
          .map((preset) => preset.id)
          .join(', '),
      )
    }
    if (axis === 'aspect') setGridValues('1:1, 3:4, 4:3')
    if (axis === 'none') setGridValues('')
  }

  const reroll = () => {
    setSeed(randomSeed())
    setSeedOverride(true)
  }

  const saveLockedSeed = async (next: number | null) => {
    if (!selected) return
    try {
      await setLockedSeed.mutateAsync({ nodeId: selected.id, seed: next })
      setSeedOverride(next === null)
      if (next !== null) setSeed(next)
      await imageReport.refetch()
      toast(next === null ? 'Seed lock cleared' : 'Seed locked to this entity')
    } catch (error) {
      report(error, next === null ? 'Could not clear the seed lock' : 'Could not lock the seed')
    }
  }

  const saveCeiling = async () => {
    const trimmed = ceilingDollars.trim()
    const dollars = trimmed === '' ? null : Number(trimmed)
    if (dollars !== null && (!Number.isFinite(dollars) || dollars < 0)) {
      report(
        new Error(
          'Enter a non-negative dollar amount, or leave it blank to disable paid generation.',
        ),
      )
      return
    }
    try {
      await setSpendCeiling.mutateAsync(
        dollars === null
          ? null
          : Math.min(Number.MAX_SAFE_INTEGER, Math.round(dollars * 1_000_000)),
      )
      toast(dollars === null ? 'Paid generation disabled' : 'Shared spend ceiling saved')
    } catch (error) {
      report(error, 'Could not save the spend ceiling')
    }
  }

  const recoverReservations = async () => {
    const confirmed = window.confirm(
      'Recover pending spend reservations only after every Wobu window using this project has stopped paid generation. The old ledger will be archived, not deleted. Continue?',
    )
    if (!confirmed) return
    try {
      await recoverSpendLedger.mutateAsync(true)
      toast('Pending spend ledger archived')
    } catch (error) {
      report(error, 'Could not recover the spend ledger')
    }
  }

  const costBlocked =
    paidEstimate !== null &&
    (spend?.ledgerLocked === true ||
      spend?.remainingUsdMicros === null ||
      (spend?.remainingUsdMicros !== undefined &&
        paidEstimate.batchUsdMicros > spend.remainingUsdMicros))
  const gridBlocked = gridAxis !== 'none' && (!grid.value || (chosenPreset?.views.length ?? 0) > 0)
  const generateDisabled =
    !selected ||
    !chosenPreset ||
    generating ||
    project.readOnly ||
    costBlocked ||
    gridBlocked ||
    !imageReport.data ||
    imageReport.isPlaceholderData
  useActionShortcut('generate', !generateDisabled, () => void generate())

  const inspector = (
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
                {bucket.kept}
                {bucket.limit === null ? '' : `/${bucket.limit}`} {bucket.label}
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
              const imageCount = card.fragments.filter(
                (fragment) => fragment.assetId !== null,
              ).length
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
            <input
              value={aspect}
              onChange={(event) => setAspect(event.target.value)}
              placeholder="3:4"
            />
          </label>
          <label className="shot-model">
            <span>Model</span>
            <input
              value={model}
              onChange={(event) => setModel(event.target.value)}
              placeholder="Selected model"
            />
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
          <div className="seed-controls">
            <label>
              <span>Seed</span>
              <input
                type="number"
                min={0}
                max={Number.MAX_SAFE_INTEGER}
                step={1}
                value={seed}
                onChange={(event) => {
                  setSeed(
                    Math.max(0, Math.min(Number.MAX_SAFE_INTEGER, Number(event.target.value) || 0)),
                  )
                  setSeedOverride(true)
                }}
              />
            </label>
            <div className={usesLockedSeed ? 'seed-state is-locked' : 'seed-state'}>
              {lockedSeed === null
                ? 'Unlocked seed'
                : gridAxis === 'seed'
                  ? `Locked at ${lockedSeed} · grid varies seed`
                  : usesLockedSeed
                    ? `Locked · next result uses ${lockedSeed}`
                    : `Locked at ${lockedSeed} · next result is re-rolled`}
            </div>
            <div className="seed-actions">
              <button className="btn-mini" onClick={reroll}>
                Re-roll
              </button>
              {lockedSeed === null ? (
                <button
                  className="btn-mini"
                  disabled={project.readOnly || setLockedSeed.isPending}
                  onClick={() => void saveLockedSeed(seed)}
                >
                  Lock seed
                </button>
              ) : (
                <>
                  {seedOverride && (
                    <button
                      className="btn-mini"
                      onClick={() => {
                        setSeed(lockedSeed)
                        setSeedOverride(false)
                      }}
                    >
                      Use locked
                    </button>
                  )}
                  <button
                    className="btn-mini"
                    disabled={project.readOnly || setLockedSeed.isPending}
                    onClick={() => void saveLockedSeed(null)}
                  >
                    Clear lock
                  </button>
                </>
              )}
            </div>
          </div>
          <div className="variant-controls">
            <label>
              <span>Variant grid</span>
              <select
                value={gridAxis}
                onChange={(event) => changeGridAxis(event.target.value as typeof gridAxis)}
                disabled={chosenPreset?.views.length !== 0}
              >
                <option value="none">Off</option>
                <option value="seed">Vary seed</option>
                <option value="fragment_weight">Vary fragment weight</option>
                <option value="preset">Vary preset</option>
                <option value="aspect">Vary aspect</option>
              </select>
            </label>
            {gridAxis === 'fragment_weight' && (
              <label>
                <span>Layer</span>
                <select value={gridNodeId} onChange={(event) => setGridNodeId(event.target.value)}>
                  {(stack.data?.layers ?? [])
                    .filter((layer) => layer.nodeId)
                    .map((layer) => (
                      <option key={layer.nodeId as string} value={layer.nodeId as string}>
                        {layer.name}
                      </option>
                    ))}
                </select>
              </label>
            )}
            {gridAxis !== 'none' && (
              <label className="variant-values">
                <span>Cell values · comma separated</span>
                <input value={gridValues} onChange={(event) => setGridValues(event.target.value)} />
              </label>
            )}
            {gridAxis !== 'none' && (
              <small className={grid.error ? 'variant-error' : ''}>
                {grid.error ??
                  `${grid.value?.values.length ?? 0} outputs · exactly one axis varies`}
              </small>
            )}
          </div>
          {paidEstimate && spend && (
            <div className="spend-panel" aria-label="Generation cost and project spend ceiling">
              <div className="spend-estimate">
                <b>Estimated {formatUsd(paidEstimate.batchUsdMicros)} batch</b>
                <span>
                  {paidEstimate.images}
                  {paidEstimate.variesByCell
                    ? ' outputs · cell price varies'
                    : ` × ${formatUsd(paidEstimate.perImageUsdMicros)} output`}
                  {paidEstimate.conservativeFallback ? ' · conservative fallback' : ''}
                </span>
              </div>
              <div className="spend-running">
                <span>
                  Receipted {formatUsd(spend.spentUsdMicros)}
                  {spend.reservedUsdMicros > 0
                    ? ` · ${formatUsd(spend.reservedUsdMicros)} pending`
                    : ''}
                </span>
                <span>
                  {spend.ceilingUsdMicros === null
                    ? 'Paid generation disabled'
                    : `${formatUsd(spend.remainingUsdMicros ?? 0)} remaining`}
                </span>
              </div>
              <div className="spend-ceiling">
                <label>
                  <span>Shared ceiling (USD)</span>
                  <input
                    type="number"
                    min={0}
                    step={0.01}
                    value={ceilingDollars}
                    placeholder="Disabled"
                    onChange={(event) => setCeilingDollars(event.target.value)}
                    disabled={project.readOnly || setSpendCeiling.isPending}
                  />
                </label>
                <button
                  className="btn-mini"
                  disabled={project.readOnly || setSpendCeiling.isPending}
                  onClick={() => void saveCeiling()}
                >
                  {setSpendCeiling.isPending ? 'Saving…' : 'Save ceiling'}
                </button>
              </div>
              {(spend.ledgerLocked || spend.pendingReservations > 0) && (
                <div className="spend-recovery">
                  <span>
                    {spend.ledgerLocked
                      ? 'The ledger is locked, possibly after a crash.'
                      : `${spend.pendingReservations} batch reservation${spend.pendingReservations === 1 ? '' : 's'} pending.`}
                  </span>
                  <button
                    className="btn-mini"
                    disabled={project.readOnly || recoverSpendLedger.isPending}
                    onClick={() => void recoverReservations()}
                  >
                    Recover…
                  </button>
                </div>
              )}
              <small title={`Pricing checked ${paidEstimate.checkedAt}`}>
                Indicative output price; input tokens and optional search charges are not included.
              </small>
            </div>
          )}
          <button
            className="btn-primary shot-generate"
            disabled={generateDisabled}
            onClick={() => void generate()}
          >
            <Icon name="image" size="sm" />
            {generating
              ? 'Queueing…'
              : paidEstimate
                ? `Generate · est. ${formatUsd(paidEstimate.batchUsdMicros)}`
                : 'Generate'}
          </button>
        </div>
      </aside>
      <PromptBox project={project} subject={selected} options={options} onJump={onJump} />
    </>
  )
  return surface === 'forge' ? (
    <section className="forge-inspector" aria-label="Forge generation controls">
      {inspector}
    </section>
  ) : (
    inspector
  )
}

function formatUsd(micros: number): string {
  return new Intl.NumberFormat(undefined, {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: 2,
    maximumFractionDigits: 4,
  }).format(micros / 1_000_000)
}

function microsAsInput(micros: number): string {
  return (micros / 1_000_000).toFixed(6).replace(/\.?0+$/, '')
}

function randomSeed(): number {
  return Math.floor(Math.random() * 0xffffffff)
}

type GridParse = { value?: api.VariantGrid; error?: string }

function parseVariantGrid(
  axis: 'none' | api.VariantGrid['axis'],
  source: string,
  nodeId: string,
): GridParse {
  if (axis === 'none') return {}
  const raw = source
    .split(',')
    .map((value) => value.trim())
    .filter(Boolean)
  if (raw.length < 2 || raw.length > 16) return { error: 'Enter 2 to 16 distinct cell values.' }
  if (new Set(raw).size !== raw.length) return { error: 'Every grid cell must be different.' }

  if (axis === 'seed') {
    const values = raw.map(Number)
    if (values.some((value) => !Number.isSafeInteger(value) || value < 0)) {
      return { error: 'Seeds must be non-negative whole numbers.' }
    }
    return { value: { axis, values } }
  }
  if (axis === 'fragment_weight') {
    const values = raw.map(Number)
    if (!nodeId) return { error: 'Choose the influence layer whose weight should vary.' }
    if (values.some((value) => !Number.isFinite(value) || value < 0 || value > 1)) {
      return { error: 'Fragment weights must be numbers from 0 to 1.' }
    }
    return { value: { axis, nodeId, values } }
  }
  if (axis === 'preset') return { value: { axis, values: raw } }
  return { value: { axis, values: raw } }
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
    <details
      className={muted ? 'layer is-muted' : 'layer'}
      style={{ '--lc': color } as CSSProperties}
    >
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
            <FragmentRow
              key={`${fragment.section}-${fragment.assetId ?? index}`}
              fragment={fragment}
            />
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
