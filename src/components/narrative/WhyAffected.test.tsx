import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
import { expect, it, vi } from 'vitest'

import type { AffectedLine } from '../../lib/api/narrativeDeps'
import { WhyAffected } from './WhyAffected'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const line = (over: Partial<AffectedLine> = {}): AffectedLine => ({
  scene: 'council',
  asset: null,
  slot: 'line',
  variant: 'high-trust',
  kind: 'changed',
  before: 'a'.repeat(64),
  after: 'b'.repeat(64),
  explanations: [
    {
      source: 'character/kael/narrative_voice',
      context: 'context',
      line: 'scene/council/beat/evidence/slot/line/variant/high-trust',
      message: 'character/kael/narrative_voice was edited.',
    },
  ],
  ...over,
})

function draw(rows: AffectedLine[], slotId = 'line') {
  const qc = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } })
  qc.setQueryData(['narrative_affected'], rows)
  render(
    <QueryClientProvider client={qc}>
      <WhyAffected slotId={slotId} />
    </QueryClientProvider>,
  )
}

it('names the source field, the context that carried it and what to do next', () => {
  draw([line()])
  expect(screen.getByRole('region', { name: 'Why affected' })).toBeInTheDocument()
  expect(screen.getByText('character/kael/narrative_voice')).toBeInTheDocument()
  expect(screen.getByText('context')).toBeInTheDocument()
  expect(screen.getByText('character/kael/narrative_voice was edited.')).toBeInTheDocument()
  // The honest limit: this pane explains, and #169 is what rebuilds.
  expect(screen.getByText(/needs the affected build planner \(#169\)/)).toBeInTheDocument()
})

it('says nothing about a line nothing has affected', () => {
  draw([line({ slot: 'other' })])
  expect(screen.queryByRole('region', { name: 'Why affected' })).not.toBeInTheDocument()
})

it('does not claim an untracked line has gone stale', () => {
  // A line the tracker has never recorded is new, or the local index was lost.
  // Neither is a reason to tell a writer their wording is out of date.
  draw([line({ kind: 'untracked', before: null, explanations: [] })])
  expect(screen.queryByRole('region', { name: 'Why affected' })).not.toBeInTheDocument()
})

it('names every source that moved rather than only the first', () => {
  draw([
    line({
      explanations: [
        ...line().explanations,
        {
          source: 'world/relationships/rel',
          context: 'query directed_relationships',
          line: 'scene/council/beat/evidence/slot/line/variant/high-trust',
          message: 'rel is new and matches directed_relationships.',
        },
      ],
    }),
  ])
  expect(screen.getByText('world/relationships/rel')).toBeInTheDocument()
  expect(screen.getByText('query directed_relationships')).toBeInTheDocument()
})
