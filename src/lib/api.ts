/**
 * Typed wrappers over the Rust command surface.
 *
 * Tauri v2 converts camelCase JS argument keys to snake_case Rust parameters,
 * and every payload struct is serde(rename_all = "camelCase"), so the shapes
 * below are exactly what crosses the bridge.
 */
import { invoke } from '@tauri-apps/api/core'

/* ── domain types ─────────────────────────────────────────────────────────── */

export type NodeKind =
  | 'style_guide'
  | 'world_bible'
  | 'species'
  | 'culture'
  | 'setting'
  | 'character'
  | 'creature'
  | 'prop'
  | 'environment'
  | 'vehicle'

export type DescriptionState = 'none' | 'enhancing' | 'fresh' | 'edited' | 'stale'

export type LinkRole = 'species_of' | 'member_of' | 'located_in' | 'styled_by' | 'related_to'

export type SectionValue = { type: 'text'; value: string } | { type: 'list'; value: string[] }

export type SectionValueKind = 'text' | 'list'

export type AttributeValueKind = 'text' | 'number' | 'boolean'

/** `key` indexes `WobuNode.attributes`; the value kind selects the generated control. */
export interface AttributeDef {
  key: string
  label: string
  valueKind: AttributeValueKind
}

/** `key` indexes `WobuNode.description.sections`; `label` is the rendered heading. */
export interface SectionDef {
  key: string
  label: string
  valueKind: SectionValueKind
}

export type InfluenceLayer =
  'style' | 'world' | 'ancestry' | 'culture' | 'place' | 'subject' | 'shot'

export interface KindDef {
  kind: NodeKind
  label: string
  plural: string
  icon: string
  color: string
  layer: InfluenceLayer
  /** folder under nodes/ — not used by the UI */
  dir: string
  nests: boolean
  singleton: boolean
  /** Kind-specific facts rendered as controls in the Notes tab. */
  attributes: AttributeDef[]
  /** already in the kind's intended display order */
  sections: SectionDef[]
  defaultLinkRoles: LinkRole[]
}

export interface ProjectSummary {
  id: string
  name: string
  path: string
  onNetworkShare: boolean
  readOnly: boolean
  lastOpenedAt: string | null
}

export interface Link {
  toId: string
  role: LinkRole
  weight: number
  enabled: boolean
}

/** An indexed link with both endpoints, used by the Relations backlinks list. */
export interface LinkEdge extends Link {
  fromId: string
}

/**
 * What a reference image is *for*, and therefore where it is routed when a
 * generation is compiled — a `palette` reference goes to colour conditioning, a
 * `pose` reference to a structure adapter.
 *
 * `mood` is the exception worth knowing: it is `moodboard_only`, shown to the
 * human and never sent anywhere. The backend enforces that; nothing on this
 * side should be written that assumes otherwise.
 */
export type AssetRole =
  'silhouette' | 'palette' | 'material' | 'mood' | 'pose' | 'costume' | 'full_ref'

export const ASSET_ROLES: AssetRole[] = [
  'silhouette',
  'palette',
  'material',
  'mood',
  'pose',
  'costume',
  'full_ref',
]

/**
 * A reference image attached to a node.
 *
 * `(assetId, role)` is the identity: the same picture can be both a `full_ref`
 * and a `palette` source for one entity, because those reach two different
 * adapters. Weight is 0.0–1.0, defaulting to 1.0, exactly as on `Link`.
 */
export interface AssetLink {
  assetId: string
  role: AssetRole
  weight: number
  /** Muted for now, without detaching it. */
  enabled: boolean
}

/** One immutable, project-owned entity fine-tune pinned by its content hash. */
export interface LoraPin {
  hash: string
  relPath: string
  bytes: number
  trainer: string
  protocol: number
  baseModel: string
  modelFamily: string
  providerName: string
  triggerToken: string
  inputAssetHashes: string[]
  createdAt: string
  strength: number
}

export interface NodeSummary {
  id: string
  kind: NodeKind
  name: string
  summary: string
  parentId: string | null
  descriptionState: DescriptionState
  slug: string
}

export interface WobuNode {
  id: string
  kind: NodeKind
  name: string
  slug: string
  summary: string
  parentId: string | null
  notesRaw: string
  description: { sections: Record<string, SectionValue> } | null
  descriptionState: DescriptionState
  attributes: Record<string, unknown>
  tags: string[]
  /** The image shown on this entity's card. Independent of `assetLinks`. */
  coverAssetId: string | null
  /** Shared identity seed used whenever Generate has no explicit re-roll. */
  lockedSeed: number | null
  /** Optional local entity fine-tune, applied only when its model is compatible. */
  lora: LoraPin | null
  links: Link[]
  assetLinks: AssetLink[]
  createdAt: string
  updatedAt: string
}

/**
 * What a blob is for. Roles (`palette`, `pose`, …) live on the link between an
 * asset and a node; this is the coarser question of where the file came from.
 */
export type AssetKind = 'reference' | 'generated' | 'upload'

/**
 * A file in `assets/originals/`, addressed by the hash of its contents.
 *
 * Both `id` and `relPath` are derived from that hash and from nothing else, so
 * two people importing the same picture on one share produce the same file
 * *and* the same record — and neither survives being renamed, because neither
 * ever depended on a name. `id` is what `coverAssetId` and every AssetLink
 * point at.
 */
export interface Asset {
  id: string
  /** Lowercase hex BLAKE3 of the file. This, not the id, names the file. */
  hash: string
  kind: AssetKind
  /** Project-relative, `/`-separated. Never absolute. */
  relPath: string
  /** Null until something has actually made a thumbnail. */
  thumbPath: string | null
  /** Read out of the file's own header, not out of its extension. */
  mime: string
  width: number
  height: number
  bytes: number
  createdAt: string
}

export interface AssetUsageRole {
  role: AssetRole
  weight: number
  enabled: boolean
}

/** One node's complete use of an asset, including its independent cover use. */
export interface AssetUsage {
  assetId: string
  nodeId: string
  nodeName: string
  nodeKind: NodeKind
  /** Canonical tags on the linked node; assets have no mutable tag document. */
  nodeTags: string[]
  roles: AssetUsageRole[]
  cover: boolean
}

/* ── environment ──────────────────────────────────────────────────────────── */

/** True when running inside the Tauri webview (as opposed to a bare `vite dev`). */
export const isTauri = (): boolean =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/* ── errors ───────────────────────────────────────────────────────────────── */

/**
 * The stable codes `src-tauri/src/error.rs` emits. Renaming one means renaming
 * it on both sides at once — there is a Rust test pinning every string here.
 *
 * Left open with `(string & {})` on purpose: the provider codes exist in Rust
 * before the provider crates do, and a code this build has never heard of must
 * degrade to "unknown error, show the message" rather than fail to type-check.
 */
