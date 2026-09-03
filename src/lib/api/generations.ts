import { call } from './call'
import type { FragmentTarget } from './influence'
import type { Asset, AssetRole, InfluenceLayer } from './model'
/* ── domain types ─────────────────────────────────────────────────────────── */

export interface GenerationSnapshotFragment {
  section: string
  text: string | null
  assetId: string | null
  /** Exact reference role when recorded; absent on older receipts. */
  assetRole?: AssetRole | null
  weight: number
  target: FragmentTarget
  dropped: boolean
}

export interface GenerationSnapshotLayer {
  layer: InfluenceLayer
  nodeId: string | null
  nodeName: string
  weight: number
  muted: boolean
  fragments: GenerationSnapshotFragment[]
}

/** Immutable generation receipt stored under `generations/YYYY-MM/`. */
export interface Generation {
  id: string
  nodeId: string
  createdAt: string
  preset: string
  viewType: string | null
  userPrompt: string
  compiledPrompt: string
  negativePrompt: string
  backend: string
  model: string
  seed: number
  params: Record<string, unknown>
  outputAssetIds: string[]
  influenceSnapshot: { layers: GenerationSnapshotLayer[] }
}

export interface GenerationSummary {
  id: string
  nodeId: string
  createdAt: string
  preset: string
  viewType: string | null
  backend: string
  model: string
  seed: number
  promptExcerpt: string
  firstAssetId: string | null
  outputCount: number
  seedSource: string | null
  usedLockedSeed: boolean | null
  sceneSubjectNames: string[]
  thumbnailPath: string | null
}

export interface GenerationPage {
  items: GenerationSummary[]
  total: number
  nextOffset: number | null
}

export interface GenerationRecorded {
  subjectId: string
  generation: Generation
  asset: Asset | null
}

export interface AppliedLoraReceipt {
  nodeId: string
  contentHash: string
  providerName: string
  triggerToken: string
  strength: number
}

export interface LoraDowngradeReceipt {
  nodeId: string
  contentHash: string
  state: string
  detail: string
}

/** Defensively read optional LoRA metadata from immutable generation params. */
export function generationLoraReceipt(generation: Generation): {
  applied: AppliedLoraReceipt[]
  downgrades: LoraDowngradeReceipt[]
} {
  const applied = Array.isArray(generation.params.loras)
    ? generation.params.loras.flatMap((value) => {
        if (!value || typeof value !== 'object' || Array.isArray(value)) return []
        const candidate = value as Record<string, unknown>
        if (
          typeof candidate.nodeId !== 'string' ||
          !candidate.nodeId ||
          typeof candidate.contentHash !== 'string' ||
          !candidate.contentHash ||
          typeof candidate.providerName !== 'string' ||
          !candidate.providerName ||
          typeof candidate.triggerToken !== 'string' ||
          !candidate.triggerToken ||
          typeof candidate.strength !== 'number' ||
          !Number.isFinite(candidate.strength) ||
          candidate.strength < 0 ||
          candidate.strength > 2
        )
          return []
        return [candidate as unknown as AppliedLoraReceipt]
      })
    : []
  const downgrades = Array.isArray(generation.params.loraDowngrades)
    ? generation.params.loraDowngrades.flatMap((value) => {
        if (!value || typeof value !== 'object' || Array.isArray(value)) return []
        const candidate = value as Record<string, unknown>
        if (
          typeof candidate.nodeId !== 'string' ||
          !candidate.nodeId ||
          typeof candidate.contentHash !== 'string' ||
          !candidate.contentHash ||
          typeof candidate.state !== 'string' ||
          !candidate.state ||
          typeof candidate.detail !== 'string' ||
          !candidate.detail
        )
          return []
        return [candidate as unknown as LoraDowngradeReceipt]
      })
    : []
  return { applied, downgrades }
}

/** One bounded page of a node's lightweight generation receipts. */
export const generationList = (nodeId: string, offset: number, limit: number) =>
  call<GenerationPage>('generation_list', { nodeId, offset, limit })

export interface MeshAsset {
  id: string
  hash: string
  bytes: number
  createdAt: string
}

