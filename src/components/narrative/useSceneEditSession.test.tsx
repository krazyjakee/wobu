import { projectClose, projectOpen } from '../../lib/api/project'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, expect, it, vi } from 'vitest'
import { useNarrativeReviewApply } from '../../lib/queries/narrativeReview'
import { useUndoRunner } from '../../lib/queries/undo'
import { qk } from '../../lib/queries/keys'
import { useUndoStack } from '../../lib/undo'
import { useUI } from '../../store/ui'
import type { Scene } from '../../lib/api'
import { resetNarrativeDraftGuards } from '../../lib/narrativeDraftGuard'
import { useScriptDrafts } from './scriptDrafts'
import { useSceneEditSession } from './useSceneEditSession'
import { sessionFile } from './sceneSession.test-support'
const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))
beforeEach(() => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  useScriptDrafts.setState({ drafts: {} })
  resetNarrativeDraftGuards()
  useUndoStack.setState({ projectId: 'project', past: [], future: [], busy: false })
  useUI.getState().selectNarrative({ sceneId: 'scene', beatId: 'beat' }, 'script')
  h.invoke.mockReset()
  h.invoke.mockImplementation(async (command: string, args: { scene: Scene; body: string }) => {
    if (command === 'narrative_scene_save')
      return { ...sessionFile(), scene: args.scene, stamp: { hash: 'saved', size: 2, mtime_ms: 2 } }
    if (command === 'narrative_text_written') return { body: args.body, revision: 'written' }
    throw new Error(`Unexpected command: ${command}`)
  })
})
function setup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  client.setQueryData(qk.projectCurrent, { path: '/project' })
  const file = sessionFile()
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  return { client, file, wrapper }
}
it('shares prose and structural edits between consumers, with local undo and one explicit canonical save', async () => {
  const { file, wrapper } = setup()
  const script = renderHook(() => useSceneEditSession(file, '/project'), { wrapper })
  const flow = renderHook(() => useSceneEditSession(file, '/project'), { wrapper })
  const history = renderHook(useUndoRunner, { wrapper })
  const prose = structuredClone(file.scene)
  prose.beats![0]!.dialogue![0]!.variants![0]!.text.body = 'Handwritten revision'
  act(() => {
    script.result.current.edit(prose)
  })
  act(() => {
    flow.result.current.edit({ ...flow.result.current.scene, summary: 'A new route' })
  })
  expect(script.result.current.scene.summary).toBe('A new route')
  expect(h.invoke).not.toHaveBeenCalled()
  await act(async () => history.result.current.undo())
  expect(script.result.current.scene.summary).toBeUndefined()
  await act(async () => history.result.current.redo())
  expect(h.invoke).not.toHaveBeenCalled()
  script.unmount()
  await act(async () => flow.result.current.save())
  expect(h.invoke.mock.calls.map(([command]) => command)).toEqual([
    'narrative_text_written',
    'narrative_scene_save',
  ])
  const saved = useUndoStack.getState().past[0]!
  expect(saved.coalesce).toBe(false)
  expect(saved.redo[0]).toMatchObject({
    type: 'sceneSave',
    scene: {
      summary: 'A new route',
      beats: [
        {
          dialogue: [
            { variants: [{ text: { body: 'Handwritten revision', revision: 'written' } }] },
          ],
        },
      ],
    },
  })
  expect(flow.result.current.dirty).toBe(false)
})
it('never falls through to canonical undo after reaching the beginning of an unsaved draft', async () => {
  const { file, wrapper } = setup()
  const session = renderHook(() => useSceneEditSession(file, '/project'), { wrapper })
  const history = renderHook(useUndoRunner, { wrapper })
  useUndoStack.getState().push({
    subjectId: 'other',
    label: 'Earlier save',
    coalesce: false,
    undo: [{ type: 'sceneDelete', id: 'other' }],
    redo: [],
  })
  act(() => {
    session.result.current.edit({ ...file.scene, name: 'Draft' })
  })
  await act(async () => {
    await history.result.current.undo()
    await history.result.current.undo()
  })
  expect(session.result.current.scene.name).toBe(file.scene.name)
  expect(session.result.current.dirty).toBe(true)
  expect(h.invoke).not.toHaveBeenCalled()
  expect(useUndoStack.getState().past).toHaveLength(1)
})
it.each(['/other', '/project'])(
  'keeps late replies out after public close/open to %s, retaining the original draft',
  async (path) => {
    const { file, wrapper, client } = setup()
    let release!: () => void
    h.invoke.mockImplementationOnce(async (_command, args: { scene: Scene }) => {
      await new Promise<void>((resolve) => {
        release = resolve
      })
      return { ...file, scene: args.scene }
    })
    const session = renderHook(() => useSceneEditSession(file, '/project'), { wrapper })
    act(() => {
      session.result.current.edit({ ...file.scene, name: 'Submitted' })
    })
    let pending!: Promise<void>
    act(() => {
      pending = session.result.current.save()
    })
    await waitFor(() => expect(release).toBeDefined())
    h.invoke.mockImplementation(async () => undefined)
    await projectClose()
    await projectOpen(path)
    // The same-path case deliberately retains the real project and undo identities.
    client.setQueryData(qk.projectCurrent, { path })
    const other = { ...file, scene: { ...file.scene, name: 'Other project' } }
    client.setQueryData(qk.narrativeScene('scene'), other)
    await act(async () => {
      release()
      await pending
    })
    expect(client.getQueryData(qk.narrativeScene('scene'))).toBe(other)
    expect(useUndoStack.getState().past).toHaveLength(0)
    expect(session.result.current.draft).toMatchObject({
      scene: { name: 'Submitted' },
      pending: false,
      error: expect.stringContaining('previous project'),
    })
  },
)

it('rejects review writes while the shared authoring draft exists', async () => {
  const { file, wrapper } = setup()
  const session = renderHook(() => useSceneEditSession(file, '/project'), { wrapper })
  const review = renderHook(() => useNarrativeReviewApply('/project'), { wrapper })
  act(() => {
    session.result.current.edit({ ...file.scene, name: 'Draft' })
  })
  await act(async () => {
    await expect(
      review.result.current.mutateAsync({
        guard: { stamp: file.stamp, head: null },
        target: { scene: 'scene', beat: 'beat', slot: 'slot', variant: 'variant' },
        state_json: '{}',
        context_revision: 'context',
        action: { kind: 'approve' },
      }),
    ).rejects.toThrow('Save or discard the shared scene draft')
  })
  expect(h.invoke).not.toHaveBeenCalled()
})
