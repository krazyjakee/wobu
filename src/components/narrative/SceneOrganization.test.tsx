import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, renderHook, screen } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, expect, it, vi } from 'vitest'
import { SceneOrganization } from './SceneOrganization'
import { sessionFile } from './sceneSession.test-support'
import { useSceneEditSession } from './useSceneEditSession'
import { useScriptDrafts } from './scriptDrafts'
import { resetNarrativeDraftGuards } from '../../lib/narrativeDraftGuard'
import { useUI } from '../../store/ui'
import { qk } from '../../lib/queries/keys'
const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
beforeEach(() => {
  useScriptDrafts.setState({ drafts: {} })
  resetNarrativeDraftGuards()
  invoke.mockReset()
  useUI.setState({ narrativeReveal: null })
})
it('shares classifications with prose drafts, undoes locally, and saves once with the original guard', async () => {
  const file = sessionFile()
  const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } })
  client.setQueryData(qk.projectCurrent, { path: '/project' })
  client.setQueryData(['narrative_world'], {
    document: {
      acts: [{ id: 'arrival', name: 'Arrival' }],
      arcs: [{ id: 'inquiry', name: 'Inquiry' }],
      tags: [{ id: 'politics', name: 'Politics' }],
    },
  })
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  const shared = renderHook(() => useSceneEditSession(file, '/project'), { wrapper })
  render(<SceneOrganization file={file} projectKey="/project" readOnly={false} />, { wrapper })
  for (const field of ['act', 'arc', 'tags'] as const) {
    act(() =>
      useUI.getState().selectNarrative({ sceneId: file.scene.id, field }, 'diagnostic', {
        projectKey: '/project',
      }),
    )
    expect(
      screen.getByLabelText(field === 'tags' ? 'Tags' : field === 'act' ? 'Act' : 'Arc'),
    ).toHaveFocus()
  }
  fireEvent.change(screen.getByLabelText('Act'), { target: { value: 'arrival' } })
  fireEvent.change(screen.getByLabelText('Arc'), { target: { value: 'inquiry' } })
  expect(shared.result.current.scene).toMatchObject({ act_id: 'arrival', arc_id: 'inquiry' })
  expect(invoke).not.toHaveBeenCalled()
  act(() => shared.result.current.undo())
  expect(shared.result.current.scene.arc_id).toBeUndefined()
  act(() => shared.result.current.redo())
  expect(shared.result.current.scene.arc_id).toBe('inquiry')
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  invoke.mockImplementation(async (_cmd, args) => ({ ...file, scene: args.scene }))
  await act(() => shared.result.current.save())
  expect(invoke).toHaveBeenCalledTimes(1)
  expect(invoke).toHaveBeenCalledWith(
    'narrative_scene_save',
    expect.objectContaining({
      expected: { kind: 'stamp', stamp: file.stamp },
      scene: expect.objectContaining({ act_id: 'arrival', arc_id: 'inquiry' }),
    }),
  )
})
