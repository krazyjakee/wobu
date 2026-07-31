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

/** `key` indexes `WobuNode.description.sections`; `label` is the rendered heading. */
export interface SectionDef {
  key: string
  label: string
  valueKind: SectionValueKind
}

export type InfluenceLayer =
  | 'style'
  | 'world'
  | 'ancestry'
  | 'culture'
  | 'place'
  | 'subject'
  | 'shot'

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
  | 'silhouette'
  | 'palette'
  | 'material'
  | 'mood'
  | 'pose'
  | 'costume'
  | 'full_ref'

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
  | 'node.not_found'
  | 'node.malformed'
  | 'node.invalid'
  | 'write.conflict'
  | 'write.read_only'
  | 'asset.not_an_image'
  | 'asset.not_found'
  | 'share.unmounted'
  | 'provider.no_key'
  | 'provider.bad_key'
  | 'provider.billing_required'
  | 'provider.rate_limited'
  | 'provider.unavailable'
  | 'provider.bad_response'
  | 'provider.context_too_long'
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
 * A read-only folder detected at open is likewise not a banner — the title
 * bar carries a `read-only` chip and the write controls disable themselves.
 * `write.read_only` reaching here means the folder changed under a session
 * that started writable, which the chip alone would not explain.
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

export const projectCurrent = () => call<ProjectSummary | null>('project_current')

export const projectClose = () => call<void>('project_close')

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

export const nodeList = () => call<NodeSummary[]>('node_list')

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

export const nodeDelete = (id: string) => call<void>('node_delete', { id })

export const nodeMove = (id: string, newParentId: string | null) =>
  call<void>('node_move', { id, newParentId })

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

/** The same, for a paste or a drop that arrived as bytes rather than a path. */
export const assetImportBytes = (bytes: Uint8Array, kind: AssetKind = 'reference') =>
  call<ImportedAsset>('asset_import_bytes', { bytes: Array.from(bytes), kind })

/** Every blob in the open project, newest first. */
export const assetList = () => call<Asset[]>('asset_list')

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
export const assetLink = (
  nodeId: string,
  assetId: string,
  role: AssetRole,
  weight?: number,
) => call<WobuNode>('asset_link', { nodeId, assetId, role, weight })

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

/**
 * Change a link's weight, its enabled flag, or both.
 *
 * Omit either to leave it alone. Sending the whole link from a slider would let
 * its stale copy of `enabled` undo a mute the user just applied.
 */
export const assetLinkUpdate = (
  nodeId: string,
  assetId: string,
  role: AssetRole,
  patch: { weight?: number; enabled?: boolean },
) => call<WobuNode>('asset_link_update', { nodeId, assetId, role, ...patch })

/** Choose the image on a node's card, or pass `null` to clear it. */
export const assetSetCover = (nodeId: string, assetId: string | null) =>
  call<WobuNode>('asset_set_cover', { nodeId, assetId })

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
export const presenceEditing = (nodeIds: string[]) =>
  call<void>('presence_editing', { nodeIds })

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
