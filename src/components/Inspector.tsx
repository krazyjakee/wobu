import { useMemo, useState } from 'react'
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
import { negotiatedAspect } from '../lib/generationCapabilities'
import { useNodeThumbs } from '../lib/nodeThumbs'
import {
  useCompiledPrompt,
  useImageGenerationCapabilities,
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
import { useDebounced } from '../hooks/useDebounced'
import { NodeThumbnail } from './AssetMedia'
import { Combobox } from './Combobox'
import { GenerationAspectSelect } from './GenerationAspectSelect'
import { Icon } from './Icon'
import { TipButton, Tooltip } from './Tooltip'
import { PromptBox } from './inspector/PromptBox'

/** The one axis a batch may vary, in the order the panel reads: off, outwards. */
const GRID_AXES = [
  { value: 'none', label: 'Off' },
  { value: 'seed', label: 'Vary seed' },
  { value: 'fragment_weight', label: 'Vary fragment weight' },
  { value: 'preset', label: 'Vary preset' },
  { value: 'aspect', label: 'Vary aspect' },
]

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
  return (
    <InspectorSession
      key={`${project.id}:${selected?.id ?? 'none'}`}
      project={project}
      selected={selected}
      kinds={_kinds}
      onJump={onJump}
      surface={surface}
    />
  )
}

