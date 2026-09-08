import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import type { Layout } from '../../../lib/api'
import { useFlowPresentation } from './useFlowPresentation'

const h = vi.hoisted(() => ({ save: vi.fn(), stored: {} as { data?: { layout: Layout } } }))
vi.mock('../../../lib/queries', () => ({
  useSceneLayout: () => h.stored,
  useSaveLayout: () => ({ mutateAsync: h.save }),
}))
const graph = { kind: 'arc' as const, arc: 'project' }
const initial: Layout = {
  schemaVersion: 2,
  graph,
  mode: 'manual',
  modeUpdatedAt: '2026-09-08T12:00:00Z',
  nodes: {},
  groups: {},
  annotations: {},
}
beforeEach(() => {
  h.save.mockReset()
  h.stored = { data: { layout: initial } }
})

it('serializes delayed saves and still saves a newer draft after an earlier failure', async () => {
  let reject!: (error: Error) => void
  h.save.mockImplementationOnce(
    () =>
      new Promise((_resolve, fail) => {
        reject = fail
      }),
  )
  h.save.mockImplementationOnce(async (layout: Layout) => {
    h.stored = { data: { layout } }
    return { outcome: 'written' }
  })
  const { result } = renderHook(() => useFlowPresentation(graph))
  act(() => result.current.presentation!.onChange({ ...initial, mode: 'automatic' }))
  act(() =>
    result.current.presentation!.onChange({ ...initial, modeUpdatedAt: '2026-09-08T13:00:00Z' }),
  )
  expect(h.save).toHaveBeenCalledTimes(1)
  await act(async () => reject(new Error('folder unavailable')))
  await waitFor(() => expect(h.save).toHaveBeenCalledTimes(2))
  expect(h.save.mock.calls[1]![0].modeUpdatedAt).toBe('2026-09-08T13:00:00Z')
  expect(result.current.presentation!.saveFailed).toBe(false)
})

it('keeps a failed draft for explicit retry without changing the loaded arrangement', async () => {
  h.save.mockRejectedValueOnce(new Error('folder unavailable'))
  const { result } = renderHook(() => useFlowPresentation(graph))
  act(() => result.current.presentation!.onChange({ ...initial, mode: 'automatic' }))
  await waitFor(() => expect(result.current.presentation!.saveFailed).toBe(true))
  expect(result.current.presentation!.layout.mode).toBe('automatic')
  expect(h.stored.data!.layout).toEqual(initial)
  h.save.mockImplementationOnce(async (layout: Layout) => {
    h.stored = { data: { layout } }
    return { outcome: 'written' }
  })
  act(() => result.current.presentation!.onChange(result.current.presentation!.layout))
  await waitFor(() => expect(result.current.presentation!.saveFailed).toBe(false))
  expect(h.stored.data!.layout.mode).toBe('automatic')
})