export type ErrorCode =
  | 'project.not_a_project'
  | 'project.already_exists'
  | 'project.schema_too_new'
  | 'project.none_open'
  | 'transfer.same_project'
  | 'node.not_found'
  | 'node.malformed'
  | 'node.invalid'
  | 'write.conflict'
  | 'write.read_only'
  | 'asset.not_an_image'
  | 'asset.not_found'
  | 'asset.in_use'
  | 'share.unmounted'
  | 'provider.no_key'
  | 'provider.keychain_unavailable'
  | 'provider.bad_key'
  | 'provider.clock_skew'
  | 'provider.billing_required'
  | 'provider.rate_limited'
  | 'provider.unavailable'
  | 'provider.bad_response'
  | 'provider.context_too_long'
  | 'billing.ceiling_exceeded'
  | 'io.failed'
  | 'cancelled'
  | 'internal'
  | (string & {})

/** Exactly what a failing command rejects with. */
export interface WobuError {
  code: ErrorCode
  message: string
  /** Technical remainder — the OS's own wording, a parser position. */
  detail?: string
  retryable: boolean
  /** Only on `write.conflict`: where our version was parked. */
  conflictPath?: string
}

export function isWobuError(e: unknown): e is WobuError {
  return (
    !!e &&
    typeof e === 'object' &&
    typeof (e as WobuError).code === 'string' &&
    typeof (e as WobuError).message === 'string'
  )
}

/** The code, or `null` for anything that did not come from a command. */
export function errorCode(e: unknown): ErrorCode | null {
  return isWobuError(e) ? e.code : null
}

/**
 * Where an error belongs on screen.
 *
 * A banner is persistent and occupies real estate, so it is reserved for the
 * two things that make the *whole workspace* untrustworthy until resolved:
 * the project folder went away, or it turned read-only underneath us.
 * Everything else — a rejected save, a bad name, a failed thumbnail — is a
 * toast, because the user already knows which action produced it and the rest
 * of the app still works. Getting this wrong in the generous direction is
 * worse: a banner that appears for routine failures is one the user learns to
 * ignore.
 *
 * Both banner codes are specifically *mid-session* conditions. Open-time
 * failures are not here on purpose: `project.not_a_project` and
 * `project.schema_too_new` can only happen while the Launcher is on screen,
 * which has its own inline error slot and no workspace to put a banner above.
 * A read-only folder detected at open does get a banner, but not through here:
 * the Workspace raises `project.read_only` once when the project opens (see
 * `src/lib/readOnly.ts`), because nothing failed — that is the state of the
 * folder rather than the outcome of a command. `write.read_only` reaching
 * *here* means a folder that started writable changed underneath the session,
 * which is a failure and needs saying in its own words.
 */
export type Surface = 'banner' | 'toast' | 'silent'

export function errorSurface(e: unknown): Surface {
  switch (errorCode(e)) {
    case 'share.unmounted':
    case 'write.read_only':
      return 'banner'
    // Not a failure. The user asked for this to stop, and telling them it
    // stopped is the app arguing with them about something they just did.
    case 'cancelled':
      return 'silent'
    default:
      return 'toast'
  }
}

export function isRetryable(e: unknown): boolean {
  return isWobuError(e) && e.retryable
}

/** Normalises the assorted things a Rust command error can arrive as. */
export function errorMessage(e: unknown): string {
  if (typeof e === 'string') return e
  if (isWobuError(e)) return e.message
  if (e instanceof Error) return e.message
  if (e && typeof e === 'object') {
    const m = (e as { message?: unknown }).message
    if (typeof m === 'string') return m
    try {
      return JSON.stringify(e)
    } catch {
      return String(e)
    }
  }
  return String(e)
}

function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    return Promise.reject(
      new Error(
        `Not running inside Tauri — the "${cmd}" command is unavailable. Launch with \`npm run tauri dev\`.`,
      ),
    )
  }
  return invoke<T>(cmd, args)
}

/* ── commands ─────────────────────────────────────────────────────────────── */

export const kindRegistry = () => call<KindDef[]>('kind_registry')

export const projectCreate = (parentDir: string, name: string) =>
  call<ProjectSummary>('project_create', { parentDir, name })

export const projectOpen = (path: string) => call<ProjectSummary>('project_open', { path })

export interface TransferCandidate {
  rootId: string
  kind: NodeKind
  name: string
  nodeCount: number
  referenceCount: number
  externalLinkCount: number
  missingAssetCount: number
  loraCount: number
  missingLoraCount: number
  replacesSingleton: boolean
}

export interface TransferPreview {
  version: number
  sourceProjectId: string
  sourceProjectName: string
  defaultRootId: string | null
  candidates: TransferCandidate[]
  pinnedLoras: string[]
  loraNote: string
}

export interface TransferOutcome {
  completed: boolean
  rootId: string
  importedRootId: string
  plannedNodeCount: number
  appliedNodeIds: string[]
  pendingNodeIds: string[]
  referenceCount: number
  dedupedReferenceCount: number
  loraCount: number
  dedupedLoraCount: number
  droppedExternalLinkCount: number
  replacedSingleton: boolean
  conflictPaths: string[]
  failure: string | null
}

export const styleTransferPreview = (sourcePath: string) =>
  call<TransferPreview>('style_transfer_preview', { sourcePath })

export const styleTransferApply = (sourcePath: string, rootId: string) =>
  call<TransferOutcome>('style_transfer_apply', { sourcePath, rootId })

/**
 * How far through the first scan of a project the backend is.
 *
 * `total` comes from one directory listing, which is cheap even over SMB —
 * re-reading the files is the expensive part. It can be stale by the end if
 * somebody else is writing, so it is an estimate, not a promise.
 */
export interface ScanProgress {
  done: number
  total: number
}

/**
 * Stop a scan in flight.
 *
 * A no-op if it has already finished — the user can press Cancel at the instant
 * the scan completes, and that race must not be an error.
 */
export const projectOpenCancel = () => call<void>('project_open_cancel')

export const projectRecent = () => call<ProjectSummary[]>('project_recent')

/** Remove one launcher hint without touching the project folder it points to. */
export const projectRecentForget = (id: string) => call<void>('project_recent_forget', { id })

export const projectCurrent = () => call<ProjectSummary | null>('project_current')

export const projectClose = () => call<void>('project_close')

export interface WikiExport {
  destination: string
  nodeCount: number
  imageCount: number
  missingImages: number
}

/** Render the open world into a new, self-contained static-site folder. */
export const projectExportWiki = (destination: string) =>
  call<WikiExport>('project_export_wiki', { destination })

/**
 * Whether the open project's folder is currently unreachable.
 *
 * The `share:offline` / `share:online` events are the live signal; this covers
 * the one case they cannot — a webview that reloaded while disconnected, and
 * so missed the event that would have raised the banner.
 */
export const shareOffline = () => call<boolean>('share_offline')

/** Quit anyway, having been told what quitting while offline costs. */
export const forceQuit = () => call<void>('force_quit')

/* ── peer-to-peer sync ───────────────────────────────────────────────────── */

