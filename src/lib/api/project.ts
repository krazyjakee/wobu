import { call } from './call'
import type { KindDef, NodeKind, ProjectSummary } from './model'
/* ── domain types ─────────────────────────────────────────────────────────── */

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
