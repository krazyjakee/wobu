import { reviewLoopFixture } from './reviewLoopFixture.test-support'
import { beforeEach, expect, it } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { clearMocks } from '@tauri-apps/api/mocks'
import { ReviewLoop } from './ReviewLoop.test-support'
import { resetNarrativeDraftGuards } from '../../../lib/narrativeDraftGuard'
import { useReviewDrafts } from './reviewDrafts'
beforeEach(() => {
  clearMocks()
  resetNarrativeDraftGuards()
  useReviewDrafts.setState({ drafts: {} })
})
it('runs generation, protected editing, regeneration comparison, approval, lock and stale attestation', async () => {
  const fixture = reviewLoopFixture()
  render(<ReviewLoop fixture={fixture} />)
  const generate = async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Open generation' }))
    fireEvent.change(screen.getByLabelText('Generation selection'), { target: { value: '0' } })
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Plan generation' })).toBeEnabled(),
    )
    fireEvent.click(screen.getByRole('button', { name: 'Plan generation' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Queue 1 provider request' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Close' })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
  }
  await generate()
  fireEvent.click(screen.getByRole('button', { name: 'Open review' }))
  expect(await screen.findByLabelText('Reviewed wording')).toHaveValue(
    'I watched the beacon go dark.',
  )
  fireEvent.change(screen.getByLabelText('Reviewed wording'), {
    target: { value: 'I should have warned you sooner.' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Save edited wording' }))
  await waitFor(() => expect(fixture.decisions.at(-1)?.action.kind).toBe('edit'))
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Save edited wording' })).toBeDisabled(),
  )
  fireEvent.click(screen.getByRole('button', { name: 'Close review' }))
  await generate()
  expect(fixture.view.lines[0]!.text!.body).toBe('I should have warned you sooner.')
  fireEvent.click(screen.getByRole('button', { name: 'Open review' }))
  await waitFor(() =>
    expect(screen.getByLabelText('Reviewed wording')).toHaveValue(
      'I saw the last light die. We were too late.',
    ),
  )
  expect(screen.getByRole('region', { name: 'Wording comparison' })).toHaveTextContent(
    'I should have warned you sooner.',
  )
  fireEvent.change(screen.getByLabelText('Reviewed wording'), {
    target: { value: 'I saw the last light die. I should have warned you.' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Accept edited proposal' }))
  await waitFor(() =>
    expect(
      screen.queryByRole('button', { name: 'Accept edited proposal' }),
    ).not.toBeInTheDocument(),
  )
  fireEvent.click(screen.getByRole('button', { name: 'Approve saved wording' }))
  await waitFor(() => expect(screen.getByText(/Approval valid/)).toBeVisible())
  fireEvent.click(screen.getByRole('button', { name: 'Lock wording' }))
  await screen.findByRole('button', { name: 'Unlock wording' })
  expect(screen.getByLabelText('Reviewed wording')).toBeDisabled()
  fireEvent.click(screen.getByRole('button', { name: 'Close review' }))
  fireEvent.click(screen.getByRole('button', { name: 'Simulate upstream context change' }))
  fireEvent.click(screen.getByRole('button', { name: 'Open review' }))
  await screen.findByText(/Character knowledge changed after approval/)
  fireEvent.click(screen.getByRole('button', { name: 'Attest unchanged wording' }))
  await waitFor(() => expect(fixture.view.lines[0]!.approval_valid).toBe(true))
  expect(fixture.view.lines[0]!.text!.body).toBe(
    'I saw the last light die. I should have warned you.',
  )
  expect(fixture.decisions.map((request) => request.action.kind)).toEqual([
    'edit',
    'accept',
    'approve',
    'policy',
    'attest',
  ])
})
