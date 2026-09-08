import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, expect, it, vi } from 'vitest'
import { projectClose, projectCreate, projectCurrent, projectOpen } from '../api/project'
import { projectSessionEpoch } from '../projectSession'
import { useSaveNarrativeState } from './narrative'
import { useUndoStack } from '../undo'
import { qk } from './keys'
const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
beforeEach(() => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  h.invoke.mockReset()
})
it('advances only at activation/close attempts, including failed attempts', async () => {
  const initial = projectSessionEpoch()
  h.invoke.mockResolvedValue(null)
  await projectCurrent()
  expect(projectSessionEpoch()).toBe(initial)
  await projectClose()
  await projectOpen('/same')
  expect(projectSessionEpoch()).toBe(initial + 2)
  h.invoke.mockRejectedValue(new Error('Permission denied'))
  await expect(projectCreate('/parent', 'Example')).rejects.toBeDefined()
  expect(projectSessionEpoch()).toBe(initial + 3)
})
it('refuses a state save after an async undo-base read crosses an actual same-project close/open', async () => {
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } })
  client.setQueryData(qk.projectCurrent, { path: '/same', id: 'same-id' })
  useUndoStack.setState({ projectId: 'same-id', past: [], future: [], busy: false })
  const file = { document: { schema_version: 1, variables: [] }, stamp: null }
  let release!: () => void
  h.invoke.mockImplementation(async (command: string) => {
    if (command === 'narrative_state_get') {
      await new Promise<void>((resolve) => {
        release = resolve
      })
      return file
    }
    if (command === 'project_close' || command === 'project_open') return null
    throw new Error(`Unexpected write: ${command}`)
  })
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  const hook = renderHook(() => useSaveNarrativeState('/same'), { wrapper })
  let result!: Promise<unknown>
  act(() => {
    result = hook.result.current
      .mutateAsync({ document: file.document, expected: { kind: 'new' } })
      .catch((error: unknown) => error)
  })
  await waitFor(() => expect(release).toBeDefined())
  await projectClose()
  await projectOpen('/same')
  await act(async () => {
    release()
    expect(await result).toMatchObject({ message: expect.stringContaining('session changed') })
  })
  expect(h.invoke.mock.calls.map(([command]) => command)).toEqual([
    'narrative_state_get',
    'project_close',
    'project_open',
  ])
  expect(useUndoStack.getState().past).toEqual([])
  expect(client.getQueryData(qk.narrativeState)).toBeUndefined()
})
