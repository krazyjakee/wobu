import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import type { ReviewLine, ReviewSceneView } from '../../lib/api/narrativeReview'
import { ScriptReviewControls } from './ScriptReviewControls'
const context = vi.hoisted(() => vi.fn())
vi.mock('../../lib/api/narrativeReview', async (original) => ({
  ...(await original<object>()),
  narrativeReviewContext: context,
}))
const line: ReviewLine = {
  target: { scene: 'scene', beat: 'beat', slot: 'slot', variant: 'variant' },
  speaker: 'narrator',
  text: {
    body: 'Unchanged final wording',
    revision: 'wording',
    lifecycle: { policy: 'locked', review: 'approved', freshness: 'current' },
  },
  slot_policy: 'edited',
  review: 'draft',
  freshness: 'out_of_date',
  approval_valid: false,
  reason: 'Character voice changed',
  context_revision: 'new-context',
  proposals: [],
}
const view: ReviewSceneView = {
  scene_id: 'scene',
  guard: { head: 'head', stamp: null },
  state_json: '{}',
  lines: [line],
  history: [],
  context_summary: 'Authored context and scenario',
}
beforeEach(() =>
  context.mockResolvedValue({
    version: 1,
    revision: 'new-context',
    state: {},
    inputs: { voice: 'More restrained' },
  }),
)
it('keeps locked stale text visible and requires inspected-context acknowledgement before attestation', async () => {
  const apply = vi.fn().mockResolvedValue(undefined)
  render(<ScriptReviewControls view={view} line={line} disabled={false} onApply={apply} />)
  expect(screen.getByText('Draft · Out of date')).toBeInTheDocument()
  const attest = screen.getByRole('button', { name: 'Attest unchanged wording still fits' })
  expect(attest).toBeDisabled()
  fireEvent.click(screen.getByRole('button', { name: 'Inspect reviewed context' }))
  fireEvent.click(await screen.findByRole('checkbox'))
  fireEvent.click(attest)
  await waitFor(() => expect(apply).toHaveBeenCalledWith({ kind: 'attest' }))
  expect(context).toHaveBeenCalledWith(line.target, '{}')
  expect(line.text?.body).toBe('Unchanged final wording')
})
it('withdraws local acknowledgement when the canonical context or guard changes', async () => {
  const apply = vi.fn()
  const rendered = render(
    <ScriptReviewControls view={view} line={line} disabled={false} onApply={apply} />,
  )
  fireEvent.click(screen.getByRole('button', { name: 'Inspect reviewed context' }))
  fireEvent.click(await screen.findByRole('checkbox'))
  expect(screen.getByRole('button', { name: 'Approve wording' })).toBeEnabled()
  rendered.rerender(
    <ScriptReviewControls
      view={{ ...view, guard: { head: 'peer-head', stamp: null } }}
      line={line}
      disabled={false}
      onApply={apply}
    />,
  )
  expect(screen.getByRole('button', { name: 'Approve wording' })).toBeDisabled()
  expect(apply).not.toHaveBeenCalled()
})
it('reports a rejected decision and disables policy controls while a script draft exists', async () => {
  const apply = vi.fn().mockRejectedValue(new Error('Scene changed; reload'))
  const rendered = render(
    <ScriptReviewControls view={view} line={line} slotOnly disabled={false} onApply={apply} />,
  )
  fireEvent.change(screen.getByLabelText('Slot policy'), { target: { value: 'locked' } })
  expect(await screen.findByRole('alert')).toHaveTextContent('Scene changed; reload')
  rendered.rerender(
    <ScriptReviewControls view={view} line={line} slotOnly disabled onApply={apply} />,
  )
  expect(screen.getByLabelText('Slot policy')).toBeDisabled()
})
