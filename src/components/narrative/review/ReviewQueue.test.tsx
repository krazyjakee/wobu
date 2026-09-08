import { beforeEach, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, within } from '@testing-library/react'
import { ReviewQueue } from './ReviewQueue'
import { reviewFixture } from './reviewFixture.test-support'
import { useReviewDrafts } from './reviewDrafts'
import { resetNarrativeDraftGuards } from '../../../lib/narrativeDraftGuard'

beforeEach(() => {
  resetNarrativeDraftGuards()
  useReviewDrafts.setState({ drafts: {} })
})
function props() {
  const scene = reviewFixture()
  scene.lines.push({
    ...scene.lines[0]!,
    target: { ...scene.lines[0]!.target, slot: 'farewell', variant: 'farewell-main' },
    speaker: 'player',
    proposals: [],
    text: {
      revision: 'farewell-revision',
      body: 'Then we start again.',
      lifecycle: { policy: 'locked', review: 'approved', freshness: 'out_of_date' },
    },
    review: 'approved',
    freshness: 'out_of_date',
    approval_valid: false,
  })
  return {
    projectKey: '/project',
    readOnly: false,
    views: [scene],
    sceneName: () => 'Beacon aftermath',
    speakerName: () => 'Mara',
    loading: false,
    error: '',
    onRefresh: vi.fn().mockResolvedValue(undefined),
    onApply: vi.fn().mockResolvedValue(undefined),
    onContext: vi.fn(),
    onSource: vi.fn(),
    onClose: vi.fn(),
  }
}
it('filters a project queue by approval, freshness and speaker with a useful empty state', () => {
  render(<ReviewQueue {...props()} />)
  const queue = within(screen.getByRole('region', { name: 'Project review queue' }))
  expect(queue.getAllByRole('checkbox')).toHaveLength(2)
  fireEvent.change(screen.getByLabelText('Approval'), { target: { value: 'invalid' } })
  expect(queue.getAllByRole('checkbox')).toHaveLength(1)
  expect(queue.getByText('Then we start again.')).toBeVisible()
  fireEvent.change(screen.getByLabelText('Speaker'), { target: { value: 'mara' } })
  expect(queue.queryAllByRole('checkbox')).toHaveLength(0)
  expect(queue.getByText(/No lines match these filters/)).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }))
  fireEvent.change(screen.getByLabelText('Freshness'), { target: { value: 'current' } })
  expect(queue.getAllByRole('checkbox')).toHaveLength(1)
  expect(queue.getByText('Mara')).toBeVisible()
})
it('supports arrow traversal, retains edits on navigation, and opens the exact source target', () => {
  const original = props()
  render(<ReviewQueue {...original} />)
  fireEvent.change(screen.getByLabelText('Reviewed wording'), {
    target: { value: 'A retained personal revision.' },
  })
  const queue = within(screen.getByRole('region', { name: 'Project review queue' }))
  const mara = queue.getByRole('button', { name: /Mara Beacon aftermath/ })
  fireEvent.keyDown(mara, { key: 'ArrowDown' })
  expect(screen.getByRole('heading', { name: 'Player' })).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: 'Open in Script' }))
  expect(original.onSource).toHaveBeenCalledWith(original.views[0]!.lines[1]!.target)
  fireEvent.click(mara)
  expect(screen.getByLabelText('Reviewed wording')).toHaveValue('A retained personal revision.')
})
it('paginates hundreds of lines and keeps explicit selection outside the current page', () => {
  const original = props()
  const first = original.views[0]!.lines[0]!
  original.views[0]!.lines = Array.from({ length: 201 }, (_, index) => ({
    ...first,
    target: { ...first.target, slot: `slot-${index}`, variant: `variant-${index}` },
  }))
  const bulk = vi.fn().mockReturnValue(null)
  render(<ReviewQueue {...original} renderBulk={bulk} />)
  expect(screen.getAllByRole('checkbox')).toHaveLength(50)
  fireEvent.click(screen.getByRole('button', { name: 'Select this page' }))
  fireEvent.click(screen.getByRole('button', { name: 'Next page' }))
  expect(screen.getAllByRole('checkbox')).toHaveLength(50)
  expect(screen.getByText('Page 2 of 5')).toBeVisible()
  expect(bulk.mock.lastCall?.[0]).toHaveLength(50)
})