function InspectorSession({
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
  const [aspectDraft, setAspect] = useState<string>()
  const backendModel = backend.data?.image?.model ?? ''
  const backendSource = `${backend.data?.image?.provider ?? ''}:${backendModel}`
  const [modelDraft, setModelDraft] = useState<{ source: string; value: string } | null>(null)
  const model = modelDraft?.source === backendSource ? modelDraft.value : backendModel
  const [seedDraft, setSeed] = useState(randomSeed)
  const [seedOverrideDraft, setSeedOverride] = useState(false)
  const [gridAxis, setGridAxis] = useState<'none' | api.VariantGrid['axis']>('none')
  const [gridValues, setGridValues] = useState('')
  const [gridNodeId, setGridNodeId] = useState('')
  const [weights, setWeights] = useState<Record<string, number>>({})
  const [muted, setMuted] = useState<Set<string>>(new Set())
  const [shotWeight, setShotWeight] = useState(1)
  const [shotMuted, setShotMuted] = useState(false)
  const [shotPrompt, setShotPrompt] = useState('')
  const [generating, setGenerating] = useState(false)
  const [ceilingDraft, setCeilingDraft] = useState<{
    source: number | null | undefined
    value: string
  } | null>(null)

  const chosenPreset =
    presets.data?.find((preset) => preset.id === presetId) ??
    presets.data?.find((preset) => selected && preset.defaultFor.includes(selected.kind)) ??
    presets.data?.[0]
  const chosenPresetId = chosenPreset?.id
  const aspect = aspectDraft ?? chosenPreset?.aspect ?? ''
  const debouncedModel = useDebounced(model.trim(), 350)
  const capabilityModel =
    model.trim() === backend.data?.image?.model ? model.trim() : debouncedModel
  const generationCapabilities = useImageGenerationCapabilities(
    project.id,
    capabilityModel || undefined,
    !!backend.data?.image && !!capabilityModel,
  )
  const aspectChoices = useMemo(
    () => generationCapabilities.data?.aspectRatios ?? [],
    [generationCapabilities.data?.aspectRatios],
  )
  const aspectNegotiation = useMemo(
    () => negotiatedAspect(generationCapabilities.data, aspect),
    [aspect, generationCapabilities.data],
  )
  const aspectAdjustment =
    aspectNegotiation &&
    chosenPresetId &&
    aspect &&
    !generationCapabilities.isPlaceholderData &&
    aspectNegotiation.actualAspect !== aspect
      ? {
          requested: aspect,
          actual: aspectNegotiation.actualAspect,
          valid: aspectNegotiation.requestedValid,
        }
      : null
  const normalizedAspect = aspectAdjustment?.actual ?? aspect

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
    () => parseVariantGrid(gridAxis, gridValues, gridNodeId, aspectChoices),
    [aspectChoices, gridAxis, gridNodeId, gridValues],
  )
  const imageReport = useImageReferenceReport(selected?.id ?? null, {
    ...options,
    aspect: normalizedAspect,
    model: model.trim() || undefined,
    grid: grid.value,
  })
  const lockedSeed = imageReport.isPlaceholderData ? null : (imageReport.data?.lockedSeed ?? null)
  const reportReady = !!imageReport.data && !imageReport.isPlaceholderData
  const seedOverride = seedOverrideDraft || (reportReady && lockedSeed === null)
  const seed = lockedSeed !== null && !seedOverrideDraft ? lockedSeed : seedDraft
  const usesLockedSeed = lockedSeed !== null && !seedOverride && gridAxis !== 'seed'
  const paidEstimate = imageReport.data?.cost ?? null
  const spendQuery = useSpendStatus(project.id, paidEstimate !== null)
  const setSpendCeiling = useSetSpendCeiling(project.id)
  const recoverSpendLedger = useRecoverSpendLedger(project.id)
  const setLockedSeed = useSetLockedSeed()
  const spend = spendQuery.data
  const ceilingSource = spend?.ceilingUsdMicros
  const ceilingDollars =
    ceilingDraft && ceilingDraft.source === ceilingSource
      ? ceilingDraft.value
      : ceilingSource === null || ceilingSource === undefined
        ? ''
        : microsAsInput(ceilingSource)
  /*
   * The stack is short — a subject and the handful of things it inherits from —
   * and it is resolved as one document, so the whole of it goes in one call.
   *
   * The shot layer has no `nodeId` and so no picture; it is left out of the
   * request rather than asked about, because "which entity is this" is the only
   * question this batch can answer.
   */
  const stackThumbs = useNodeThumbs(
    useMemo(
      () =>
        (stack.data?.layers ?? [])
          .map((layer) => layer.nodeId)
          .filter((nodeId): nodeId is string => !!nodeId),
      [stack.data?.layers],
    ),
  )
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
        aspect: normalizedAspect,
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
    if (axis === 'aspect') setGridValues(aspectChoices.slice(0, 3).join(', '))
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
    (!spend ||
      spend.ledgerLocked === true ||
      spend.remainingUsdMicros === null ||
      paidEstimate.batchUsdMicros > spend.remainingUsdMicros)
  const gridBlocked = gridAxis !== 'none' && (!grid.value || (chosenPreset?.views.length ?? 0) > 0)

  /*
   * `sort="none"` throughout this panel, and deliberately.
   *
   * Presets arrive from the backend already ranked, the grid axes read as a
   * sequence from "off" outwards, and the influence layers are in the order
   * they are applied to the prompt. Alphabetising any of the three would put
   * the list in an order that contradicts what the panel above it is showing.
   */
  const presetOptions = useMemo(
    () =>
      (presets.data ?? []).map((preset) => ({
        value: preset.id,
        label: `${preset.label} · ${preset.images} image${preset.images === 1 ? '' : 's'}`,
      })),
    [presets.data],
  )
  const layerOptions = useMemo(
    () =>
      (stack.data?.layers ?? [])
        .filter((layer) => layer.nodeId)
        .map((layer) => ({ value: layer.nodeId as string, label: layer.name })),
    [stack.data?.layers],
  )
  const aspectReady =
    !!generationCapabilities.data &&
    capabilityModel === model.trim() &&
    !generationCapabilities.isFetching &&
    !generationCapabilities.isPlaceholderData &&
    !!aspectNegotiation &&
    aspectChoices.includes(normalizedAspect)
  const generateDisabled =
    !selected ||
    !chosenPreset ||
    generating ||
    project.readOnly ||
    costBlocked ||
    gridBlocked ||
    !aspectReady ||
    !imageReport.data ||
    imageReport.isPlaceholderData
  useActionShortcut('generate', !generateDisabled, () => void generate())

  /*
   * Why Generate is refused, in the order the user can act on.
   *
   * Generate is the primary action of the whole application and it has eight
   * separate preconditions, any of which greys it out identically. A user who
   * has met one of the middle three — a spend cap, a grid axis that clashes
   * with the preset's views, an aspect the provider has not confirmed — cannot
   * work out which from looking, because looking is all `disabled` allows.
   */
  const generateReason = !generateDisabled
    ? null
    : !selected
      ? 'Select an entity first — a generation is a picture of something.'
      : project.readOnly
        ? 'This project is open read-only, and a generation writes its results into it.'
        : generating
          ? 'This batch is already being queued.'
          : !chosenPreset
            ? 'Choose a shot preset.'
            : costBlocked
              ? 'This batch would go past the spending cap left on this project. Raise the cap in Settings, or generate fewer images.'
              : gridBlocked
                ? 'A grid axis and a preset that already fixes its views cannot both decide the frames. Set the axis back to none, or pick a preset without views.'
                : 'Waiting for the image backend to say what it accepts. Check it is connected in Settings.'

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
                  thumb={card.nodeId ? stackThumbs.get(card.nodeId) : null}
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
            <Combobox
              label="Output preset"
              value={chosenPreset?.id ?? ''}
              options={presetOptions}
              placeholder={presets.data?.length ? 'Choose a preset' : 'No presets'}
              onChange={(next) => {
                setPresetId(next)
                setAspect(presets.data?.find((preset) => preset.id === next)?.aspect)
              }}
              disabled={!selected || !presets.data?.length}
            />
          </label>
          <label>
            <span>Aspect</span>
            <GenerationAspectSelect
              label="Aspect"
              value={normalizedAspect}
              choices={aspectChoices}
              onChange={setAspect}
            />
          </label>
          {generationCapabilities.data &&
            aspectNegotiation &&
            !generationCapabilities.isPlaceholderData && (
              <div className="aspect-preview" role="status">
                <span>
                  Actual output · {aspectNegotiation.actualAspect} · {aspectNegotiation.width}×
                  {aspectNegotiation.height}px
                </span>
                {generationCapabilities.data.flexibleAspect && (
                  <small>Flexible backend · using Wobu's curated, validated aspect choices.</small>
                )}
              </div>
            )}
          {aspectAdjustment && (
            <div className="aspect-preview is-substituted" role="status">
              <span>
                {aspectAdjustment.valid ? 'Unsupported saved aspect' : 'Malformed saved aspect'}{' '}
                {aspectAdjustment.requested} was replaced with {aspectAdjustment.actual}.
              </span>
              {aspectNegotiation && (
                <small>
                  Confirmed output · {aspectNegotiation.width}×{aspectNegotiation.height}px
                </small>
              )}
            </div>
          )}
          <label className="shot-model">
            <span>Model</span>
            <input
              value={model}
              onChange={(event) =>
                setModelDraft({ source: backendSource, value: event.target.value })
              }
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
              <Combobox
                label="Variant grid"
                value={gridAxis}
                options={GRID_AXES}
                onChange={(next) => changeGridAxis(next as typeof gridAxis)}
                disabled={chosenPreset?.views.length !== 0}
              />
            </label>
            {gridAxis === 'fragment_weight' && (
              <label>
                <span>Layer</span>
                <Combobox
                  label="Layer"
                  value={gridNodeId}
                  options={layerOptions}
                  placeholder="Choose a layer"
                  onChange={setGridNodeId}
                />
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
                    onChange={(event) =>
                      setCeilingDraft({ source: ceilingSource, value: event.target.value })
                    }
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
              <Tooltip tip={`Pricing checked ${paidEstimate.checkedAt}`}>
                <small tabIndex={0}>
                  Indicative output price; input tokens and optional search charges are not
                  included.
                </small>
              </Tooltip>
            </div>
          )}
          <TipButton
            className="btn-primary shot-generate"
            disabledReason={generateReason}
            tip="Queue this batch against the connected image backend"
            placement="top"
            onClick={() => void generate()}
          >
            <Icon name="image" size="sm" />
            {generating
              ? 'Queueing…'
              : paidEstimate
                ? `Generate · est. ${formatUsd(paidEstimate.batchUsdMicros)}`
                : 'Generate'}
          </TipButton>
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
  aspectChoices: string[] = [],
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
  const unsupported = raw.find((value) => !aspectChoices.includes(value))
  if (unsupported) {
    return {
      error: aspectChoices.length
        ? `${unsupported} is not supported by the selected image backend.`
        : 'Aspect capabilities are not available yet.',
    }
  }
  return { value: { axis, values: raw } }
}

function Layer({
  card,
  thumb,
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
  /** Resolved above for the whole stack; `null` for the shot and for text-only sources. */
  thumb: string | null
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
        {/* The dot is the fallback rather than a second marker: a layer that has
            a picture says which entity it is with the picture, and one that has
            none keeps the coloured dot in the same box, so the rows below a
            resolved thumbnail never move. */}
        <NodeThumbnail path={thumb} fallback={<span className="layer-dot" aria-hidden="true" />} />
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
            aria-valuetext={muted ? 'Muted' : `${Math.round(value * 100)} percent`}
            onChange={(event) => onWeight(Number(event.target.value))}
          />
        </label>
        <button
          className={muted ? 'btn-mini is-on' : 'btn-mini'}
          aria-pressed={muted}
          onClick={onMute}
        >
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
