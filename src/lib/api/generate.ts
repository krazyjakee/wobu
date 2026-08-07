import { call } from './call'
import type { Generation } from './generations'
import type { ShotControls, SliderSetting } from './influence'
import type { InfluenceLayer, LoraPin } from './model'
/* ── domain types ─────────────────────────────────────────────────────────── */

export interface GenerateOptions {
  preset?: string
  sliders?: SliderSetting[]
  shot?: ShotControls
  aspect?: string
  model?: string
  seed?: number
  grid?: VariantGrid
  /**
   * Restrict a named-view preset to some of its views.
   *
   * Only Turnaround has views at all, and this exists for one reason: re-rolling
   * the back view has to be one image tagged `back`, not eight more. Omitted
   * means the whole sheet, which is what the Inspector's Generate button sends.
   */
  views?: string[]
}

export type VariantGrid =
  | { axis: 'seed'; values: number[] }
  | { axis: 'fragment_weight'; nodeId: string; values: number[] }
  | { axis: 'preset'; values: string[] }
  | { axis: 'aspect'; values: string[] }

export interface ReferenceBucketReport {
  bucket: 'objects' | 'characters' | 'style_refs'
  label: string
  kept: number
  limit: number | null
  dropped: number
}

export interface ReferenceLayerReport {
  nodeId: string | null
  layer: InfluenceLayer
  kept: number
  dropped: number
  reasons: string[]
}

export interface CostEstimate {
  currency: 'USD'
  perImageUsdMicros: number
  batchUsdMicros: number
  images: number
  variesByCell: boolean
  indicative: boolean
  conservativeFallback: boolean
  checkedAt: string
  sourceUrl: string
}

export interface SpendStatus {
  /** Null deliberately disables paid generation; it is never an unlimited ceiling. */
  ceilingUsdMicros: number | null
  /** Reconstructed from immutable generation receipts. */
  spentUsdMicros: number
  /** Paid batches admitted but not yet fully receipted. */
  reservedUsdMicros: number
  remainingUsdMicros: number | null
  pendingReservations: number
  oldestReservationAt: string | null
  /** True when a prior process may have crashed while holding the ledger. */
  ledgerLocked: boolean
}

export interface ImageReferenceReport {
  buckets: ReferenceBucketReport[]
  layers: ReferenceLayerReport[]
  /** Null for a local provider such as ComfyUI. */
  cost: CostEstimate | null
  lockedSeed: number | null
}

export const imageReferenceReport = (
  subjectId: string,
  options: Pick<
    GenerateOptions,
    'preset' | 'sliders' | 'shot' | 'aspect' | 'model' | 'seed' | 'grid'
  > = {},
) => call<ImageReferenceReport>('image_reference_report', { subjectId, ...options })

/**
 * The active image backend's ordered aspect vocabulary and the exact shape a
 * request would negotiate to. Flexible backends expose the app's curated ratio
 * vocabulary rather than accepting arbitrary Inspector text.
 */
export interface ImageGenerationCapabilities {
  provider: string
  model: string
  aspectRatios: string[]
  flexibleAspect: boolean
  previews: ImageAspectPreview[]
}

export interface ImageAspectPreview {
  requestedAspect: string
  actualAspect: string
  width: number
  height: number
  substituted: boolean
}

export const imageGenerationCapabilities = (model?: string) =>
  call<ImageGenerationCapabilities>('image_generation_capabilities', {
    ...(model === undefined ? {} : { model }),
  })

export const spendStatus = () => call<SpendStatus>('spend_status')

/** Null disables paid generation for the project. Amounts are integer USD micros. */
export const spendCeilingSet = (ceilingUsdMicros: number | null) =>
  call<SpendStatus>('spend_ceiling_set', { ceilingUsdMicros })

/** Archive crash-orphaned reservations after every paid job has stopped. */
export const spendRecoveryReset = (confirmNoPaidJobs: boolean) =>
  call<SpendStatus>('spend_recovery_reset', { confirmNoPaidJobs })

/** Queue one negotiated image generation and return its job id. */
export const generateStart = (subjectId: string, options: GenerateOptions = {}) =>
  call<string>('generate_start', { subjectId, ...options })

export interface SceneGenerateOptions {
  prompt?: string
  aspect?: string
  model?: string
  seed?: number
}

/** Queue one image containing two to four ordered world entities. */
export const sceneGenerateStart = (subjectIds: string[], options: SceneGenerateOptions = {}) =>
  call<string>('scene_generate_start', { subjectIds, ...options })

export interface LoraStatus {
  subjectId: string
  pinnedCount: number
  invalidPinnedCount: number
  requiredCount: number
  eligible: boolean
  trainerState: string
  trainerDetail: string
  selectedModel: string | null
  pin: LoraPin | null
  applicationState: string
  applicationDetail: string
}

/** Inspect training inputs, the local trainer, and application compatibility. */
export const loraStatus = (subjectId: string) => call<LoraStatus>('lora_status', { subjectId })

/** Queue local LoRA training for one entity. */
export const loraTrainStart = (subjectId: string) => call<string>('lora_train_start', { subjectId })

export interface SceneCompositionReceipt {
  version: 1
  subjectIds: string[]
  subjectNames: string[]
}

export function sceneComposition(generation: Generation): SceneCompositionReceipt | null {
  const value = generation.params.sceneComposition
  if (!value || typeof value !== 'object') return null
  const scene = value as Record<string, unknown>
  if (
    scene.version !== 1 ||
    !Array.isArray(scene.subjectIds) ||
    !Array.isArray(scene.subjectNames)
  ) {
    return null
  }
  if (!scene.subjectIds.every((id) => typeof id === 'string')) return null
  if (!scene.subjectNames.every((name) => typeof name === 'string')) return null
  if (
    scene.subjectIds.length < 2 ||
    scene.subjectIds.length > 4 ||
    scene.subjectIds.length !== scene.subjectNames.length ||
    new Set(scene.subjectIds).size !== scene.subjectIds.length
  ) {
    return null
  }
  return scene as unknown as SceneCompositionReceipt
}

/* ── conflicts ────────────────────────────────────────────────────────────── */
