import { create } from 'zustand'
import type { MediaNotes, MediaPolicy } from '../../../lib/api/narrativeMedia'
import { setNarrativeDraftGuard } from '../../../lib/narrativeDraftGuard'
export interface NotesDraft {
  notes: MediaNotes
  policy: MediaPolicy
  guard: string
}
export const useMediaNotes = create<{
  entries: Record<string, NotesDraft>
  update: (key: string, value: NotesDraft | null) => void
}>((set) => ({
  entries: {},
  update: (key, value) => {
    setNarrativeDraftGuard(`recording:${key}`, value !== null)
    set(({ entries }) => {
      const next = { ...entries }
      if (value) next[key] = value
      else delete next[key]
      return { entries: next }
    })
  },
}))
