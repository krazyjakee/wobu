import { create } from 'zustand'
import type { Scene, SceneFile } from '../../lib/api'
import { setNarrativeDraftGuard } from '../../lib/narrativeDraftGuard'

export interface ScriptDraft {
  file: SceneFile
  scene: Scene
}

/** Drafts survive tab changes; their original stamp still guards the eventual save. */
export const useScriptDrafts = create<{
  drafts: Record<string, ScriptDraft>
  put: (key: string, draft: ScriptDraft) => void
  clear: (key: string) => void
}>((set) => ({
  drafts: {},
  put: (key, draft) => {
    setNarrativeDraftGuard(`script:${key}`, true)
    set((state) => ({ drafts: { ...state.drafts, [key]: draft } }))
  },
  clear: (key) =>
    set((state) => {
      setNarrativeDraftGuard(`script:${key}`, false)
      const drafts = { ...state.drafts }
      delete drafts[key]
      return { drafts }
    }),
}))
