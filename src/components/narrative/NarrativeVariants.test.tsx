import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { NarrativeVariants } from './NarrativeVariants'
import type { AnalysisReport, AnalysisView } from '../../lib/api/narrativeVariants'
import type { Scene } from '../../lib/api'
import { useUndoStack } from '../../lib/undo'
const api = vi.hoisted(() => ({
  get: vi.fn(),
  plan: vi.fn(),
  materialize: vi.fn(),
  build: vi.fn(),
}))
vi.mock('../../lib/api/narrativeVariants', () => ({
  narrativeVariantsGet: api.get,
  narrativeVariantsPlan: api.plan,
  narrativeVariantsMaterialize: api.materialize,
  narrativeVariantsBuild: api.build,
}))
const scene: Scene = {
  id: 'scene',
  name: 'Hearing',
  beats: [
    {
      id: 'beat',
      title: 'Evidence',
      dialogue: [{ id: 'slot', speaker: 'narrator', policy: 'generated', variants: [] }],
    },
  ],
}
const policy: AnalysisView['policy'] = {
  version: 1,
  target: { scene: 'scene', beat: 'beat' },
  initial: [{ permit: false }],
  invariants: [],
  quests: [],
  events: [],
  external: false,
  limits: { states: 100, variants: 50, milliseconds: 500 },
}
const saved: AnalysisReport = {
  id: 'report',
  source_guard: 'source',
  world_guard: 'world',
  policies: { policies: [policy], guard: 'guard' },
  report: {
    target: policy.target,
    domains: [{ name: 'permit', ty: 'bool' }],
    potential: { value: '3', exact: true },
    included: { value: '2', exact: true },
    excluded: { value: '0', exact: true },
    unknown: { value: '1', exact: true },
    complete: false,
    explored_states: 2,
    limits: policy.limits,
    reasons: ['External input remains unknown'],
    assumptions: ['Declared model only'],
    rows: ['a', 'b', 'unknown'].map((id, index) => ({
      id,
      values: { permit: index > 0 },
      when: 'always',
      classification: index < 2 ? 'included' : 'unknown',
      reason: 'Declared model only',
      witness:
        index < 2
          ? {
              state: { permit: index > 0 },
              initial: 0,
              transitions: [],
              target: policy.target,
              runtime_route_verified: false,
            }
          : null,
      coverage: [],
    })),
  },
}
const onBuild = vi.fn()
function mount(readOnly = false) {
  return render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <NarrativeVariants
        projectKey="project"
        scene={scene}
        beatId="beat"
        readOnly={readOnly}
        onClose={vi.fn()}
        onBuild={onBuild}
      />
    </QueryClientProvider>,
  )
}
beforeEach(() => {
  vi.clearAllMocks()
  api.get.mockResolvedValue({ policy, capture: saved.policies, latest: saved, stale: false })
  api.plan.mockResolvedValue(saved)
  api.materialize.mockResolvedValue({
    before: { scene, slug: 'hearing', rel: 'narrative/scenes/hearing.yaml', stamp: null },
    after: {
      scene: { ...scene, summary: 'changed' },
      slug: 'hearing',
      rel: 'narrative/scenes/hearing.yaml',
      stamp: null,
    },
  })
  api.build.mockResolvedValue({ id: 'build' })
})
it('reopens canonical counts and unknown rows without planning or provider actions', async () => {
  mount()
  expect(
    await screen.findByText('Potential 3 · Included 2 · Excluded 0 · Unknown 1'),
  ).toBeInTheDocument()
  expect(screen.getByLabelText('Select configuration unknown')).toBeDisabled()
  expect(api.plan).not.toHaveBeenCalled()
  expect(api.materialize).not.toHaveBeenCalled()
  expect(api.build).not.toHaveBeenCalled()
})
it('materializes only the explicit included subset before opening the shared build', async () => {
  const push = vi.spyOn(useUndoStack.getState(), 'push')
  mount()
  fireEvent.click(await screen.findByLabelText('Select configuration b'))
  fireEvent.click(screen.getByRole('button', { name: 'Materialize 1 variants and open Build' }))
  await waitFor(() => expect(onBuild).toHaveBeenCalledWith('build'))
  expect(api.materialize).toHaveBeenCalledWith('report', 'slot', ['b'])
  expect(api.build).toHaveBeenCalledWith('report', 'slot', ['b'])
  expect(push).toHaveBeenCalled()
  expect(api.materialize.mock.invocationCallOrder[0]).toBeLessThan(
    api.build.mock.invocationCallOrder[0]!,
  )
})
it('keeps a source conflict visible and never prepares generation after failed materialization', async () => {
  api.materialize.mockRejectedValue(new Error('Sources changed; replan'))
  mount()
  fireEvent.click(await screen.findByRole('button', { name: 'Select all included (2)' }))
  fireEvent.click(screen.getByRole('button', { name: 'Materialize 2 variants and open Build' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('Sources changed; replan')
  expect(api.build).not.toHaveBeenCalled()
})
it('disables writes in read-only projects and historical stale matrices', async () => {
  api.get.mockResolvedValue({ policy, capture: saved.policies, latest: saved, stale: true })
  mount(true)
  expect(await screen.findByText(/Sources or policy changed since/)).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Save policy and plan' })).toBeDisabled()
  expect(screen.getByLabelText('Select configuration a')).toBeDisabled()
  expect(screen.getByRole('button', { name: /Materialize 0/ })).toBeDisabled()
})
it('paginates the matrix while whole-plan selection includes off-page eligible rows', async () => {
  const large = structuredClone(saved)
  large.report.rows = Array.from({ length: 61 }, (_, i) => ({
    ...saved.report.rows[0]!,
    id: String(i),
  }))
  api.get.mockResolvedValue({ policy, capture: saved.policies, latest: large, stale: false })
  mount()
  expect(await screen.findByText('Page 1 of 3')).toBeInTheDocument()
  expect(screen.getAllByRole('checkbox')).toHaveLength(25)
  fireEvent.click(screen.getByRole('button', { name: 'Select all included (61)' }))
  expect(screen.getByText(/Generation count: 61/)).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Next page' }))
  expect(screen.getByLabelText('Select configuration 25')).toBeChecked()
  expect(screen.queryByLabelText('Select configuration 0')).not.toBeInTheDocument()
})
