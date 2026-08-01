import type {
  CompiledPrompt,
  Generation,
  GenerationSnapshotFragment,
  GenerationSnapshotLayer,
  InfluenceFragment,
  InfluenceStack,
  LayerCard,
} from './api'

export type DriftStatus = 'unchanged' | 'changed' | 'added' | 'removed'

export interface LayerDrift {
  key: string
  status: DriftStatus
  historical: GenerationSnapshotLayer | null
  current: LayerCard | null
  changes: string[]
}

export interface GenerationDrift {
  promptChanged: boolean
  negativeChanged: boolean
  negativeComparable: boolean
  layers: LayerDrift[]
}

/** Compare an immutable receipt with a fresh compilation of today's world. */
export function generationDrift(
  generation: Generation,
  currentStack: InfluenceStack | null,
  currentPrompt: CompiledPrompt | null,
): GenerationDrift | null {
  if (!currentStack || !currentPrompt) return null
  const historical = new Map(
    generation.influenceSnapshot.layers.map((layer) => [layerKey(layer), layer]),
  )
  const current = new Map(currentStack.layers.map((layer) => [layerKey(layer), layer]))
  const keys = [...historical.keys(), ...[...current.keys()].filter((key) => !historical.has(key))]
  const layers = keys.map((key): LayerDrift => {
    const before = historical.get(key) ?? null
    const after = current.get(key) ?? null
    if (!before)
      return { key, status: 'added', historical: null, current: after, changes: ['added'] }
    if (!after)
      return { key, status: 'removed', historical: before, current: null, changes: ['removed'] }
    const changes: string[] = []
    if (before.nodeName !== after.name) changes.push('name')
    if (!close(before.weight, after.weight * after.slider)) changes.push('weight')
    if (before.muted !== after.slider <= 0) changes.push('mute')
    if (fragmentFingerprint(before.fragments) !== fragmentFingerprint(after.fragments)) {
      changes.push('fragments')
    }
    return {
      key,
      status: changes.length ? 'changed' : 'unchanged',
      historical: before,
      current: after,
      changes,
    }
  })
  const recordedNegativeSupport = generation.params.negativePromptSupported
  const negativeComparable =
    recordedNegativeSupport === true ||
    (typeof recordedNegativeSupport !== 'boolean' &&
      (generation.negativePrompt.length > 0 || currentPrompt.negative.length === 0))
  return {
    promptChanged: generation.compiledPrompt !== currentPrompt.prompt,
    negativeChanged: negativeComparable && generation.negativePrompt !== currentPrompt.negative,
    negativeComparable,
    layers,
  }
}

function layerKey(layer: GenerationSnapshotLayer | LayerCard): string {
  return `${layer.layer}:${layer.nodeId ?? 'shot'}`
}

function close(left: number, right: number): boolean {
  return Math.abs(left - right) < 0.0001
}

function fragmentFingerprint(
  fragments: Array<GenerationSnapshotFragment | InfluenceFragment>,
): string {
  return JSON.stringify(
    fragments
      .filter((fragment) => fragment.section !== 'user_prompt')
      .map((fragment) => ({
        section: fragment.section,
        text: fragment.text,
        assetId: fragment.assetId,
        weight: Math.round(fragment.weight * 10_000) / 10_000,
        target: fragment.target,
      })),
  )
}