export type SyncPhase = 'idle' | 'connecting' | 'syncing' | 'offline'

export interface SyncPeerStatus {
  /** The authenticated endpoint identity; aliases are display-only. */
  endpointId: string
  alias: string
  /** True only while a live round is using this peer. */
  connected: boolean
  /** ISO timestamp, absent until a complete conflict-free round finishes. */
  lastConvergedAt: string | null
}

/** Payload shared by `sync:state`, `sync:peer`, and the catch-up query. */
export interface ProjectSyncStatus {
  project: string
  state: SyncPhase
  peers: SyncPeerStatus[]
}

export interface SharedProjectStatus {
  project: string
  root: string
  peers: number
  open: boolean
}

export interface SyncStatus {
  running: boolean
  alias: string
  endpointId: string
  persistent: boolean
  shares: SharedProjectStatus[]
  projects: ProjectSyncStatus[]
}

export interface SharedTicket {
  project: string
  token: string
  relayed: boolean
  alias: string
}

export interface AcceptedTicket {
  project: string
  alias: string
  joined: boolean
  /** Present when an existing replica was joined or a clone is ready to open. */
  root: string | null
}

export const syncStatus = () => call<SyncStatus>('sync_status')

export const syncShare = () => call<SharedTicket>('sync_share')

/** Probe without `destination`; pass a parent folder to create or resume a clone. */
export const syncAccept = (token: string, destination?: string) =>
  call<AcceptedTicket | null>('sync_accept', {
    token,
    destination: destination ?? null,
    cancel: false,
  })

/** Signal the in-flight Accept operation from a second command invocation. */
export const syncAcceptCancel = () =>
  call<null>('sync_accept', { token: null, destination: null, cancel: true })

export const syncUnshare = (project: string) => call<void>('sync_unshare', { project })

export const nodeList = () => call<NodeSummary[]>('node_list')

/** Every explicit influence edge. Parent edges are derived from NodeSummary. */
export const nodeLinks = () => call<LinkEdge[]>('node_links')

/**
 * A node file that is on disk and cannot be read — a sync client copied it
 * half-written, most likely, leaving truncated YAML frontmatter.
 */
export interface CorruptFile {
  /** Project-relative, `/`-separated. Never absolute. */
  relPath: string
  /** Set when the index still remembers the entity this file used to be. */
  nodeId: string | null
  /** The parser's own words — the only thing that says what to fix. */
  error: string
  /** When it was first seen broken, not when it was last scanned. */
  detectedAt: string
}

export const corruptFiles = () => call<CorruptFile[]>('corrupt_files')

/** Re-read the folder now instead of waiting for the watcher's debounce. */
export const projectReload = () => call<void>('project_reload')

/**
 * Full-text search over names, summaries, notes and descriptions.
 *
 * Returns ids in rank order, not nodes: the caller already holds every summary
 * from `node_list`, and sending them back would duplicate the world across the
 * bridge on every keystroke. Rank order is the part that cannot be
 * reconstructed on this side.
 */
export const nodeSearch = (query: string) => call<string[]>('node_search', { query })

export const nodeGet = (id: string) => call<WobuNode>('node_get', { id })

export const nodeCreate = (kind: NodeKind, name: string, parentId: string | null) =>
  call<WobuNode>('node_create', { kind, name, parentId })

export const nodeUpsert = (node: WobuNode) => call<WobuNode>('node_upsert', { node })

/** Set the node-persisted identity seed, or null to clear it. */
export const nodeSeedLockSet = (nodeId: string, seed: number | null) =>
  call<WobuNode>('node_seed_lock_set', { nodeId, seed })

export const nodeDelete = (id: string) => call<void>('node_delete', { id })

export const nodeMove = (id: string, newParentId: string | null) =>
  call<void>('node_move', { id, newParentId })

/** Add an explicit influence edge. The backend also enforces the kind registry's roles. */
export const nodeLinkAdd = (
  nodeId: string,
  toId: string,
  role: LinkRole,
  options: { weight?: number; enabled?: boolean } = {},
) => call<WobuNode>('node_link_add', { nodeId, toId, role, ...options })

/** Remove one `(target, role)` edge; the target node itself is untouched. */
export const nodeLinkRemove = (nodeId: string, toId: string, role: LinkRole) =>
  call<WobuNode>('node_link_remove', { nodeId, toId, role })

/** Re-weight or mute an edge while leaving omitted properties unchanged. */
export const nodeLinkUpdate = (
  nodeId: string,
  toId: string,
  role: LinkRole,
  patch: { weight?: number; enabled?: boolean },
) => call<WobuNode>('node_link_update', { nodeId, toId, role, ...patch })

/** Every explicit link whose target is `id`. */
export const nodeBacklinks = (id: string) => call<LinkEdge[]>('node_backlinks', { id })

/* ── assets ───────────────────────────────────────────────────────────────── */

/**
 * What an import did, as opposed to what it produced.
 *
 * `deduped` is the only part a caller cannot work out for itself: the asset
 * comes back identical whether the bytes were written or were already there,
 * which is exactly what content addressing is for.
 */
export interface ImportedAsset {
  asset: Asset
  /** True when the picture was already in the folder and nothing was written. */
  deduped: boolean
}

/**
 * Import a file by path — a drop, or a file picker result.
 *
 * The path is read and then discarded. What the file was called has no bearing
 * on where it lands or on the id it gets, so importing the same picture twice
 * under two names is a no-op the second time.
 *
 * Rejects with `asset.not_an_image` for anything that is not PNG, JPEG, GIF or
 * WebP; the format is read out of the file's header, not its extension.
 */
export const assetImport = (path: string, kind: AssetKind = 'reference') =>
  call<ImportedAsset>('asset_import', { path, kind })

export const ASSET_TRANSFER_CHUNK_BYTES = 1024 * 1024
export const ASSET_TRANSFER_MAX_BYTES = 512 * 1024 * 1024

export interface AssetTransferProgress {
  transferId: string
  receivedBytes: number
  totalBytes: number
}

export interface AssetTransferOptions {
  signal?: AbortSignal
  onProgress?: (progress: AssetTransferProgress) => void
}

/**
 * Import a paste/browser drop without ever expanding it into a JSON number
 * array—or reading the whole Blob into a second webview buffer.
 *
 * Tauri's top-level ArrayBuffer invoke body is binary IPC. Chunks are strictly
 * backpressured: the next slice is not read until Rust has appended the current
 * one to its temp file. The desktop shell therefore holds at most one 1 MiB JS
 * chunk and one 1 MiB Rust IPC body in addition to the browser-owned Blob.
 */
