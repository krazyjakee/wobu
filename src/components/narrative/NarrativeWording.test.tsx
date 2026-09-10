import { describe, expect, it, vi, beforeEach } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { NarrativeWording } from './NarrativeWording'
import type { WordingReport } from '../../lib/api/narrativeWording'

const report = vi.fn()
const suppress = vi.fn()
vi.mock('../../lib/api/narrativeWording', () => ({
  narrativeWordingReport: (...args: unknown[]) => report(...args),
  narrativeWordingSuppress: (...args: unknown[]) => suppress(...args),
}))

const REVISION = '71578f9efa77f9e4b9a979ca3d93be0c'
const found: WordingReport = {
  duplicated: [
    {
      revision: REVISION,
      body: 'You look like you need a job more than a coffee.',
      sites: [
        {
          container: { scene: 's02' },
          containerName: 'A shift at the diner',
          slot: 'slot-1',
          variant: 'variant-1',
          site: null,
        },
        {
          container: { text_asset: 'a01' },
          containerName: 'Objective — A shift at the diner',
          slot: 'slot-2',
          variant: 'variant-2',
          site: null,
        },
      ],
    },
  ],
  suppressions: [],
  guard: 'g'.repeat(8),
  unreadable: [],
}

beforeEach(() => {
  report.mockReset()
  suppress.mockReset()
  report.mockResolvedValue(found)
})

describe('Repeated wording', () => {
  it('names every copy of a duplicated wording, in both kinds of document', async () => {
    render(<NarrativeWording readOnly={false} onClose={() => {}} />)
    await screen.findByText(found.duplicated[0]!.body)
    const copies = within(screen.getByRole('list', { name: `Copies of ${REVISION}` }))
    // A quest summary that is a copy of a scene line is the case this exists
    // for, so both sides have to be nameable from one place.
    expect(copies.getByText('A shift at the diner (scene)')).toBeInTheDocument()
    expect(
      copies.getByText('Objective — A shift at the diner (supporting text)'),
    ).toBeInTheDocument()
  })

  it('will not allow a repetition without a reason, and sends the guard with the one it has', async () => {
    suppress.mockResolvedValue({
      ...found,
      duplicated: [],
      suppressions: [{ revision: REVISION, rationale: 'A deliberate refrain.' }],
    })
    render(<NarrativeWording readOnly={false} onClose={() => {}} />)
    const allow = await screen.findByRole('button', { name: 'Allow this repetition' })
    // A suppression with no reason cannot be told apart from a warning somebody
    // dismissed, so there is nothing to press until there is one.
    expect(allow).toBeDisabled()

    fireEvent.change(screen.getByLabelText('Why this repetition is deliberate'), {
      target: { value: 'A deliberate refrain.' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Allow this repetition' }))
    await waitFor(() =>
      expect(suppress).toHaveBeenCalledWith(
        [{ revision: REVISION, rationale: 'A deliberate refrain.' }],
        found.guard,
      ),
    )
    expect(await screen.findByText('A deliberate refrain.')).toBeInTheDocument()
  })

  it('shows findings but offers no write on a read-only folder', async () => {
    render(<NarrativeWording readOnly onClose={() => {}} />)
    await screen.findByText(found.duplicated[0]!.body)
    expect(screen.getByRole('button', { name: 'Allow this repetition' })).toBeDisabled()
    expect(suppress).not.toHaveBeenCalled()
  })

  it('says when a source file could not be read rather than reporting it as clean', async () => {
    report.mockResolvedValue({
      ...found,
      duplicated: [],
      unreadable: ['narrative/scenes/broken.yaml'],
    })
    render(<NarrativeWording readOnly={false} onClose={() => {}} />)
    expect(
      await screen.findByText(/narrative\/scenes\/broken\.yaml would not parse/),
    ).toBeInTheDocument()
  })
})
