import type { ReactNode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { useNarrativeReviewApply } from './narrativeReview'
import { qk } from './keys'
import type { ReviewRequest } from '../api/narrativeReview'
const pending = vi.hoisted(() => vi.fn())
vi.mock('../api/narrativeReview', () => ({
  narrativeReviewApply: pending,
  narrativeReviewList: vi.fn(),
}))
it('does not install an old project scene when a review reply arrives after a project switch', async () => {
  const client = new QueryClient()
  let finish!: (result: unknown) => void
  pending.mockReturnValueOnce(
    new Promise((resolve) => {
      finish = resolve
    }),
  )
  client.setQueryData(qk.projectCurrent, { path: '/old' })
  const { result } = renderHook(() => useNarrativeReviewApply('/old'), {
    wrapper: ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  })
  const request: ReviewRequest = {
    guard: { stamp: null, head: 'old-head' },
    target: { scene: 'same-imported-id', beat: 'beat', slot: 'slot', variant: 'variant' },
    context_revision: 'old-context',
    state_json: '{}',
    action: { kind: 'approve' },
  }
  let saving!: Promise<unknown>
  await act(async () => {
    saving = result.current.mutateAsync(request)
    await Promise.resolve()
  })
  client.setQueryData(qk.projectCurrent, { path: '/new' })
  client.setQueryData(qk.narrativeScene('same-imported-id'), {
    scene: { id: 'same-imported-id', name: 'New project scene' },
  })
  await act(async () => {
    finish({
      file: { scene: { id: 'same-imported-id', name: 'Old project approved scene' } },
      review: {},
    })
    await saving
  })
  expect(client.getQueryData(qk.narrativeScene('same-imported-id'))).toEqual({
    scene: { id: 'same-imported-id', name: 'New project scene' },
  })
})
