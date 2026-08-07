import { call } from './call'
/* ── domain types ─────────────────────────────────────────────────────────── */

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
