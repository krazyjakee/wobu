import { create } from 'zustand'
import { setNarrativeDraftGuard } from '../../lib/narrativeDraftGuard'
import type { WorldFile, WorldDocument } from '../../lib/api/narrativeWorld'
import type { StateFile, StateDocument } from '../../lib/api'

export interface WorldDraft {
  file: WorldFile
  document: WorldDocument
}
export interface VariablesDraft {
  file: StateFile
  document: StateDocument
}
interface Drafts {
  world: Record<string, WorldDraft>
  variables: Record<string, VariablesDraft>
  putWorld: (key: string, draft: WorldDraft | null) => void
  putVariables: (key: string, draft: VariablesDraft | null) => void
}
export const useWorldDrafts = create<Drafts>()((set) => ({
  world: {},
  variables: {},
  putWorld: (key, draft) => {
    setNarrativeDraftGuard(`world:${key}`, !!draft)
    set((s) => {
      const world = { ...s.world }
      if (draft) world[key] = draft
      else delete world[key]
      return { world }
    })
  },
  putVariables: (key, draft) => {
    setNarrativeDraftGuard(`variables:${key}`, !!draft)
    set((s) => {
      const variables = { ...s.variables }
      if (draft) variables[key] = draft
      else delete variables[key]
      return { variables }
    })
  },
}))