export async function assetImportBytes(
  blob: Blob,
  kind: AssetKind = 'reference',
  options: AssetTransferOptions = {},
): Promise<ImportedAsset> {
  if (blob.size <= 0 || blob.size > ASSET_TRANSFER_MAX_BYTES) {
    throw new Error(
      `Pasted images must be between 1 byte and ${ASSET_TRANSFER_MAX_BYTES / 1024 / 1024} MiB.`,
    )
  }
  throwIfAssetTransferAborted(options.signal)
  const started = await call<AssetTransferProgress>('asset_import_transfer_begin', {
    totalBytes: blob.size,
    kind,
  })
  options.onProgress?.(started)

  try {
    for (let offset = 0; offset < blob.size; offset += ASSET_TRANSFER_CHUNK_BYTES) {
      throwIfAssetTransferAborted(options.signal)
      const end = Math.min(blob.size, offset + ASSET_TRANSFER_CHUNK_BYTES)
      const chunk = await blob.slice(offset, end).arrayBuffer()
      throwIfAssetTransferAborted(options.signal)
      const progress = await rawCall<AssetTransferProgress>('asset_import_transfer_chunk', chunk, {
        'x-wobu-transfer-id': started.transferId,
        'x-wobu-offset': String(offset),
      })
      options.onProgress?.(progress)
    }
    throwIfAssetTransferAborted(options.signal)
    return await call<ImportedAsset>('asset_import_transfer_finish', {
      transferId: started.transferId,
    })
  } catch (error) {
    await call<void>('asset_import_transfer_cancel', { transferId: started.transferId }).catch(
      () => undefined,
    )
    throw error
  }
}

function rawCall<T>(cmd: string, body: ArrayBuffer, headers: Record<string, string>): Promise<T> {
  if (!isTauri()) {
    return Promise.reject(
      new Error(
        `Not running inside Tauri — the "${cmd}" command is unavailable. Launch with \`npm run tauri dev\`.`,
      ),
    )
  }
  return invoke<T>(cmd, body, { headers })
}

function throwIfAssetTransferAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted) throw new DOMException('The image import was cancelled.', 'AbortError')
}

/** Every blob in the open project, newest first. */
export const assetList = () => call<Asset[]>('asset_list')

/** Every node/role/cover using every asset, for library filters and details. */
export const assetUsageList = () => call<AssetUsage[]>('asset_usage_list')

/** Permanently delete one orphan; the backend refuses every linked/cover use. */
export const assetDelete = (assetId: string) => call<void>('asset_delete', { assetId })

/** Thumbnail path for grids; null when the asset is absent or cannot decode. */
export const assetThumb = (assetId: string) => call<string | null>('asset_thumb', { assetId })

/** Full-resolution path, fetched only when a viewer is opened. */
export const assetOriginal = (assetId: string) => call<string | null>('asset_original', { assetId })

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
  presets: string[]
  models: string[]
  nextOffset: number | null
}

