import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { NarrativeGeneration } from './NarrativeGeneration'
const api = vi.hoisted(() => ({
  plan: vi.fn(),
  start: vi.fn(),
  history: vi.fn(),
  retry: vi.fn(),
  recover: vi.fn(),
  jobs: vi.fn(),
  cancel: vi.fn(),
}))
vi.mock('../../lib/api/narrativeGeneration', () => ({
  narrativeGenerationPlan: api.plan,
  narrativeGenerationStart: api.start,
  narrativeGenerationHistory: api.history,
  narrativeGenerationRetry: api.retry,
  narrativeGenerationRecover: api.recover,
}))
vi.mock('../../lib/api', async (original) => ({
  ...(await original<object>()),
  jobList: api.jobs,
  jobCancel: api.cancel,
}))
vi.mock('../../lib/queries', () => ({
  useNarrativeState: () => ({ data: { document: { variables: [] } }, isLoading: false }),
}))
const scene = {
  id: 'scene',
  name: 'Harbour rumours',
  beats: [
    {
      id: 'beat',
      title: 'Ask about the beacon',
      dialogue: [{ id: 'slot', speaker: 'narrator' as const, variants: [] }],
    },
  ],
}
const selection = { scene: 'scene', beat: 'beat', slot: 'slot', variant: null }
const item = {
  request_id: 'request',
  batch_id: 'batch',
  target: selection,
  provider: 'anthropic',
  model: 'fixture-model',
  attempts: 0,
  status: 'interrupted',
  receipt_id: null,
  candidate: null,
  usage: { input: 0, output: 0, cached_input: 0 },
  billing_unknown: true,
  error_code: null,
  proposal_published: false,
}
function mount(readOnly = false) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <NarrativeGeneration
        projectKey="test-project"
        scene={scene}
        readOnly={readOnly}
        onClose={vi.fn()}
      />
    </QueryClientProvider>,
  )
}
beforeEach(() => {
  vi.resetAllMocks()
  api.history.mockResolvedValue([])
  api.jobs.mockResolvedValue({ jobs: [], queued: 0, running: 0, retrying: 0 })
  api.start.mockResolvedValue([])
  api.plan.mockResolvedValue({
    id: 'batch',
    provider: 'anthropic',
    model: 'fixture-model',
    requests: [
      {
        request_id: 'request',
        target: selection,
        candidate_variant_id: 'variant',
        expected_policy: null,
        context: { estimated_tokens: 200 },
        prompt: 'Frozen attributed context',
      },
    ],
    skipped: [
      { target: { ...selection, slot: 'locked-slot' }, reason: 'Locked dialogue is excluded.' },
    ],
  })
})
it('shows request count, model and skipped locks before explicit queue submission', async () => {
  mount()
  fireEvent.click(screen.getByRole('button', { name: 'Plan generation' }))
  expect(await screen.findByText('1 request · 1 skipped')).toBeInTheDocument()
  expect(screen.getByText(/anthropic \/ fixture-model/)).toBeInTheDocument()
  expect(screen.getByText(/Locked dialogue is excluded/)).toBeInTheDocument()
  expect(api.start).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: 'Queue 1 provider request' }))
  await waitFor(() => expect(api.start).toHaveBeenCalledWith('batch'))
  await waitFor(() => expect(screen.queryByText('1 request · 1 skipped')).not.toBeInTheDocument())
})
it('preserves raw numeric spelling and selected variant intent for backend validation', async () => {
  mount()
  fireEvent.change(screen.getByLabelText('Generation selection'), { target: { value: '0' } })
  fireEvent.change(screen.getByLabelText('Generation state (JSON)'), {
    target: { value: '{"trust":9007199254740991.1}' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Plan generation' }))
  await waitFor(() => expect(api.plan).toHaveBeenCalled())
  expect(api.plan.mock.calls[0]?.[0]).toContain('9007199254740991.1')
  expect(api.plan.mock.calls[0]?.[0]).toContain(
    '"selection":{"scene":"scene","beat":"beat","slot":"slot","variant":null}',
  )
})
it('requires explicit charged retry and recovers successful output without a provider call', async () => {
  api.history.mockResolvedValue([
    item,
    {
      ...item,
      request_id: 'success',
      status: 'succeeded',
      candidate: { text: 'They say the beacon went dark.' },
    },
  ])
  mount()
  expect(await screen.findAllByText(/Billing is unknown/)).toHaveLength(2)
  expect(api.retry).not.toHaveBeenCalled()
  fireEvent.click(
    screen.getByRole('button', { name: 'Recover retained result — no provider call' }),
  )
  await waitFor(() => expect(api.recover).toHaveBeenCalledWith('success'))
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Retry request — may incur charges' })).toBeEnabled(),
  )
  fireEvent.click(screen.getByRole('button', { name: 'Retry request — may incur charges' }))
  await waitFor(() => expect(api.retry).toHaveBeenCalledWith('request'))
})
it('cancels only active requests from this project and exposes read-only history', async () => {
  api.history.mockResolvedValue([item])
  api.jobs.mockResolvedValue({
    jobs: [
      { id: 'ours', kind: 'narrative', subjectId: 'request', state: 'running' },
      { id: 'other-project', kind: 'narrative', subjectId: 'unrelated', state: 'running' },
    ],
    queued: 0,
    running: 2,
    retrying: 0,
  })
  mount(true)
  expect(screen.getByRole('button', { name: 'Plan generation' })).toBeDisabled()
  fireEvent.click(await screen.findByRole('button', { name: 'Cancel request' }))
  await waitFor(() => expect(api.cancel).toHaveBeenCalledWith('ours'))
  expect(api.cancel).toHaveBeenCalledTimes(1)
})
it('displays planning errors without submitting a request', async () => {
  api.plan.mockRejectedValue({ message: 'Save or reload source before planning.' })
  mount()
  fireEvent.click(screen.getByRole('button', { name: 'Plan generation' }))
  expect(await screen.findByRole('alert')).toHaveTextContent(
    'Save or reload source before planning.',
  )
  expect(api.start).not.toHaveBeenCalled()
})

it('shows an active retry ahead of its retained older terminal attempt', async () => {
  api.history.mockResolvedValue([{ ...item, status: 'failed', attempts: 1 }])
  api.jobs.mockResolvedValue({
    jobs: [
      { id: 'old-failed', kind: 'narrative', subjectId: 'request', state: 'failed' },
      { id: 'new-running', kind: 'narrative', subjectId: 'request', state: 'running' },
    ],
    queued: 0,
    running: 1,
    retrying: 0,
  })
  mount()
  expect(await screen.findByText('slot · running')).toBeInTheDocument()
  expect(
    screen.queryByRole('button', { name: 'Retry request — may incur charges' }),
  ).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Cancel request' }))
  await waitFor(() => expect(api.cancel).toHaveBeenCalledWith('new-running'))
  expect(api.cancel).not.toHaveBeenCalledWith('old-failed')
})