export interface TurnaroundView {
  generationId: string
  viewType: string
  assetId: string
}

export interface MeshConcept {
  generationId: string
  createdAt: string
  backend: string
  model: string
  asset: MeshAsset
  /** Empty when the immutable source receipt was absent or incomplete. */
  turnaround: TurnaroundView[]
}

/** Mesh metadata only. The GLB body remains untouched until `meshAssetPath`. */
export const meshConcepts = (nodeId: string) => call<MeshConcept[]>('mesh_concepts', { nodeId })

/** Full validation and absolute path for the one GLB the open viewer needs. */
export const meshAssetPath = (assetId: string) =>
  call<string | null>('mesh_asset_path', { assetId })

/** Canonical project GLB path, requested only when the user chooses Reveal. */
export const meshSourcePath = (assetId: string) =>
  call<string | null>('mesh_source_path', { assetId })

/** Copy a fully validated GLB to the modeller's chosen destination. */
export const meshExport = (assetId: string, destination: string) =>
  call<void>('mesh_export', { assetId, destination })

/**
 * One rendered answer for one of the eight views.
 *
 * A view can have several: the Turnaround preset locks one seed across the whole
 * sheet, so re-rolling a single view produces a take on a seed of its own rather
 * than a ninth member of the original batch.
 */
export interface TurnaroundTake {
  generationId: string
  assetId: string
  seed: number
  createdAt: string
  backend: string
  model: string
}

export interface TurnaroundSlot {
  viewType: string
  /** Newest first. Empty when this view has never been rendered. */
  takes: TurnaroundTake[]
}

/** One complete eight-view run, identified by the seed the preset locked. */
export interface TurnaroundBatch {
  seed: number
  createdAt: string
  /** In provider view order — front, left, right, back, top, bottom, and the two three-quarters. */
  generationIds: string[]
}

export interface TurnaroundSheet {
  /** Always eight, in provider view order, present or not. */
  views: TurnaroundSlot[]
  /** Complete runs, newest first. */
  batches: TurnaroundBatch[]
  /** View names with nothing rendered yet. */
  missing: string[]
}

/** What this entity has rendered towards a mesh. Receipts only; no image bytes. */
export const turnaroundSheet = (nodeId: string) =>
  call<TurnaroundSheet>('turnaround_sheet', { nodeId })

/**
 * What the project's selected 3D backend accepts, and whether it could run now.
 *
 * `requiresBilling` is the gate on `meshStart`: a backend that charges per job
 * and cannot report the amount back has to be confirmed rather than estimated.
 */
export interface MeshOptions {
  /** Null when `project.json` selects no 3D backend at all. */
  provider: string | null
  label: string
  model: string
  region: string | null
  /** Including the front view. One means single-image reconstruction. */
  maxViews: number
  faceCountMin: number
  faceCountMax: number
  defaultFaceCount: number
  pbr: boolean
  generateTypes: string[]
  requiresBilling: boolean
  ready: boolean
  /** Why not, when `ready` is false. */
  detail: string
}

export const meshOptions = () => call<MeshOptions>('mesh_options')

export interface MeshStartOptions {
  faceCount?: number
  enablePbr?: boolean
  generateType?: string
  /** Required by the backend whenever `requiresBilling` is true. */
  acceptCost?: boolean
}

/** Queue one reconstruction from reviewed turnaround views and return its job id. */
export const meshStart = (
  nodeId: string,
  generationIds: string[],
  options: MeshStartOptions = {},
) => call<string>('mesh_start', { nodeId, generationIds, ...options })

/** Full immutable receipt for the one history tile being opened. */
export const generationGet = (generationId: string) =>
  call<Generation | null>('generation_get', { generationId })

/** Remove one visible concept receipt while retaining its archived copy. */
export const generationDelete = (generationId: string) =>
  call<void>('generation_delete', { generationId })

/** Resolve one page of thumbnail paths in a single backend command. */
export const assetThumbBatch = (assetIds: string[]) =>
  call<Record<string, string>>('asset_thumb_batch', { assetIds })

/** Queue the immutable request captured by a past generation. */
export const generationReplay = (generationId: string) =>
  call<string>('generation_replay', { generationId })
