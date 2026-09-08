import { beforeEach, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { ReviewBulk } from './ReviewBulk'
import { fixtureRow } from './reviewFixture.test-support'
import { useReviewDrafts } from './reviewDrafts'
import { resetNarrativeDraftGuards } from '../../../lib/narrativeDraftGuard'
const api = vi.hoisted(() => ({ narrativeReviewBatch: vi.fn() }))
vi.mock('../../../lib/api/narrativeReview', () => api)
beforeEach(() => {
  resetNarrativeDraftGuards()
  useReviewDrafts.setState({ drafts: {} })
  api.narrativeReviewBatch.mockReset()
})
it('freezes reviewed requests and requires another explicit review after context changes', async () => {
  const row = fixtureRow()
  api.narrativeReviewBatch.mockResolvedValue({
    items: [{ index: 0, target: row.line.target, status: 'eligible', reason: 'Eligible' }],
  })
  const view = render(
    <ReviewBulk rows={[row]} projectKey="/project" readOnly={false} onApplied={vi.fn()} />,
  )
  fireEvent.click(screen.getByRole('button', { name: 'Review 1 selected decisions' }))
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Apply 1 eligible decisions' })).toBeEnabled(),
  )
  const changed = { ...row, scene: { ...row.scene, guard: { stamp: null, head: 'peer-head' } } }
  view.rerender(
    <ReviewBulk rows={[changed]} projectKey="/project" readOnly={false} onApplied={vi.fn()} />,
  )
  expect(screen.getByRole('button', { name: 'Apply 1 eligible decisions' })).toBeDisabled()
  expect(screen.getByText(/original authorization has not been refreshed/)).toBeVisible()
  expect(api.narrativeReviewBatch).toHaveBeenCalledTimes(1)
  expect(api.narrativeReviewBatch.mock.calls[0]![0][0].guard.head).toBe('original-head')
})
it('skips hidden proposal drafts instead of approving their saved counterpart', async () => {
  const row = fixtureRow()
  useReviewDrafts.getState().edit(
    `/project:${row.key}:hidden-proposal`,
    {
      authorization: {
        target: row.line.target,
        guard: row.scene.guard,
        context_revision: row.line.context_revision,
        state_json: '{}',
      },
      body: 'My hidden revision',
      originalBody: 'Original',
      currentBody: 'Current',
      currentRevision: 'revision',
      proposal: null,
    },
    'My hidden revision',
  )
  render(<ReviewBulk rows={[row]} projectKey="/project" readOnly={false} onApplied={vi.fn()} />)
  fireEvent.click(screen.getByRole('button', { name: 'Review 1 selected decisions' }))
  await waitFor(() =>
    expect(screen.getByRole('status')).toHaveTextContent('0 eligible · 1 skipped'),
  )
  expect(api.narrativeReviewBatch).not.toHaveBeenCalled()
})
it('applies only the originally eligible requests and retains skipped/conflicting counts', async () => {
  const row = fixtureRow()
  const other = {
    ...row,
    key: 'other',
    line: { ...row.line, target: { ...row.line.target, slot: 'other' } },
  }
  api.narrativeReviewBatch
    .mockResolvedValueOnce({
      items: [
        { index: 0, target: row.line.target, status: 'eligible', reason: 'Eligible' },
        { index: 1, target: other.line.target, status: 'conflicting', reason: 'Source changed' },
      ],
    })
    .mockResolvedValueOnce({
      items: [{ index: 0, target: row.line.target, status: 'applied', reason: 'Saved' }],
    })
  render(
    <ReviewBulk rows={[row, other]} projectKey="/project" readOnly={false} onApplied={vi.fn()} />,
  )
  fireEvent.click(screen.getByRole('button', { name: 'Review 2 selected decisions' }))
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Apply 1 eligible decisions' })).toBeEnabled(),
  )
  fireEvent.click(screen.getByRole('button', { name: 'Apply 1 eligible decisions' }))
  await waitFor(() =>
    expect(screen.getByRole('status')).toHaveTextContent(
      '0 eligible · 0 skipped · 1 conflicting · 1 applied',
    ),
  )
  expect(api.narrativeReviewBatch.mock.calls[1]![0]).toHaveLength(1)
  expect(api.narrativeReviewBatch.mock.calls[1]![0][0].target).toEqual(row.line.target)
  expect(api.narrativeReviewBatch.mock.calls[1]![1]).toBe(true)
})
