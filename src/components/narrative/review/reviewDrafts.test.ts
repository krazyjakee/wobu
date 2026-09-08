import { beforeEach, expect, it } from 'vitest'
import { editorWrites } from '../../../lib/editorWrites'
import { resetNarrativeDraftGuards } from '../../../lib/narrativeDraftGuard'
import { useReviewDrafts, type ReviewDraft } from './reviewDrafts'

const draft: ReviewDraft = {
  authorization: {
    guard: { stamp: null, head: 'reviewed-head' },
    target: { scene: 'scene', beat: 'beat', slot: 'slot', variant: 'variant' },
    context_revision: 'reviewed-context',
    state_json: '{}',
  },
  body: 'Proposal.',
  originalBody: 'Proposal.',
  currentBody: 'Current.',
  currentRevision: 'revision',
  proposal: null,
}
beforeEach(() => {
  resetNarrativeDraftGuards()
  editorWrites.reset()
  useReviewDrafts.setState({ drafts: {} })
})
it('keeps reviewed revisions and writing when refreshed data arrives, and blocks project close', async () => {
  const store = useReviewDrafts.getState()
  store.edit('project:line', draft, 'My first edit.')
  store.edit(
    'project:line',
    {
      ...draft,
      authorization: { ...draft.authorization, guard: { stamp: null, head: 'peer-head' } },
    },
    'My second edit.',
  )
  expect(useReviewDrafts.getState().drafts['project:line']?.authorization.guard.head).toBe(
    'reviewed-head',
  )
  expect(useReviewDrafts.getState().drafts['project:line']?.body).toBe('My second edit.')
  await expect(editorWrites.flushAll()).rejects.toThrow()
  store.clear('project:line')
  await expect(editorWrites.flushAll()).resolves.toBeUndefined()
})
it('does not discard newer typing when an older acceptance finishes', () => {
  const store = useReviewDrafts.getState()
  store.edit('project:line', draft, 'First edit.')
  const submitting = useReviewDrafts.getState().drafts['project:line']!
  store.edit('project:line', draft, 'Newer edit.')
  store.clear('project:line', submitting)
  expect(useReviewDrafts.getState().drafts['project:line']?.body).toBe('Newer edit.')
  store.clear('another-project:line')
  expect(editorWrites.snapshot()).toHaveLength(1)
})
