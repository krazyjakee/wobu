import { useState } from 'react'
import type {
  Generation,
  GenerationSnapshotFragment,
  ShotControls,
  SliderSetting,
} from '../lib/api'
import { generationLoraReceipt, sceneComposition } from '../lib/api'
import { useCompiledPrompt, useInfluenceStack, useReplayGeneration } from '../lib/queries'
import { generationDrift } from '../lib/generationDiff'
import { layerLabel } from '../lib/kinds'
import { sectionLabel } from '../lib/prompt'
import { loraStateLabel } from '../lib/stateLabels'
import { GenerationModelSeed, GenerationSubject, GenerationTimestamp } from './GenerationMetadata'
import { ImageViewer } from './ImageViewer'
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
  const [imageOpen, setImageOpen] = useState(false)
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
            <GenerationSubject generation={generation} fallback={nodeName} /> · {generation.preset}{' '}
            · <GenerationModelSeed generation={generation} includeBackend /> ·{' '}
            <GenerationTimestamp generation={generation} />
          </p>
        </div>
        <div className="generation-detail-actions">
          <button
            className="btn"
            type="button"
            disabled={readOnly || replay.isPending}
            onClick={() => replay.mutate(generation.id)}
          >
            {replay.isPending ? 'Queuing…' : 'Run these settings again'}
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
        <section className="generation-receipt" aria-label="What was sent">
          {imageSrc && (
            <button
              className="generation-receipt-image"
              type="button"
              onClick={() => setImageOpen(true)}
              aria-label="View generated image full size"
            >
              <img src={imageSrc} alt={generation.compiledPrompt} />
            </button>
          )}
          <dl>
            <dt>Your extra direction</dt>
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
            <dt>Sent as</dt>
            <dd>
              {stringParam(generation, 'aspect') ?? 'unknown aspect'} · {sizeLabel(generation)} ·
              seed {generation.seed}
            </dd>
            <dt>Receipt</dt>
            <dd>
              {generation.id}
              {typeof generation.params.replayOf === 'string' && (
                <> · a repeat of {generation.params.replayOf}</>
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
            Running it again sends exactly what is recorded here, without looking at today’s world.
            If it costs money, it is set aside at today’s price for the model, separately from the
            original estimate.
          </p>
          {(loras.applied.length > 0 || loras.downgrades.length > 0) && (
            <section className="generation-lora-receipt" aria-label="Trained styles used">
              <h3>Trained styles (LoRAs)</h3>
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
                        <strong>{loraStateLabel(lora.state)}</strong>
                        <span>{lora.detail}</span>
                        <small>
                          {hashPrefix(lora.contentHash)} · entity {lora.nodeId}
                        </small>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </section>
          )}
        </section>

        <section className="generation-snapshot" aria-label="The influence stack that was recorded">
          <h3>The exact stack that was used</h3>
          {generation.influenceSnapshot.layers.map((layer) => (
            <article className="generation-layer" key={`${layer.layer}:${layer.nodeId ?? 'shot'}`}>
              <header>
                <b>{layer.nodeName}</b>
                <span>
                  {layerLabel(layer.layer)} · weight {weight(layer.weight)}
                  {layer.muted ? ' · muted' : ''}
                </span>
              </header>
              <ul>
                {layer.fragments.map((fragment, index) => (
                  <li key={`${fragment.section}:${fragment.assetId ?? fragment.text}:${index}`}>
                    <span>
                      {sectionLabel(fragment.section)} · {weight(fragment.weight)} ·{' '}
                      {sectionLabel(fragment.target)}
                      {fragment.dropped ? ' · dropped' : ''}
                    </span>
                    <p>{fragmentLabel(fragment)}</p>
                  </li>
                ))}
              </ul>
            </article>
          ))}
        </section>

        <section className="generation-drift" aria-label="What has changed since">
          <h3>
            {scene
              ? 'What has changed since (scene)'
              : controls
                ? 'What has changed since'
                : 'Today with default controls'}
          </h3>
          {scene && (
            <p className="generation-drift-basis">
              This receipt keeps the combined stack of several entities, so Wobu does not compare it
              against the first entity alone.
            </p>
          )}
          {!scene && (
            <>
              <p className="generation-drift-basis">
                {controls
                  ? 'Today’s world is compiled again using the sliders and shot controls that were recorded here.'
                  : 'This receipt was written before Wobu stored the controls, so a different weight here may be a control you set at the time rather than a change to the world.'}
              </p>
              {(currentStack.isPending || currentPrompt.isPending) && (
                <p>Working out today’s stack…</p>
              )}
              {(currentStack.isError || currentPrompt.isError) && (
                <p>
                  Today’s stack could not be worked out. What was recorded above is still complete.
                </p>
              )}
              {drift && (
                <>
                  <p
                    className={
                      drift.promptChanged || drift.negativeChanged ? 'is-drifted' : 'is-same'
                    }
                  >
                    {drift.promptChanged || drift.negativeChanged
                      ? `The prompt has changed${drift.negativeChanged ? ' (including negative prompt)' : ''}.`
                      : 'The parts that can be compared are unchanged.'}
                  </p>
                  {!drift.negativeComparable && (
                    <p className="generation-drift-basis">
                      Wobu will not say whether the Never list has changed: this receipt does not
                      say what the provider accepted at the time.
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
                      <summary>Show today’s compiled prompt</summary>
                      <pre>{currentPrompt.data?.prompt}</pre>
                    </details>
                  )}
                </>
              )}
            </>
          )}
        </section>
      </div>
      {imageOpen && imageSrc && (
        <ImageViewer
          src={imageSrc}
          alt={generation.compiledPrompt}
          title="Full-size generated image"
          description="The original generated image. Press Escape, or use Close, to go back to the details."
          onClose={() => setImageOpen(false)}
        />
      )}
    </Modal>
  )
}

function fragmentLabel(fragment: GenerationSnapshotFragment): string {
  if (fragment.text) return fragment.text
  if (fragment.assetId)
    return `asset ${fragment.assetId}${fragment.assetRole ? ` · ${fragment.assetRole}` : ''}`
  return 'Nothing recorded'
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
