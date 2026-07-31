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
  coverAssetId: string | null
  links: Link[]
  createdAt: string
  updatedAt: string
}

/* ── environment ──────────────────────────────────────────────────────────── */

/** True when running inside the Tauri webview (as opposed to a bare `vite dev`). */
export const isTauri = (): boolean =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/** Normalises the assorted things a Rust command error can arrive as. */
export function errorMessage(e: unknown): string {
  if (typeof e === 'string') return e
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

export const projectRecent = () => call<ProjectSummary[]>('project_recent')

export const projectCurrent = () => call<ProjectSummary | null>('project_current')

export const projectClose = () => call<void>('project_close')

export const nodeList = () => call<NodeSummary[]>('node_list')

export const nodeGet = (id: string) => call<WobuNode>('node_get', { id })

export const nodeCreate = (kind: NodeKind, name: string, parentId: string | null) =>
  call<WobuNode>('node_create', { kind, name, parentId })

export const nodeUpsert = (node: WobuNode) => call<WobuNode>('node_upsert', { node })

export const nodeDelete = (id: string) => call<void>('node_delete', { id })

export const nodeMove = (id: string, newParentId: string | null) =>
  call<void>('node_move', { id, newParentId })
