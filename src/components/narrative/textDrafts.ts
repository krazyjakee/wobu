import { create } from 'zustand'
import type { TextAsset, TextFile } from '../../lib/api/narrativeText'
import { setNarrativeDraftGuard } from '../../lib/narrativeDraftGuard'

export interface TextDraft {
  file: TextFile
  asset: TextAsset
}
export const textDraftKey = (project: string, asset: string) => `${project}:${asset}`

/** Buffers survive library navigation and participate in the shared project/quit guard. */
export const useTextDrafts = create<{
  drafts: Record<string, TextDraft>
  put: (key: string, file: TextFile, asset: TextAsset) => void
  clear: (key: string, expected?: TextDraft) => void
}>((set, get) => ({
  drafts: {},
  put: (key, file, asset) => {
    const original = get().drafts[key]?.file ?? file
    if (JSON.stringify(original.asset) === JSON.stringify(asset)) {
      get().clear(key)
      return
    }
    setNarrativeDraftGuard(`text:${key}`, true)
    set((state) => ({ drafts: { ...state.drafts, [key]: { file: original, asset } } }))
  },
  clear: (key, expected) => {
    if (expected && get().drafts[key] !== expected) return
    setNarrativeDraftGuard(`text:${key}`, false)
    set((state) => {
      const drafts = { ...state.drafts }
      delete drafts[key]
      return { drafts }
    })
  },
}))
