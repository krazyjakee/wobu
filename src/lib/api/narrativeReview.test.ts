import { beforeEach, expect, it, vi } from 'vitest'
import {
  narrativeReviewApply,
  narrativeReviewContext,
  narrativeReviewGet,
  narrativeReviewList,
  narrativeReviewBatch,
  type ReviewRequest,
} from './narrativeReview'
const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue({})
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})
it('passes original raw state and guarded actions unchanged across the command boundary', async () => {
  const target = { scene: 'scene', beat: 'beat', slot: 'slot', variant: 'variant' }
  const state = '{"trust":9007199254740991.1}'
  await narrativeReviewGet('scene', state)
  expect(invoke).toHaveBeenLastCalledWith('narrative_review_get', {
    sceneId: 'scene',
    stateJson: state,
  })
  await narrativeReviewContext(target, state)
  expect(invoke).toHaveBeenLastCalledWith('narrative_review_context', { target, stateJson: state })
  const request: ReviewRequest = {
    guard: { stamp: { mtime_ms: 1, size: 2, hash: 'original-scene' }, head: 'original-head' },
    target,
    context_revision: 'reviewed-context',
    state_json: state,
    action: {
      kind: 'accept',
      proposal_id: 'proposal',
      proposal_hash: 'proposal-hash',
      reviewed_text: 'A careful revision.',
    },
  }
  await narrativeReviewApply(request)
  expect(invoke).toHaveBeenLastCalledWith('narrative_review_apply', { request })
  expect(request.guard.head).toBe('original-head')
  await narrativeReviewList(state, 32, 'catalog-guard')
  expect(invoke).toHaveBeenLastCalledWith('narrative_review_list', {
    stateJson: state,
    offset: 32,
    expectedCatalog: 'catalog-guard',
  })
  await narrativeReviewBatch([request], false)
  expect(invoke).toHaveBeenLastCalledWith('narrative_review_batch', {
    requests: [request],
    commit: false,
  })
})
