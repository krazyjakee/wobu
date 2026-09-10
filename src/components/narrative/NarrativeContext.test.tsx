import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import {
  narrativeContextCapture,
  narrativeContextFreshness,
  type FrozenContext,
} from '../../lib/api/narrativeContext'
import { qk } from '../../lib/queries/keys'
import { NarrativeContext } from './NarrativeContext'
vi.mock('../../lib/api/narrativeContext', () => ({
  narrativeContextCapture: vi.fn(),
  narrativeContextFreshness: vi.fn(),
}))
const selection = { scene: 'council', beat: 'evidence', slot: 'witness' }
const frozen: FrozenContext = {
  version: 1,
  options: {
    selection: { ...selection, variant: null },
    state: { chapter: 3 },
    token_budget: 4000,
  },
  fragments: [
    {
      kind: 'knowledge',
      source: 'world/knowledge/witness',
      required: false,
      data: {
        claim: { belief: 'true', provenance: 'witnessed' },
        canonical_fact: { name: 'Attack', assertion: 'The Citadel was attacked', sources: [] },
      },
    },
  ],
  omitted: [],
  diagnostics: [],
  dependencies: { 'world/facts/attack': 'fact-revision' },
  queries: [{ name: 'restrictions', parameters: {}, members: {} }],
  request: 'Frozen request with witnessed knowledge',
  estimated_tokens: 150,
  ready: true,
  hash: 'frozen-hash',
}
function setup() {
  const qc = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } })
  qc.setQueryData(qk.narrativeState, { document: { variables: [{ name: 'chapter', default: 3 }] } })
  render(
    <QueryClientProvider client={qc}>
      <NarrativeContext selection={selection} slot={{ id: 'witness', speaker: 'narrator' }} />
    </QueryClientProvider>,
  )
}
beforeEach(() => {
  vi.clearAllMocks()
  vi.mocked(narrativeContextCapture).mockResolvedValue(structuredClone(frozen))
})
it('captures complete scenario and retains exact frozen request when freshness fails', async () => {
  setup()
  fireEvent.click(screen.getByRole('button', { name: 'Inspect generation request' }))
  await screen.findByText(/Context ready for review/)
  expect(narrativeContextCapture).toHaveBeenCalledWith(
    frozen.options,
    JSON.stringify(frozen.options.state, null, 2),
  )
  expect(screen.getByText(frozen.request)).toBeInTheDocument()
  vi.mocked(narrativeContextFreshness).mockResolvedValue({ current: false, hash: 'changed' })
  fireEvent.click(screen.getByRole('button', { name: 'Check source freshness' }))
  await screen.findByText(/Out of date: saved dependencies/)
  expect(narrativeContextFreshness).toHaveBeenCalledWith(frozen.options, frozen.hash)
  expect(screen.getByText(frozen.request)).toBeInTheDocument()
})
it('shows blocking constraints and explicit omissions without implying a provider run', async () => {
  vi.mocked(narrativeContextCapture).mockResolvedValue({
    ...frozen,
    ready: false,
    omitted: ['world/events/aftermath'],
    diagnostics: [
      {
        code: 'required_overflow',
        source: 'request',
        message: 'Required context exceeds the estimated budget.',
        blocking: true,
      },
    ],
  })
  setup()
  fireEvent.click(screen.getByRole('button', { name: 'Inspect generation request' }))
  await screen.findByText(/Context blocked/)
  expect(screen.getByText(/Required context exceeds/)).toBeInTheDocument()
  expect(screen.getByText('world/events/aftermath')).toBeInTheDocument()
  expect(screen.getByText(/No provider is called/)).toBeInTheDocument()
})
it('rejects unsafe or malformed scenario input and keeps a capture error visible', async () => {
  setup()
  fireEvent.change(screen.getByLabelText('Scenario state (JSON)'), {
    target: { value: '{"chapter":9007199254740993}' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Inspect generation request' }))
  await screen.findByRole('alert')
  expect(narrativeContextCapture).not.toHaveBeenCalled()
  fireEvent.change(screen.getByLabelText('Scenario state (JSON)'), {
    target: { value: '{"chapter":3}' },
  })
  vi.mocked(narrativeContextCapture).mockRejectedValue(
    new Error('Source changed while capturing context'),
  )
  fireEvent.click(screen.getByRole('button', { name: 'Inspect generation request' }))
  await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('Source changed'))
})

it('sends original numeric spelling to Rust before a decimal can become an integer', async () => {
  setup()
  const raw = '{"chapter":9007199254740991.4}'
  fireEvent.change(screen.getByLabelText('Scenario state (JSON)'), { target: { value: raw } })
  vi.mocked(narrativeContextCapture).mockRejectedValue(new Error('Expected an integer'))
  fireEvent.click(screen.getByRole('button', { name: 'Inspect generation request' }))
  await screen.findByRole('alert')
  expect(narrativeContextCapture).toHaveBeenCalledWith(expect.anything(), raw)
})