export interface GenerationFilters {
  preset?: string
  model?: string
  from?: string
  to?: string
  seed?: number
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

/** One filtered, bounded page of project generation summaries. */
export const generationListAll = (offset: number, limit: number, filters: GenerationFilters) =>
  call<GenerationPage>('generation_list_all', { offset, limit, ...filters })

/** Full immutable receipt for the one history tile being opened. */
export const generationGet = (generationId: string) =>
  call<Generation | null>('generation_get', { generationId })

/** Remove one visible concept receipt while retaining its archived spend record. */
export const generationDelete = (generationId: string) =>
  call<void>('generation_delete', { generationId })

/** Resolve one page of thumbnail paths in a single backend command. */
export const assetThumbBatch = (assetIds: string[]) =>
  call<Record<string, string>>('asset_thumb_batch', { assetIds })

/** Queue the immutable request captured by a past generation. */
export const generationReplay = (generationId: string) =>
  call<string>('generation_replay', { generationId })

/**
 * Attach a reference image to a node in a role.
 *
 * All four calls below return the saved node — attaching a reference is an edit
 * to that node's Markdown, so it goes through the same guarded write as any
 * other and can reject with `write.conflict` in exactly the same way.
 *
 * `weight` is 0.0–1.0 and defaults to 1.0; anything outside the range is
 * clamped rather than refused. Rejects with `asset.not_found` if the id names
 * no blob in this project — an asset id is derived from a file's hash, so one
 * that matches nothing here matches nothing anywhere.
 */
export const assetLink = (nodeId: string, assetId: string, role: AssetRole, weight?: number) =>
  call<WobuNode>('asset_link', { nodeId, assetId, role, weight })

/**
 * Detach one.
 *
 * The picture itself is untouched: assets are content-addressed and shared
 * between nodes, so removing the last link is not a reason to delete the file.
 * Rejects with `asset.not_found` when there is no such link, which is what a
 * panel showing a reference somebody else already removed will get.
 */
export const assetUnlink = (nodeId: string, assetId: string, role: AssetRole) =>
  call<WobuNode>('asset_unlink', { nodeId, assetId, role })

/** Choose the image on a node's card, or pass `null` to clear it. */
export const assetSetCover = (nodeId: string, assetId: string | null) =>
  call<WobuNode>('asset_set_cover', { nodeId, assetId })

/* ── influence ────────────────────────────────────────────────────────────── */

/**
 * Where a fragment is routed once a generation is compiled.
 *
 * `moodboard_only` is the one that matters: it is shown to the human and never
 * sent anywhere. It appears on a layer card and in nothing `promptCompile`
 * returns — see `InfluenceFragment.sendable`.
 */
export type FragmentTarget =
  'prompt' | 'negative' | 'style_ref' | 'structure_ref' | 'palette' | 'moodboard_only'

/**
 * How a source got into the stack, and therefore what the layer card says about
 * why it is there. A card that cannot answer that is unarguable — the user sees
 * a culture they did not expect and has nowhere to go.
 */
export type Reached =
  | 'subject'
  /** A project singleton seeded into every stack: the Style Guide, the World Bible. */
  | 'root'
  | { link: LinkRole }
  /** Followed `parentId`, the implicit link of weight 1.0. */
  | 'parent'
  /** The shot controls, which are not part of the world. */
  | 'shot'

/** Why a fragment the user wrote is not in the compiled prompt. */
export type DropReason =
  /** Turned down to nothing — a slider at the bottom, or a link weighted to 0. */
  | 'silenced'
  /** It did not fit. The lightest go first, so this one was among the least weighted. */
  | 'budget'

/** How much an output preset cares about one description section. 1.0 is no opinion. */
export interface SectionPriority {
  section: string
  weight: number
}

export interface PresetView {
  /** Provider-ready tag recorded on the generation and sent with the mesh input. */
  viewType: string
  /** Camera instruction appended to this generation's Shot fragments. */
  framing: string
}

export interface ImageConstraints {
  mimeTypes: string[]
  minSide: number
  maxSide: number
  /** Whole named-view batch, before base64 encoding. */
  maxBatchBytes: number
}

/**
 * The recipe that turns one description into a particular kind of sheet.
 *
 * Returned whole by both commands rather than as an id: the panel needs the
 * aspect and image count to describe what Generate would do, and a round trip
 * for a static table would be a round trip for nothing.
 */
export interface Preset {
  id: string
  label: string
  kinds: NodeKind[]
  defaultFor: NodeKind[]
  priorities: SectionPriority[]
  /** The Shot layer's own text — pose, lighting, background, distance. */
  framing: string
  aspect: string
  images: number
  /** Tagged, framed views in emission order; empty when the batch just varies. */
  views: PresetView[]
  imageConstraints: ImageConstraints | null
}

/** Presets applicable to one kind, in registry order. */
export const presetList = (kind: NodeKind) => call<Preset[]>('preset_list', { kind })

/**
 * One thing one layer contributes.
 *
 * The same shape appears three times — a card's contributions, the spans in the
 * compiled prompt, and the drop report — because they are the same fragments
 * seen from three angles.
 */
export interface InfluenceFragment {
  layer: InfluenceLayer
  /** Null for the Shot layer, whose framing text comes from the preset. */
  nodeId: string | null
  sourceName: string
  /** A description section key, or a reference's role. */
  section: string
  /** Prose. Null for a reference image, which carries `assetId` instead. */
  text: string | null
  assetId: string | null
  /** `link.weight × section_priority × user_slider`, already multiplied out. */
  weight: number
  target: FragmentTarget
  /**
   * Whether this may be put in front of a provider. False only for
   * `moodboard_only`. Read it rather than re-deriving it from `target` — one
   * list of what is private is the whole point, and the direction a second one
   * fails in is somebody's mood board on a third party's servers.
   */
  sendable: boolean
}

/** One layer card. */
export interface LayerCard {
  layer: InfluenceLayer
  nodeId: string | null
  /** What the card is titled — the node's name, or the shot's label. */
  name: string
  kind: NodeKind | null
  reached: Reached
  /** Hops from whichever root reached this. The subject and the singletons are 0. */
  distance: number
  /** The product of the link weights along the path. Not the user's slider. */
  weight: number
  /** Where this card's slider sits, 0.0–1.0. */
  slider: number
  fragments: InfluenceFragment[]
}

/** The resolved stack for one subject, outermost layer first. */
export interface InfluenceStack {
  subjectId: string
  preset: Preset
  layers: LayerCard[]
}

export interface DroppedFragment {
  fragment: InfluenceFragment
  reason: DropReason
}

export interface CompiledPrompt {
  subjectId: string
  preset: Preset
  prompt: string
  negative: string
  /**
   * The fragments the two strings are made of, in emission order — what lets the
   * prompt box tint each span by where it came from. That attribution is the
   * main feedback loop for learning to write good upstream notes, not a debug
   * feature.
   */
  spans: InfluenceFragment[]
  /**
   * Everything left out, in reading order, so it can be walked alongside the
   * layer cards. The Inspector reports what was dropped rather than truncating
   * silently; a user who cannot see what was cut cannot learn to fix it.
   */
  dropped: DroppedFragment[]
  /**
   * Characters over budget, or null when it fits. Only ever set when the budget
   * could not fit even one fragment: the compiler keeps the heaviest and says
   * so, because an empty prompt is not a smaller picture, it is a different one
   * that still costs money.
   */
  overflow: number | null
}

/** Where one layer card's weight slider sits. */
export interface SliderSetting {
  nodeId: string
  /** 0.0–1.0. Clamped on the far side rather than refused. */
  value: number
  /** Removes the card for this run while preserving `value` for unmute. */
  muted?: boolean
}

/** The Shot layer — layer 7, the one the Inspector's own controls own. */
export interface ShotControls {
  /** What the Shot card is titled. Defaults to the preset's label. */
  label?: string
  weight?: number
  /** Extra framing typed for this run; unlike `label`, this is sent. */
  prompt?: string
}

/**
 * What one compilation may spend on text, in characters rather than tokens.
 *
 * Omit either and that pool is unlimited, which is the right answer until a
 * backend has been chosen — a limit invented here would drop fragments to fit a
 * number nobody measured. The two are metered separately because a request with
 * no negative prompt is ordinary and one with no positive prompt is a picture of
 * nothing that still costs money.
 */
export interface PromptBudget {
  promptChars?: number
  negativeChars?: number
}

/**
 * The resolved stack for a subject, with the per-layer detail the layer cards
 * read.
 *
 * Answers from the local index and never touches the project folder, so it is
 * as fast on a share that has just been unplugged as on a local disk. A project
 * with no Style Guide, or with none of the links the stack walks, resolves to a
 * short list rather than an error — that is the state every project is in on day
 * one. Rejects with `node.not_found` for a subject that is not there, which is
 * usually a panel still pointing at something a collaborator deleted.
 *
 * `shot` is optional and omitting it means there is no shot yet: no Shot card
 * appears, because nothing has been framed. `promptCompile` always has one.
 */
export const influenceResolve = (
  subjectId: string,
  options: {
    preset?: string
    sliders?: SliderSetting[]
    shot?: ShotControls
  } = {},
) => call<InfluenceStack>('influence_resolve', { subjectId, ...options })

/**
 * The compiled positive and negative prompt, the spans they are made of, and the
 * account of what did not make it.
 *
 * Called on every Inspector interaction — every slider drag, every preset
 * change — and does no file I/O at all, so it is cheap enough to run on each
 * one rather than behind a debounce.
 *
 * A preset the backend has never heard of falls back to the kind's default
 * rather than failing, because a generation record naming a preset since renamed
 * still has to open.
 */
export const promptCompile = (
  subjectId: string,
  options: {
    preset?: string
    sliders?: SliderSetting[]
    shot?: ShotControls
    budget?: PromptBudget
  } = {},
) => call<CompiledPrompt>('prompt_compile', { subjectId, ...options })

export interface GenerateOptions {
  preset?: string
  sliders?: SliderSetting[]
  shot?: ShotControls
  aspect?: string
  model?: string
  seed?: number
  grid?: VariantGrid
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

/**
 * A version of a node that lost a save race and was parked beside the winner.
 *
 * Both documents arrive whole rather than as a diff: how much context to show
 * and whether to fold the unchanged parts is a rendering decision, and `Conflict`
 * is a few kilobytes. `src/lib/diff.ts` does the aligning on this side.
 */
export interface Conflict {
  /** The sibling file, project-relative. Also the handle `conflictResolve` takes. */
  relPath: string
  /** The node file it was parked beside. */
  nodeRelPath: string
  /** Null when the node file has since been deleted or cannot be parsed. */
  nodeId: string | null
  nodeName: string | null
  /** Whose version was set aside, read back out of the filename. */
  user: string | null
  /** ISO 8601. Null for a sibling whose name nothing could parse. */
  savedAt: string | null
  /**
   * Whether the parked version carries *this* session's user name — the
   * difference between a card that says "keep mine" and one that says "keep
   * Nadia's". Only the wording depends on it.
   */
  mine: boolean
  /** The text that was set aside. */
  parked: string
  /** What is at `nodeRelPath` right now. Empty if the node file is gone. */
  current: string
  /**
   * Hash of `current` as the card rendered it. Handed straight back to
   * `conflictResolve`, which refuses if it no longer matches — that is what
   * stops a decision being applied to a version the user never read.
   */
  currentHash: string
}

/** Which version the user chose. There is deliberately no "merge". */
export type ConflictKeep = 'parked' | 'current'

/**
 * What came of a resolution. Only `done` changed anything on disk; `stale` and
 * `conflict` both left the two files exactly as they were.
 */
export type ConflictResolved =
  | { outcome: 'done' }
  /** A third writer moved the node file while the user was reading the diff. */
  | { outcome: 'stale' }
  /** The write lost a race of its own; our pick is parked as a further sibling. */
  | { outcome: 'conflict'; conflictPath: string }

export const conflicts = () => call<Conflict[]>('conflicts')

/**
 * Apply the user's decision, and delete the version they rejected.
 *
 * The only call in the app that removes a conflict sibling, which is why it is
 * only ever reached from a button. `expectedHash` is `Conflict.currentHash`
 * unchanged — passing anything else defeats the staleness check that stops a
 * third writer's version being discarded unseen.
 */
export const conflictResolve = (relPath: string, keep: ConflictKeep, expectedHash: string) =>
  call<ConflictResolved>('conflict_resolve', { relPath, keep, expectedHash })

/* ── presence ─────────────────────────────────────────────────────────────── */

/**
 * Someone else with this project open.
 *
 * Advisory, and only advisory. Nothing here reserves a node or refuses a save —
 * hard locks over a share strand files whenever a laptop sleeps or a VPN drops,
 * and the recovery is worse than the collision. A peer editing the node you are
 * in is a banner, never a block. See `docs/07-file-shares.md`.
 */
export interface Peer {
  /** ULID of their session, not of a node. Stable for as long as they stay. */
  sessionId: string
  user: string
  /** Best effort — reads `unknown` on platforms that do not hand it over. */
  host: string
  /**
   * Seconds since their heartbeat file was last written, measured by *this*
   * machine's clock against the file's mtime.
   *
   * Deliberately not a timestamp. Two machines on a LAN routinely sit an hour
   * apart, so a duration computed on this side from a time the other side wrote
   * would be wrong by exactly that much, and nothing here could tell.
   */
  seenSecsAgo: number
  /** Node ids they have open. Render it; do not act on it. */
  editing: string[]
}

/**
 * Who else has this project open.
 *
 * Poll it — there is no event, because the answer only changes at human speed
 * and pushing one per heartbeat per peer would be traffic on the same share the
 * presence is describing. Empty when no project is open, rather than an error.
 */
export const presencePeers = () => call<Peer[]>('presence_peers')

/**
 * Tell everyone else which nodes this session has open.
 *
 * The whole list every time, not a delta: this side already knows exactly which
 * nodes are open, and a delta would drift the first time one closed while the
 * share was away.
 */
export const presenceEditing = (nodeIds: string[]) => call<void>('presence_editing', { nodeIds })

/* ── provider keys ────────────────────────────────────────────────────────── */

/**
 * Where a provider's key came from.
 *
 * `environment` only ever appears in a development build — a repo-root `.env`,
 * or a variable exported into the process. A shipped Wobu reads keys from the
 * OS keychain and from nowhere else.
 */
export type KeySource = 'keychain' | 'environment'

/**
 * Whether this computer has a credential store that answers.
 *
 * `unavailable` is not a failure: a headless Linux box or a session whose login
 * keyring is locked has no store, and the app still runs. What it means is that
 * a key cannot be *saved* here, which is worth saying beside the field rather
 * than discovering when Save fails.
 */
export type KeychainState = 'ready' | 'unavailable'

/**
 * Presence, never value.
 *
 * Keys live in the Rust process and in this machine's keychain; none of them
 * ever crosses the bridge, which is why there is no field here that could carry
 * one. Keys are per *installation*, so a project shared from a drive opens with
 * *your* keys — `project.json` records only which provider and model were
 * chosen, and a collaborator without a key gets `source: null` rather than an
 * error.
 */
export interface KeyStatus {
  provider: string
  /** `null` means no key on this machine. A state, not a failure. */
  source: KeySource | null
  keychain: KeychainState
}

/** Takes a list because a providers pane renders every row at once. */
export const providerKeyStatus = (providers: string[]) =>
  call<KeyStatus[]>('provider_key_status', { providers })

/**
 * The one call that carries key material, and it carries it *inwards*: the user
 * pasted it into a field, so it is already in the webview and the only question
 * is where it goes next. Nothing sends one back.
 *
 * Rejects with `provider.keychain_unavailable` when there is no store to save
 * into — the message says to unlock the login keyring.
 */
export const providerKeySet = (provider: string, key: string) =>
  call<KeyStatus>('provider_key_set', { provider, key })

export interface KeyRemoval {
  /** False when there was nothing stored. A no-op, not a failure. */
  removed: boolean
  /**
   * What the provider resolves to now. On a development build this can still be
   * configured after a successful delete, because the repo-root `.env` answers
   * next — which is the one outcome worth showing rather than assuming.
   */
  status: KeyStatus
}

export const providerKeyDelete = (provider: string) =>
  call<KeyRemoval>('provider_key_delete', { provider })

/* ── the capability probe ─────────────────────────────────────────────────── */

/** What a probe was charged, as the provider reported it. */
export interface ProbeUsage {
  inputTokens: number
  cachedInputTokens: number
  outputTokens: number
}

/**
 * What checking a key found out.
 *
 * A rejected key arrives here as `ok: false`, not as a rejected promise. It is
 * the answer the pane asked for and belongs beside the field that caused it —
 * a toast would put "Anthropic says this key is wrong" somewhere the key is no
 * longer on screen.
 */
export interface ProbeResult {
  provider: string
  /** The model asked about — the adapter's own default when none was passed. */
  model: string
  ok: boolean
  /** One sentence. On success it says what was proved, not just "OK". */
  message: string
  /** `null` when the probe passed. */
  code: ErrorCode | null
  usage: ProbeUsage
}

/**
 * Check a stored key against the provider it belongs to.
 *
 * Cheap by construction rather than by promise: the backend asks for one
 * description and cuts the answer off after a couple of dozen tokens, because
 * everything worth knowing — the key is accepted, the model id resolves, the
 * schema is one this provider will take — is settled before the first sentence
 * is finished. A refused key is never billed at all.
 *
 * Rejects only when the probe could not run: no key on this machine
 * (`provider.no_key`), or a provider this build has no adapter for.
 */
export const providerProbe = (provider: string, model?: string) =>
  call<ProbeResult>('provider_probe', { provider, model })

/* ── machine-local provider settings ────────────────────────────────────── */

/** Installation settings. Deliberately separate from project provider selections. */
export interface MachineSettings {
  comfyuiEndpoint: string
}

export type ComfyEndpointState =
  'connected' | 'unreachable' | 'authentication_required' | 'incompatible'

export interface ComfyEndpointProbe {
  endpoint: string
  state: ComfyEndpointState
  ok: boolean
  message: string
}

/** Read the route stored under this installation's application-data directory. */
export const machineSettings = () => call<MachineSettings>('machine_settings')

/** Validate and persist a ComfyUI route on this machine, never in project.json. */
export const comfyuiEndpointSet = (endpoint: string) =>
  call<MachineSettings>('comfyui_endpoint_set', { endpoint })

/** Probe a draft route without changing which route generation uses. */
export const comfyuiEndpointProbe = (endpoint?: string) =>
  call<ComfyEndpointProbe>('comfyui_endpoint_probe', {
    ...(endpoint === undefined ? {} : { endpoint }),
  })

/* ── the provider selection ───────────────────────────────────────────────── */

/**
 * The three jobs a provider can be chosen for, selected independently.
 *
 * Not one setting: enhancing with Gemini, generating on a ComfyUI running
 * downstairs and meshing through Hunyuan3D is the ordinary combination
 * (`docs/08-providers.md`), and a single provider field cannot express it.
 */
export type Capability = 'text' | 'image' | 'mesh'

/**
 * One capability's entry in `project.json`.
 *
 * All fields are optional because each has a meaningful absence: no `provider`
 * is "nobody has chosen", no `model` is "whatever the adapter's default is",
 * and no `region` means a hosted Hunyuan3D selection is not ready to run.
 */
export interface ProviderSelection {
  provider?: string
  model?: string
  /** Tencent Hunyuan3D only; kept beside the provider because submit and poll must agree. */
  region?: string
}

/**
 * The shared half of the providers pane.
 *
 * This is what `project.json` says, so it is what *everyone* who opens the
 * folder sees — the counterpart to `KeyStatus`, which is what only this machine
 * has. Keeping them as two separate shapes is deliberate: they have different
 * lifetimes, different owners, and merging them into one "provider is ready"
 * flag would erase exactly the distinction a collaborator needs.
 *
 * Keyed loosely because a project written by a newer Wobu may carry a
 * capability this build has never heard of, and the backend round-trips the map
 * rather than parsing it into three fields.
 */
export interface ProviderSelections {
  providers: Record<string, ProviderSelection | undefined>
  /**
   * Whether the *selection* can be changed. Keys are unaffected — they are per
   * installation — so a read-only world is still one you can add a key for.
   */
  readOnly: boolean
}

export const projectProviders = () => call<ProviderSelections>('project_providers')

/**
 * Choose a provider for one capability and write it into `project.json`.
 *
 * Merged rather than replaced on the Rust side, so default params set by another
 * build survive a change of provider. Passing no `model` clears the model, which
 * is how "use the adapter's default" is spelled. Omitting `region` leaves it
 * unchanged; only the explicit Hunyuan region picker sends one.
 *
 * Rejects with `write.read_only` on a read-only folder.
 */
export const projectProviderSelect = (
  capability: Capability,
  provider: string,
  model?: string,
  region?: string,
) =>
  call<ProviderSelections>('project_provider_select', {
    capability,
    provider,
    model,
    ...(region === undefined ? {} : { region }),
  })

/** A provider/model pair after backend defaults have been resolved. */
export interface ActiveModel {
  provider: string
  label: string
  model: string
  contextTokens: number | null
}

export type BackendHealth =
  | { state: 'connected'; externalQueue: number | null }
  | { state: 'unavailable'; detail: string }
  | { state: 'unconfigured'; detail: string }
  | { state: 'unsupported'; detail: string }

export interface StatusBarBackend {
  image: ActiveModel | null
  text: ActiveModel
  health: BackendHealth
}

/** Selected models plus a non-generating reachability check of the image backend. */
export const statusBarBackend = () => call<StatusBarBackend>('status_bar_backend')

/* ── storage and about ────────────────────────────────────────────────────── */

/** The local SQLite index for the open project. Disposable by design. */
export interface IndexInfo {
  path: string
  sizeBytes: number
  nodeCount: number
}

export const indexInfo = () => call<IndexInfo>('index_info')

/** Throw the index away and rebuild it from the Markdown. */
export const indexRebuild = () => call<void>('index_rebuild')

export interface AboutInfo {
  appVersion: string
  /** The on-disk format of the project folder. */
  projectSchemaVersion: number
  /** The local index layout; a bump silently rebuilds on next open. */
  indexSchemaVersion: number
  logPath: string
}

export const aboutInfo = () => call<AboutInfo>('about_info')

/* ── diagnostics ──────────────────────────────────────────────────────────── */

/**
 * Least to most verbose. `off` records nothing at all, errors included — it is
 * there for someone who wants the file to stop existing, not as a default.
 */
export type LogLevel = 'off' | 'error' | 'warn' | 'info' | 'debug'

export const LOG_LEVELS: LogLevel[] = ['off', 'error', 'warn', 'info', 'debug']

export interface LogInfo {
  /** Absolute. Shown to the user, who may well go and find it by hand. */
  path: string
  level: LogLevel
  /** False until something has been recorded — there may be nothing to reveal. */
  exists: boolean
  sizeBytes: number
}

export const logInfo = () => call<LogInfo>('log_info')

export const logSetLevel = (level: LogLevel) => call<void>('log_set_level', { level })

/** The end of the log, so the user can read it before handing it over. */
export const logTail = (lines: number) => call<string>('log_tail', { lines })

/** Show it in the OS file manager, which is how it gets attached to something. */
export const logReveal = () => call<void>('log_reveal')

/* ── jobs ─────────────────────────────────────────────────────────────────── */

/**
 * Everything long-running is a job: it returns an id immediately and reports
 * itself over the events below. Nothing on this side ever waits for one.
 *
 * The shapes here mirror `wobu-jobs` exactly — see `src-tauri/crates/wobu-jobs`,
 * where the reasoning for each of them is written down.
 */
export type JobKind = 'enhance' | 'generate' | 'train_lora' | 'mesh' | 'thumbnail'

/**
 * Whether the attempt that failed cost the user money.
 *
 * This is the field that decides whether the queue retried on its own. It never
 * retries a `charged` or `unknown` failure without being told to in advance,
 * because that would be spending someone's money on a hunch.
 */
export type Billed = 'nothing' | 'charged' | 'unknown'

export interface JobFailure {
  /** The same dotted codes command errors use, so `errorSurface` fits both. */
  code: string
  message: string
  /** Whether another attempt could work. Not whether it should — see `billed`. */
  retryable: boolean
  detail?: string
  /** The provider's own wait hint, in milliseconds. */
  retryAfter?: number
  billed: Billed
  /** What the failed attempt cost, in the backend's words. The queue cannot
   *  price a call, so this is the only thing that says what "again" means. */
  costNote?: string
}

/**
 * Where a job is. Flat rather than nested so a component switches on one field.
 *
 * `retryHeld` on a failure is the interesting one: the job *can* be retried and
 * the queue would not do it, because the attempt was billed. That is an offer to
 * put in front of the user, not an apology.
 */
export type JobState =
  | { state: 'queued' }
  | { state: 'running' }
  | { state: 'retrying'; inMs: number; costsMoney: boolean }
  | { state: 'done' }
  | { state: 'cancelled' }
  | { state: 'failed'; failure: JobFailure; retryHeld: boolean }

export type JobSnapshot = {
  id: string
  kind: JobKind
  label: string
  /** Domain subject, when the task has one. Generate jobs use their node id. */
  subjectId: string | null
  /** Attempts started so far, from 1. Zero while queued. */
  attempt: number
  /** Backend-measured time since the first attempt, frozen at completion. */
  elapsedMs: number
} & JobState

/**
 * The whole queue, sent on every transition. It includes a bounded tail of
 * finished jobs, so the last outcome is still on screen after it happened.
 */
export interface QueueSnapshot {
  jobs: JobSnapshot[]
  queued: number
  running: number
  retrying: number
}

/** Everything that has not finished — the number a status bar shows. */
export function jobDepth(snapshot: QueueSnapshot): number {
  return snapshot.queued + snapshot.running + snapshot.retrying
}

export interface JobProgress {
  id: string
  done: number
  total: number
  /** A backend's own words for the step — "sampling 12/30". */
  note?: string
}

export interface JobPreview {
  id: string
  /** Opaque on purpose: how a latent preview reaches here is #40's decision. */
  image: string
  step?: number
}

export interface JobDone {
  id: string
  kind: JobKind
  label: string
  /** Whatever the job decided its caller needs; absent when the result is on disk. */
  result?: unknown
}

export interface JobFailed {
  id: string
  kind: JobKind
  label: string
  failure: JobFailure
  retryHeld: boolean
}

/**
 * The event names, mirrored from `wobu_jobs::events`.
 *
 * There is deliberately no `job:cancelled` — a user who pressed Stop does not
 * need to be told it stopped, and `job:state` carries it for anything drawing
 * the queue.
 */
export const JOB_EVENTS = {
  state: 'job:state',
  progress: 'job:progress',
  preview: 'job:preview',
  retry: 'job:retry',
  done: 'job:done',
  error: 'job:error',
} as const

/**
 * Stop a job. `false` if there is no such job or it had already finished.
 *
 * Returns as soon as the backend has been told, not when the work has stopped:
 * a job that had not started is over immediately, and one in flight is aborted
 * within the grace it gets to report what it was charged. Cancelling something
 * that has already finished is not an error — the user can press Stop at the
 * instant a job ends, and that race is ordinary.
 */
export const jobCancel = (jobId: string) => call<boolean>('job_cancel', { jobId })

/**
 * The queue as it stands. The `job:state` event is the live signal; this covers
 * the one case it cannot — a webview that reloaded mid-generation.
 */
export const jobList = () => call<QueueSnapshot>('job_list')

/* ── enhance ──────────────────────────────────────────────────────────────── */

/**
 * A structured description, in the one shape it crosses the bridge in. The same
 * type `WobuNode.description` holds, deliberately: a half-written description
 * and a finished one are rendered by the same component.
 */
export interface WobuDescription {
  sections: Record<string, SectionValue>
}

/**
 * The document so far, repainted as it arrives.
 *
 * Whole snapshots rather than appends. Events are fire-and-forget, so a pane
 * that accumulated fragments would be permanently wrong the first time one was
 * dropped — and wrong in a way that reads as the model having written nonsense.
 * Draw whatever the last one of these said and nothing else.
 *
 * Sections arrive in the kind's declared order, and one that has opened but
 * streamed nothing is present and empty, so its heading can appear before its
 * text does. **None of this is saved anywhere.** It is display state; what a
 * node ends up holding is whatever `enhanceAccept` is given.
 */
export interface EnhanceDelta {
  jobId: string
  nodeId: string
  description: WobuDescription
  questions: string[]
}

/** Mirrored from `src-tauri/src/enhance.rs`. */
export const ENHANCE_DELTA = 'enhance:delta'

/**
 * A finished description waiting for an answer — a whole, schema-valid one,
 * which is the only kind there is.
 *
 * Arrives twice over: as the `result` on an Enhance's `job:done`, and from
 * `enhancePending` afterwards. One shape rather than two, because they are the
 * same thing seen a moment apart.
 *
 * `questions` is what the model would otherwise have had to invent, asked
 * instead. It is not part of the description, is never written to the node, and
 * never reaches an image model: it is addressed to whoever wrote the notes.
 */
export interface EnhanceReady {
  /** The id to pass to `enhanceAccept` or `enhanceDiscard`. */
  jobId: string
  nodeId: string
  description: WobuDescription
  questions: string[]
}

/**
 * Everything still waiting to be accepted, for the open project.
 *
 * The catch-up read for the one case `job:done` cannot cover — a webview that
 * reloaded after it fired. That case matters more here than anywhere else in the
 * app, because what was lost has already been *paid for*: without this, a reload
 * mid-review means running the call again to recover an answer the backend is
 * still holding.
 *
 * The whole list, because after a reload there is no job id to look one up by —
 * match on `nodeId` and answer with `jobId`. At most a handful of entries, and
 * empty when nothing is open.
 */
export const enhancePending = () => call<EnhanceReady[]>('enhance_pending')

/**
 * What `enhanceAccept` did.
 *
 * `refusedEdit` is a result, not a failure. The description on disk was written
 * by hand and nobody has said to replace it, so the node comes back untouched
 * and the right response is to show the user what is about to be overwritten and
 * ask — then call again with `force`. A *conflict* is different and arrives as a
 * rejection with `write.conflict`, the same as any other lost save race.
 */
export type EnhanceAccepted =
  { outcome: 'saved'; node: WobuNode } | { outcome: 'refusedEdit'; node: WobuNode }

/**
 * Start an Enhance. Resolves with a job id before any of the work happens.
 *
 * Rejects, without spending anything, when this machine has no key for the
 * provider the project selected (`provider.no_key`, not retryable — the answer
 * is to paste a key in Settings, not to press the button again), when the
 * project is read-only, or when the node is gone. Everything after that is the
 * queue's: watch `job:state` for the job, `enhance:delta` for the text, and
 * `job:done` for an `EnhanceReady`.
 *
 * Stopping it is `jobCancel`, like any other job. That aborts the request rather
 * than discarding its answer, so a Stop actually stops the meter.
 */
export const enhanceStart = (nodeId: string) => call<string>('enhance_start', { nodeId })

/**
 * Write a finished description to its node, stamping the upstream versions it
 * was built from.
 *
 * `description` is what the user is accepting — pass an edited one to save the
 * edit, or omit it for exactly what the model sent. `force` answers a previous
 * `refusedEdit` and means nothing else.
 */
export const enhanceAccept = (jobId: string, description?: WobuDescription, force?: boolean) =>
  call<EnhanceAccepted>('enhance_accept', { jobId, description, force })

/** Reject one. Not an error when there is nothing left to reject. */
export const enhanceDiscard = (jobId: string) => call<void>('enhance_discard', { jobId })
