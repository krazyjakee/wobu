import { beforeEach, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { ReviewDetail } from './ReviewDetail'
import { fixtureRow, reviewFixture } from './reviewFixture.test-support'
import { useReviewDrafts } from './reviewDrafts'
import { resetNarrativeDraftGuards } from '../../../lib/narrativeDraftGuard'

beforeEach(() => {
  resetNarrativeDraftGuards()
  useReviewDrafts.setState({ drafts: {} })
})
function props() {
  return {
    row: fixtureRow(),
    projectKey: '/project',
    readOnly: false,
    proposalId: 'proposal',
    onProposal: vi.fn(),
    onApply: vi.fn().mockResolvedValue(undefined),
    onContext: vi
      .fn()
      .mockResolvedValue({
        version: 1,
        revision: 'original-context',
        state: {},
        inputs: { facts: ['beacon_failed'] },
      }),
    onSource: vi.fn(),
  }
}
it('keeps edited wording and original authorization after peer refresh and an acceptance conflict', async () => {
  const original = props()
  const view = render(<ReviewDetail {...original} />)
  fireEvent.change(screen.getByLabelText('Reviewed wording'), {
    target: { value: 'My carefully edited line.' },
  })
  const peer = reviewFixture()
  peer.guard.head = 'peer-head'
  peer.lines[0]!.text!.body = 'A peer changed this.'
  peer.lines[0]!.context_revision = 'peer-context'
  original.onApply.mockRejectedValueOnce(new Error('Source changed. Reload and compare.'))
  view.rerender(<ReviewDetail {...original} row={fixtureRow(peer)} />)
  expect(screen.getByText(/Saved source changed while you were editing/)).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: 'Accept edited proposal' }))
  await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('Source changed'))
  expect(original.onApply).toHaveBeenCalledWith(
    expect.objectContaining({
      guard: { stamp: null, head: 'original-head' },
      context_revision: 'original-context',
      action: {
        kind: 'accept',
        proposal_id: 'proposal',
        proposal_hash: 'proposal-hash',
        reviewed_text: 'My carefully edited line.',
      },
    }),
  )
  expect(screen.getByLabelText('Reviewed wording')).toHaveValue('My carefully edited line.')
  expect(screen.getByRole('button', { name: 'Approve saved wording' })).toBeDisabled()
})
it('requires inspected current context to prepare a manual edit and leaves the proposal pending', async () => {
  const original = props()
  render(<ReviewDetail {...original} />)
  const adopt = screen.getByRole('button', {
    name: 'Prepare manual edit from this wording',
    hidden: true,
  })
  expect(adopt).toBeDisabled()
  fireEvent.click(screen.getByText('Use proposal wording as a manual edit'))
  fireEvent.click(screen.getByRole('button', { name: 'Inspect context for manual edit' }))
  await waitFor(() => expect(adopt).toBeEnabled())
  fireEvent.click(adopt)
  expect(original.onApply).not.toHaveBeenCalled()
  expect(original.onProposal).toHaveBeenCalledWith(null)
  const drafts = Object.values(useReviewDrafts.getState().drafts)
  expect(drafts).toHaveLength(1)
  expect(drafts[0]).toMatchObject({
    body: 'I watched the beacon go dark.',
    proposal: null,
    authorization: { guard: { head: 'original-head' } },
  })
  expect(original.row.line.proposals[0]!.status).toBe('pending')
})
it('blocks locked replacements while permitting an explicit unlock of the protecting slot', async () => {
  const original = props()
  original.row.line.slot_policy = 'locked'
  render(<ReviewDetail {...original} />)
  expect(screen.getByLabelText('Reviewed wording')).toBeDisabled()
  expect(screen.getByRole('button', { name: 'Accept proposal' })).toBeDisabled()
  fireEvent.click(screen.getByRole('button', { name: 'Unlock slot' }))
  await waitFor(() =>
    expect(original.onApply).toHaveBeenCalledWith(
      expect.objectContaining({ action: { kind: 'policy', scope: 'slot', policy: 'edited' } }),
    ),
  )
})
