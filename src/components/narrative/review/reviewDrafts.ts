import { create } from 'zustand'
import { setNarrativeDraftGuard } from '../../../lib/narrativeDraftGuard'
import type { ReviewProposal, ReviewRequest } from '../../../lib/api/narrativeReview'

export interface ReviewDraft {
  authorization: Omit<ReviewRequest, 'action'>
  body: string
  originalBody: string
  currentBody: string
  currentRevision: string | null
  proposal: ReviewProposal | null
}

/** Keep the first reviewed guard until explicit save/discard, including after peer refreshes. */
export const useReviewDrafts = create<{
  drafts: Record<string, ReviewDraft>
  edit: (key: string, initial: ReviewDraft, body: string) => void
  clear: (key: string, expected?: ReviewDraft) => void
}>((set) => ({
  drafts: {},
  edit: (key, initial, body) =>
    set((state) => {
      if (!state.drafts[key] && Object.keys(state.drafts).length >= 100) {
        throw new Error('Save or discard a review draft before opening more than 100 edited lines.')
      }
      setNarrativeDraftGuard(`review:${key}`, true)
      return { drafts: { ...state.drafts, [key]: { ...(state.drafts[key] ?? initial), body } } }
    }),
  clear: (key, expected) =>
    set((state) => {
      if (expected && state.drafts[key] !== expected) return state
      const { [key]: removed, ...drafts } = state.drafts
      if (removed) setNarrativeDraftGuard(`review:${key}`, false)
      return { drafts }
    }),
}))
