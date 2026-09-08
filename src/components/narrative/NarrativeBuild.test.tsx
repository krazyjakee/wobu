import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { NarrativeBuild } from './NarrativeBuild'
import type { BuildItem, NarrativeBuild as Build } from '../../lib/api/narrativeBuild'
const api = vi.hoisted(() => ({
  plan: vi.fn(),
  start: vi.fn(),
  list: vi.fn(),
  status: vi.fn(),
  jobs: vi.fn(),
  cancel: vi.fn(),
}))
vi.mock('../../lib/api/narrativeBuild', () => ({
  narrativeBuildPlan: api.plan,
  narrativeBuildStart: api.start,
  narrativeBuildList: api.list,
  narrativeBuildStatus: api.status,
}))
vi.mock('../../lib/api', async (original) => ({
  ...(await original<object>()),
  jobList: api.jobs,
  jobCancel: api.cancel,
}))
vi.mock('../../lib/queries', () => ({
  useNarrativeState: () => ({ data: { document: { variables: [] } }, isLoading: false }),
}))
const row = (id: string, action: BuildItem['action']): BuildItem => ({
  id,
  target: { scene: 'scene', beat: 'beat', slot: id, variant: 'variant' },
  asset: false,
  label: id,
  action,
  reasons: [{ source: 'character/voice', context: 'speaker', line: id, message: 'Voice changed' }],
  diagnostics: [],
  request_id: ['locked', 'blocked'].includes(action) ? null : `request-${id}`,
  reusable: false,
  candidate_variant_id: 'variant',
  state: {},
})
const build: Build = {
  id: 'build',
  version: 1,
  scope: 'affected',
  provider: 'anthropic',
  model: 'saved-model',
  items: [row('generated', 'generate'), row('edited', 'propose'), row('locked', 'locked')],
  diagnostics: [],
}
function mount() {
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <NarrativeBuild
        projectKey="project"
        readOnly={false}
        currentScene="scene"
        onClose={vi.fn()}
        onScenarios={vi.fn()}
        onSource={vi.fn()}
      />
    </QueryClientProvider>,
  )
}
beforeEach(() => {
  vi.resetAllMocks()
  api.list.mockResolvedValue([])
  api.jobs.mockResolvedValue({ jobs: [] })
  api.plan.mockResolvedValue(build)
  api.status.mockResolvedValue({ build, history: [] })
  api.start.mockResolvedValue([])
})
it('plans keylessly, displays reasons and policies, and submits only the explicit subset', async () => {
  mount()
  fireEvent.click(screen.getByRole('button', { name: 'Plan work' }))
  expect(await screen.findByText(/anthropic \/ saved-model/)).toBeInTheDocument()
  expect(screen.getByLabelText('Policy counts')).toHaveTextContent(
    '1 Regenerate · 1 Review proposal · 1 Locked',
  )
  expect(screen.getByLabelText('Select locked')).toBeDisabled()
  expect(screen.getAllByText(/Voice changed/)).toHaveLength(3)
  expect(api.start).not.toHaveBeenCalled()
  fireEvent.click(screen.getByLabelText('Select edited'))
  fireEvent.click(screen.getByRole('button', { name: 'Generate / resume 1 selected' }))
  await waitFor(() => expect(api.start).toHaveBeenCalledWith('build', ['generated']))
})
it('reopens completed work without spending and selects only failed items for explicit retry', async () => {
  api.list.mockResolvedValue([
    { id: 'build', scope: 'affected', items: 3, provider: 'anthropic', model: 'saved-model' },
  ])
  const previous = (id: string, status: string) => ({
    request_id: `request-${id}`,
    receipt_id: `receipt-${id}`,
    attempts: 1,
    status,
    proposal_published: status === 'succeeded',
    billing_unknown: status === 'failed',
  })
  api.status.mockResolvedValue({
    build,
    history: [previous('generated', 'succeeded'), previous('edited', 'failed')],
    decided: ['receipt-generated'],
  })
  mount()
  fireEvent.change(await screen.findByLabelText('Resume a saved build'), {
    target: { value: 'build' },
  })
  expect(await screen.findByText('Completed')).toBeInTheDocument()
  expect(screen.getByLabelText('Select generated')).toBeDisabled()
  expect(api.start).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: 'Select failed items' }))
  fireEvent.click(screen.getByRole('button', { name: 'Generate / resume 1 selected' }))
  await waitFor(() => expect(api.start).toHaveBeenCalledWith('build', ['edited']))
})
it('sends raw scenario numbers and current container scope without webview rounding', async () => {
  mount()
  fireEvent.change(screen.getByLabelText('Selected scope'), { target: { value: 'current' } })
  fireEvent.change(screen.getByLabelText('State (JSON)'), {
    target: { value: '{"trust":9007199254740991.1}' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Plan work' }))
  await waitFor(() => expect(api.plan).toHaveBeenCalled())
  expect(api.plan.mock.calls[0]?.[0]).toContain('9007199254740991.1')
  expect(api.plan.mock.calls[0]?.[0]).toContain('"containers":["scene"]')
})

it('paginates 483 rows while selecting eligible work across the whole build', async () => {
  const large = {
    ...build,
    items: Array.from({ length: 483 }, (_, i) => row(`line-${i}`, 'generate')),
  }
  api.plan.mockResolvedValue(large)
  api.status.mockResolvedValue({ build: large, history: [] })
  mount()
  fireEvent.click(screen.getByRole('button', { name: 'Plan work' }))
  expect(await screen.findByText('1–50 of 483')).toBeInTheDocument()
  expect(screen.getAllByRole('checkbox')).toHaveLength(50)
  fireEvent.click(screen.getByRole('button', { name: 'Next page' }))
  expect(screen.getByText('51–100 of 483')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Generate / resume 483 selected' }))
  await waitFor(() =>
    expect(api.start).toHaveBeenCalledWith(
      'build',
      large.items.map((item) => item.id),
    ),
  )
})

it('requires an explicit reuse action for a successful result referenced by a new plan', async () => {
  const reused = { ...build, items: [{ ...row('edited', 'propose'), reusable: true }] }
  api.plan.mockResolvedValue(reused)
  api.status.mockResolvedValue({
    build: reused,
    dispatched: [],
    history: [
      { request_id: 'request-edited', status: 'succeeded', attempts: 1, proposal_published: true },
    ],
  })
  mount()
  fireEvent.click(screen.getByRole('button', { name: 'Plan work' }))
  expect(await screen.findByText('Stored result ready')).toBeInTheDocument()
  expect(api.start).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: 'Generate / resume 1 selected' }))
  await waitFor(() => expect(api.start).toHaveBeenCalledWith('build', ['edited']))
})

it('keeps a published Generated result selectable until canonical acceptance is recorded', async () => {
  const pending = { ...build, items: [row('generated', 'generate')] }
  api.plan.mockResolvedValue(pending)
  api.status.mockResolvedValue({
    build: pending,
    dispatched: ['request-generated'],
    decided: [],
    history: [
      {
        request_id: 'request-generated',
        receipt_id: 'receipt-generated',
        status: 'succeeded',
        attempts: 1,
        proposal_published: true,
      },
    ],
  })
  mount()
  fireEvent.click(screen.getByRole('button', { name: 'Plan work' }))
  expect(await screen.findByText('Resume acceptance; no provider call')).toBeInTheDocument()
  expect(screen.getByLabelText('Select generated')).toBeEnabled()
  expect(api.start).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: 'Generate / resume 1 selected' }))
  await waitFor(() => expect(api.start).toHaveBeenCalledWith('build', ['generated']))
})
