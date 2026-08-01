import type {
  Generation,
  GenerationSnapshotFragment,
  ShotControls,
  SliderSetting,
} from '../lib/api'
import { generationLoraReceipt, sceneComposition } from '../lib/api'
import { useCompiledPrompt, useInfluenceStack, useReplayGeneration } from '../lib/queries'
import { generationDrift } from '../lib/generationDiff'
import { Modal } from './Modal'

export function GenerationDetail({
  generation,
  nodeName,
  imageSrc,
  readOnly,
  onClose,
}: {
  generation: Generation
  nodeName: string
  imageSrc: string | null
  readOnly: boolean
  onClose: () => void
}) {
  const scene = sceneComposition(generation)
  const shotName = generation.influenceSnapshot.layers.find(
    (layer) => layer.layer === 'shot',
  )?.nodeName
  const controls = recordedControls(generation)
  const options = {
    preset: generation.preset,
    sliders: controls?.sliders,
    shot: controls?.shot ?? { label: shotName, prompt: generation.userPrompt },
  }
  const currentStack = useInfluenceStack(scene ? null : generation.nodeId, options)
  const currentPrompt = useCompiledPrompt(scene ? null : generation.nodeId, options)
  const replay = useReplayGeneration()
  const drift = generationDrift(generation, currentStack.data ?? null, currentPrompt.data ?? null)
  const originalCost = numberParam(generation, 'estimatedCostUsdMicros')
  const replaySourceCost = numberParam(generation, 'replayOriginalEstimatedCostUsdMicros')
  const loras = generationLoraReceipt(generation)

  return (
    <Modal
      className="generation-detail"
      scrimClassName="generation-detail-scrim"
      titleId="generation-detail-title"
      descriptionId="generation-detail-description"
      onClose={onClose}
    >
      <header className="generation-detail-head">
        <div>
          <h2 id="generation-detail-title">Generation details</h2>
          <p id="generation-detail-description">
            {scene ? `Scene · ${scene.subjectNames.join(' + ')}` : nodeName} · {generation.preset} ·{' '}
            {generation.backend} / {generation.model} · seed {generation.seed} ·{' '}
            {new Date(generation.createdAt).toLocaleString()}
          </p>
        </div>
        <div className="generation-detail-actions">
          <button
            className="btn"
            type="button"
            disabled={readOnly || replay.isPending}
            onClick={() => replay.mutate(generation.id)}
          >
            {replay.isPending ? 'Queuing…' : 'Replay snapshot'}
          </button>
          <button
            className="ibtn"
            type="button"
            onClick={onClose}
            aria-label="Close generation details"
            data-modal-initial-focus
          >
            ×
          </button>
        </div>
      </header>

      <div className="generation-detail-body">
        <section className="generation-receipt" aria-label="Recorded request">
          {imageSrc && <img src={imageSrc} alt={generation.compiledPrompt} />}
          <dl>
            <dt>User shot</dt>
            <dd>{generation.userPrompt || 'None'}</dd>
            {scene && (
              <>
                <dt>Participants</dt>
                <dd>{scene.subjectNames.join(' · ')}</dd>
              </>
            )}
            <dt>Compiled prompt</dt>
            <dd>
              <pre>{generation.compiledPrompt}</pre>
            </dd>
            <dt>Negative prompt</dt>
            <dd>
              <pre>{generation.negativePrompt || 'None'}</pre>
            </dd>
            <dt>Request</dt>
            <dd>
              {stringParam(generation, 'aspect') ?? 'unknown aspect'} · {sizeLabel(generation)} ·
              seed {generation.seed}
            </dd>
            <dt>Receipt</dt>
            <dd>
              {generation.id}
              {typeof generation.params.replayOf === 'string' && (
                <> · replay of {generation.params.replayOf}</>
              )}
            </dd>
            {replaySourceCost !== null && (
              <>
                <dt>Source estimate</dt>
                <dd>{usd(replaySourceCost)}</dd>
              </>
            )}
            {originalCost !== null && (
              <>
                <dt>{replaySourceCost === null ? 'Original estimate' : 'Replay estimate'}</dt>
                <dd>{usd(originalCost)}</dd>
              </>
            )}
          </dl>
          <p className="generation-replay-note">
            Replay resubmits this recorded request without reading today’s stack. Paid requests are
            reserved at the current model price, separately from the original estimate.
          </p>
          {(loras.applied.length > 0 || loras.downgrades.length > 0) && (
            <section className="generation-lora-receipt" aria-label="Recorded LoRA application">
              <h3>Entity LoRAs</h3>
              {loras.applied.length > 0 && (
                <div>
                  <b>Applied</b>
                  <ul>
                    {loras.applied.map((lora, index) => (
                      <li key={`${lora.nodeId}:${lora.contentHash}:${index}`}>
                        <strong>{lora.providerName}</strong>
                        <span>
                          {hashPrefix(lora.contentHash)} · strength {weight(lora.strength)} · node{' '}
                          {lora.nodeId}
                        </span>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              {loras.downgrades.length > 0 && (
                <div>
                  <b>Not applied</b>
                  <ul>
                    {loras.downgrades.map((lora, index) => (
                      <li key={`${lora.nodeId}:${lora.contentHash}:${lora.state}:${index}`}>
                        <strong>{lora.state.replaceAll('_', ' ')}</strong>
                        <span>{lora.detail}</span>
                        <small>
                          {hashPrefix(lora.contentHash)} · node {lora.nodeId}
                        </small>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </section>
          )}
        </section>

        <section className="generation-snapshot" aria-label="Recorded influence snapshot">
          <h3>Exact recorded stack</h3>
          {generation.influenceSnapshot.layers.map((layer) => (
            <article className="generation-layer" key={`${layer.layer}:${layer.nodeId ?? 'shot'}`}>
              <header>
                <b>{layer.nodeName}</b>
                <span>
                  {layer.layer} · weight {weight(layer.weight)}
                  {layer.muted ? ' · muted' : ''}
                </span>
              </header>
              <ul>
                {layer.fragments.map((fragment, index) => (
                  <li key={`${fragment.section}:${fragment.assetId ?? fragment.text}:${index}`}>
                    <span>
                      {fragment.section} · {weight(fragment.weight)} · {fragment.target}
                      {fragment.dropped ? ' · dropped' : ''}
                    </span>
                    <p>{fragmentLabel(fragment)}</p>
                  </li>
                ))}
              </ul>
            </article>
          ))}
        </section>

        <section className="generation-drift" aria-label="Stack drift">
          <h3>
            {scene ? 'Scene drift' : controls ? 'Drift from today' : 'Today with default controls'}
          </h3>
          {scene && (
            <p className="generation-drift-basis">
              This receipt preserves a merged multi-entity stack. Single-entity drift is not
              compared against only its primary participant.
            </p>
          )}
          {!scene && (
            <>
              <p className="generation-drift-basis">
                {controls
                  ? 'Today’s world is recompiled with the recorded sliders and shot controls.'
                  : 'This legacy receipt predates stored controls. Weight differences may be generation controls, not world edits.'}
              </p>
              {(currentStack.isPending || currentPrompt.isPending) && (
                <p>Resolving today’s stack…</p>
              )}
              {(currentStack.isError || currentPrompt.isError) && (
                <p>Today’s stack is unavailable. The recorded snapshot above remains complete.</p>
              )}
              {drift && (
                <>
                  <p
                    className={
                      drift.promptChanged || drift.negativeChanged ? 'is-drifted' : 'is-same'
                    }
                  >
                    {drift.promptChanged || drift.negativeChanged
                      ? `Prompt drifted${drift.negativeChanged ? ' (including negative prompt)' : ''}.`
                      : 'Comparable compiled prompts are unchanged.'}
                  </p>
                  {!drift.negativeComparable && (
                    <p className="generation-drift-basis">
                      Negative-prompt drift is not claimed: this receipt does not record a
                      comparable provider capability.
                    </p>
                  )}
                  <ul>
                    {drift.layers.map((layer) => (
                      <li className={`is-${layer.status}`} key={layer.key}>
                        <b>{layer.historical?.nodeName ?? layer.current?.name}</b>
                        <span>
                          {layer.status}
                          {layer.changes.length ? ` · ${layer.changes.join(', ')}` : ''}
                        </span>
                        {layer.historical && layer.current && layer.status === 'changed' && (
                          <small>
                            recorded {weight(layer.historical.weight)} → current{' '}
                            {weight(layer.current.weight * layer.current.slider)}
                          </small>
                        )}
                      </li>
                    ))}
                  </ul>
                  {drift.promptChanged && (
                    <details>
                      <summary>Show current compiled prompt</summary>
                      <pre>{currentPrompt.data?.prompt}</pre>
                    </details>
                  )}
                </>
              )}
            </>
          )}
        </section>
      </div>
    </Modal>
  )
}

function fragmentLabel(fragment: GenerationSnapshotFragment): string {
  if (fragment.text) return fragment.text
  if (fragment.assetId)
    return `asset ${fragment.assetId}${fragment.assetRole ? ` · ${fragment.assetRole}` : ''}`
  return 'No payload'
}

function numberParam(generation: Generation, key: string): number | null {
  const value = generation.params[key]
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function stringParam(generation: Generation, key: string): string | null {
  const value = generation.params[key]
  return typeof value === 'string' ? value : null
}

function sizeLabel(generation: Generation): string {
  const width = numberParam(generation, 'width')
  const height = numberParam(generation, 'height')
  return width && height ? `${width}×${height}` : 'unknown size'
}

function weight(value: number): string {
  return value.toFixed(2).replace(/0+$/, '').replace(/\.$/, '') || '0'
}

function hashPrefix(value: string): string {
  return value.length > 12 ? `${value.slice(0, 12)}…` : value
}

function usd(micros: number): string {
  return new Intl.NumberFormat(undefined, { style: 'currency', currency: 'USD' }).format(
    micros / 1_000_000,
  )
}

function recordedControls(
  generation: Generation,
): { sliders: SliderSetting[]; shot: ShotControls } | null {
  const value = generation.params.controls
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const controls = value as Record<string, unknown>
  if (!Array.isArray(controls.sliders)) return null
  const sliders: SliderSetting[] = []
  for (const candidate of controls.sliders) {
    if (!candidate || typeof candidate !== 'object' || Array.isArray(candidate)) return null
    const slider = candidate as Record<string, unknown>
    if (typeof slider.nodeId !== 'string' || typeof slider.value !== 'number') return null
    sliders.push({ nodeId: slider.nodeId, value: slider.value, muted: slider.muted === true })
  }
  const rawShot = controls.shot
  if (!rawShot || typeof rawShot !== 'object' || Array.isArray(rawShot)) return null
  const candidate = rawShot as Record<string, unknown>
  const shot: ShotControls = {}
  if (typeof candidate.label === 'string') shot.label = candidate.label
  if (typeof candidate.weight === 'number') shot.weight = candidate.weight
  if (typeof candidate.prompt === 'string') shot.prompt = candidate.prompt
  return { sliders, shot }
}
