import { create } from 'zustand'
import type { Scene, SceneFile } from '../../lib/api'
import { setNarrativeDraftGuard } from '../../lib/narrativeDraftGuard'

export const sceneEditKey = (projectKey: string, sceneId: string) => `${projectKey}:${sceneId}`
const MAX_UNDO_STEPS = 100
const MAX_UNDO_BYTES = 4 * 1024 * 1024
let revision = 0

type Checkpoint = { scene: Scene; bytes: number }
export interface ScriptDraft {
  /** Original guarded source and complete working document, shared by all scene editors. */
  file: SceneFile
  scene: Scene
  revision: number
  pending: boolean
  incoming: SceneFile | null
  error: string | null
  conflictPath: string | null
  past: Checkpoint[]
  future: Checkpoint[]
  historyTruncated: boolean
  coalesceKey: string | null
  editedAt: number
}
export interface SceneEditOptions {
  coalesceKey?: string
  now?: number
}
const checkpoint = (scene: Scene): Checkpoint => ({
  scene,
  bytes: JSON.stringify(scene).length * 2,
})
const sameFile = (a: SceneFile, b: SceneFile) =>
  a.stamp?.hash === b.stamp?.hash && JSON.stringify(a.scene) === JSON.stringify(b.scene)

function bounded(past: Checkpoint[]) {
  let bytes = past.reduce((sum, step) => sum + step.bytes, 0)
  let start = 0
  while (past.length - start > MAX_UNDO_STEPS || bytes > MAX_UNDO_BYTES) {
    bytes -= past[start++]!.bytes
  }
  return { past: past.slice(start), truncated: start > 0 }
}

/** One store: the original Script buffer now owns the scene's authoring session. */
export const useScriptDrafts = create<{
  drafts: Record<string, ScriptDraft>
  put: (
    key: string,
    draft: Pick<ScriptDraft, 'file' | 'scene'>,
    options?: SceneEditOptions,
  ) => boolean
  observe: (key: string, file: SceneFile) => void
  beginSave: (key: string) => ScriptDraft | null
  completeSave: (key: string, captured: number) => void
  failSave: (key: string, captured: number, error: string, conflictPath?: string | null) => void
  undo: (key: string) => void
  redo: (key: string) => void
  clear: (key: string) => void
}>((set, get) => {
  const travel = (key: string, redo: boolean) => {
    const draft = get().drafts[key]
    if (!draft || draft.pending) return
    const from = redo ? draft.future : draft.past
    const next = from.at(-1)
    if (!next) return
    const other = [...(redo ? draft.past : draft.future), checkpoint(draft.scene)]
    const limited = bounded(other)
    set((state) => ({
      drafts: {
        ...state.drafts,
        [key]: {
          ...draft,
          scene: next.scene,
          revision: ++revision,
          coalesceKey: null,
          past: redo ? limited.past : from.slice(0, -1),
          future: redo ? from.slice(0, -1) : limited.past,
          historyTruncated: draft.historyTruncated || limited.truncated,
        },
      },
    }))
  }
  return {
    drafts: {},
    put: (key, next, options = {}) => {
      const prior = get().drafts[key]
      if (prior?.pending) return false
      const before = prior?.scene ?? next.file.scene
      if (JSON.stringify(before) === JSON.stringify(next.scene)) return false
      const now = options.now ?? Date.now()
      const coalesces =
        prior &&
        options.coalesceKey &&
        prior.coalesceKey === options.coalesceKey &&
        now >= prior.editedAt &&
        now - prior.editedAt <= 750 &&
        !prior.future.length
      const history = bounded(coalesces ? prior.past : [...(prior?.past ?? []), checkpoint(before)])
      setNarrativeDraftGuard(`script:${key}`, true)
      set((state) => ({
        drafts: {
          ...state.drafts,
          [key]: {
            file: prior?.file ?? next.file,
            scene: next.scene,
            revision: ++revision,
            pending: false,
            incoming: prior?.incoming ?? null,
            error: prior?.error ?? null,
            conflictPath: prior?.conflictPath ?? null,
            past: history.past,
            future: [],
            historyTruncated: !!prior?.historyTruncated || history.truncated,
            coalesceKey: options.coalesceKey ?? null,
            editedAt: now,
          },
        },
      }))
      return true
    },
    observe: (key, file) => {
      const draft = get().drafts[key]
      if (!draft || sameFile(draft.incoming ?? draft.file, file)) return
      set((state) => ({ drafts: { ...state.drafts, [key]: { ...draft, incoming: file } } }))
    },
    beginSave: (key) => {
      const draft = get().drafts[key]
      if (!draft || draft.pending) return null
      set((state) => ({
        drafts: {
          ...state.drafts,
          [key]: {
            ...draft,
            pending: true,
            error: null,
            conflictPath: null,
            coalesceKey: null,
          },
        },
      }))
      return draft
    },
    completeSave: (key, captured) => {
      if (get().drafts[key]?.revision === captured) {
        setNarrativeDraftGuard(`script:${key}`, false)
        set((state) => {
          const drafts = { ...state.drafts }
          delete drafts[key]
          return { drafts }
        })
      }
    },
    failSave: (key, captured, error, conflictPath = null) => {
      const draft = get().drafts[key]
      if (draft?.revision !== captured) return
      set((state) => ({
        drafts: {
          ...state.drafts,
          [key]: {
            ...draft,
            pending: false,
            error,
            conflictPath,
          },
        },
      }))
    },
    undo: (key) => travel(key, false),
    redo: (key) => travel(key, true),
    clear: (key) => {
      if (get().drafts[key]?.pending) return
      setNarrativeDraftGuard(`script:${key}`, false)
      set((state) => {
        const drafts = { ...state.drafts }
        delete drafts[key]
        return { drafts }
      })
    },
  }
})
