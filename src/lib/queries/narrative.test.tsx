import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Layout, Scene, SceneFile } from '../api'
import { applyCommand, useUndoStack } from '../undo'
import { qk } from './keys'
import {
  useCreateScene,
  useDeleteScene,
  useDiagnoseScene,
  useNarrativeState,
  useRenameScene,
  useSaveLayout,
  useSaveNarrativeState,
  useSaveScene,
  useScene,
  useSceneDiagnostics,
  useSceneLayout,
  useScenes,
} from './narrative'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }))

/** `call()` refuses outside the Tauri webview, so say we are inside one. */
beforeEach(() => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  h.invoke.mockReset()
  useUndoStack.setState({ projectId: 'proj', past: [], future: [], busy: false })
})

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  )
  return { qc, Wrapper }
}

function scene(over: Partial<Scene> & { id: string }): Scene {
  return { name: 'Council hearing', ...over }
}

function file(over: Partial<SceneFile> & { scene: Scene }): SceneFile {
  return {
    slug: 'council-hearing',
    rel: 'narrative/scenes/council-hearing.yaml',
    stamp: { mtime_ms: 1, size: 2, hash: 'first' },
    ...over,
  }
}

/** Every `invoke` this test saw, as `[command, args]` pairs. */
function calls(): Array<[string, Record<string, unknown>]> {
  return h.invoke.mock.calls as Array<[string, Record<string, unknown>]>
}

function argsOf(command: string): Record<string, unknown> | undefined {
  return calls().find(([name]) => name === command)?.[1]
}

describe('the guarded-write precondition, end to end', () => {
  it('sends the stamp the editor loaded, and caches the one the save returned', async () => {
    // The whole chain: the file the editor holds carries the precondition, the
    // save presents it, and the answer replaces it. Nothing derives a stamp and
    // nothing keeps a second copy — a cache that could go stale in that role
    // turns a detected conflict into a silent overwrite.
    const before = file({ scene: scene({ id: 's1' }) })
    const after = file({
      scene: scene({ id: 's1', summary: 'edited' }),
      stamp: { mtime_ms: 9, size: 3, hash: 'second' },
    })
    h.invoke.mockResolvedValue(after)

    const { qc, Wrapper } = wrapper()
    const { result } = renderHook(() => useSaveScene(), { wrapper: Wrapper })
    result.current.mutate({ file: before, scene: after.scene })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    expect(argsOf('narrative_scene_save')).toMatchObject({
      expected: { kind: 'stamp', stamp: before.stamp },
      slug: before.slug,
    })
    expect(qc.getQueryData(qk.narrativeScene('s1'))).toEqual(after)
  })

  it('says a file is new rather than sending no precondition at all', async () => {
    // "I hold no stamp" and "whatever is there now" are opposite intentions,
    // and only one of them is allowed to overwrite.
    const fresh = file({ scene: scene({ id: 's1' }), stamp: null })
    h.invoke.mockResolvedValue(fresh)

    const { Wrapper } = wrapper()
    const { result } = renderHook(() => useSaveScene(), { wrapper: Wrapper })
    result.current.mutate({ file: fresh, scene: fresh.scene })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    expect(argsOf('narrative_scene_save')).toMatchObject({ expected: { kind: 'new' } })
  })
})

describe('the one choke point for a narrative edit', () => {
  it('records a save without the caller knowing undo exists', async () => {
    const before = file({ scene: scene({ id: 's1' }) })
    const after = file({ scene: scene({ id: 's1', name: 'The hearing' }) })
    h.invoke.mockResolvedValue(after)

    const { Wrapper } = wrapper()
    const { result } = renderHook(() => useSaveScene(), { wrapper: Wrapper })
    result.current.mutate({ file: before, scene: after.scene })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    const [entry] = useUndoStack.getState().past
    expect(entry?.label).toContain('rename')
    expect(entry?.undo).toEqual([{ type: 'sceneSave', scene: before.scene, slug: before.slug }])
  })

  it('records a rename made from a row that never held the document', async () => {
    // The Library has an id and a name. Reading the scene first is what keeps
    // the entry a real previous version rather than an invented one.
    const before = file({ scene: scene({ id: 's1' }) })
    const after = file({ scene: scene({ id: 's1', name: 'The hearing' }) })
    h.invoke.mockImplementation((command: string) =>
      Promise.resolve(command === 'narrative_scene_get' ? before : after),
    )

    const { Wrapper } = wrapper()
    const { result } = renderHook(() => useRenameScene(), { wrapper: Wrapper })
    result.current.mutate({ sceneId: 's1', name: 'The hearing' })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    expect(argsOf('narrative_scene_rename')).toEqual({ sceneId: 's1', name: 'The hearing' })
    expect(useUndoStack.getState().past[0]?.undo).toEqual([
      { type: 'sceneSave', scene: before.scene, slug: before.slug },
    ])
  })

  it('goes ahead unrecorded when the previous version cannot be read', async () => {
    // An undo that restores a guess is worse than no undo — but a rename that
    // refused because a read failed would be worse than both.
    const after = file({ scene: scene({ id: 's1', name: 'The hearing' }) })
    h.invoke.mockImplementation((command: string) =>
      command === 'narrative_scene_get'
        ? Promise.reject(new Error('gone'))
        : Promise.resolve(after),
    )

    const { Wrapper } = wrapper()
    const { result } = renderHook(() => useRenameScene(), { wrapper: Wrapper })
    result.current.mutate({ sceneId: 's1', name: 'The hearing' })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    expect(useUndoStack.getState().past).toEqual([])
  })

  it('records a create that inverts to a delete, and a delete that restores the document', async () => {
    const made = file({ scene: scene({ id: 's1' }) })
    h.invoke.mockResolvedValue(made)

    const { Wrapper } = wrapper()
    const created = renderHook(() => useCreateScene(), { wrapper: Wrapper })
    created.result.current.mutate('Council hearing')
    await waitFor(() => expect(created.result.current.isSuccess).toBe(true))
    expect(useUndoStack.getState().past[0]?.undo).toEqual([{ type: 'sceneDelete', id: 's1' }])

    useUndoStack.setState({ past: [], future: [] })
    h.invoke.mockImplementation((command: string) =>
      Promise.resolve(command === 'narrative_scene_get' ? made : undefined),
    )
    const deleted = renderHook(() => useDeleteScene(), { wrapper: Wrapper })
    deleted.result.current.mutate('s1')
    await waitFor(() => expect(deleted.result.current.isSuccess).toBe(true))

    const [entry] = useUndoStack.getState().past
    expect(entry?.undo).toEqual([{ type: 'sceneSave', scene: made.scene, slug: made.slug }])
    // The layout sidecar went with the scene and no inverse brings it back.
    expect(entry?.caveat).toBeTruthy()
  })

  it('records nothing for a save that changed nothing but the stamp', async () => {
    const before = file({ scene: scene({ id: 's1' }) })
    h.invoke.mockResolvedValue({ ...before, stamp: { mtime_ms: 9, size: 2, hash: 'second' } })

    const { Wrapper } = wrapper()
    const { result } = renderHook(() => useSaveScene(), { wrapper: Wrapper })
    result.current.mutate({ file: before, scene: before.scene })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    expect(useUndoStack.getState().past).toEqual([])
  })
})

describe('arrangement is not the world', () => {
  const layout = (): Layout => ({
    schemaVersion: 1,
    graph: { kind: 'scene', scene: 's1' },
    mode: 'manual',
    modeUpdatedAt: '2026-01-01T00:00:00Z',
    nodes: { 'beat:b1': { x: 120, y: 40, updatedAt: '2026-01-01T00:00:00Z' } },
    groups: {},
    annotations: {},
  })

  it('puts no entry on the undo stack for a drag', async () => {
    // Moving a box is not a story change. A ⌘Z that rewound a drag would, on
    // the very next press, rewind a paragraph — and nothing on screen tells the
    // writer which press they are about to make.
    h.invoke.mockResolvedValue({ outcome: 'written' })

    const { qc, Wrapper } = wrapper()
    const { result } = renderHook(() => useSaveLayout(), { wrapper: Wrapper })
    result.current.mutate(layout())
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    expect(useUndoStack.getState().past).toEqual([])
    expect(qc.getQueryData(qk.narrativeLayout({ kind: 'scene', scene: 's1' }))).toEqual({
      layout: layout(),
      notices: [],
    })
  })

  it('does not touch a single narrative source query', async () => {
    // The proof that a layout write cannot disturb a source read: no scene, no
    // catalog and no diagnostic query is invalidated by one.
    h.invoke.mockResolvedValue({ outcome: 'written' })
    const { qc, Wrapper } = wrapper()
    const invalidate = vi.spyOn(qc, 'invalidateQueries')

    const { result } = renderHook(() => useSaveLayout(), { wrapper: Wrapper })
    result.current.mutate(layout())
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    expect(invalidate).not.toHaveBeenCalled()
    expect(calls().map(([name]) => name)).toEqual(['narrative_layout_save'])
  })

  it('treats a refused arrangement as information rather than a failure', async () => {
    // The canvas autosaves one of these per drag. An outcome the caller reads
    // is a line in the corner; a rejected promise is a toast per rectangle.
    h.invoke.mockResolvedValue({ outcome: 'unwritable', reason: 'the folder is read-only' })

    const { Wrapper } = wrapper()
    const { result } = renderHook(() => useSaveLayout(), { wrapper: Wrapper })
    result.current.mutate(layout())
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    expect(result.current.isError).toBe(false)
    expect(result.current.data).toMatchObject({ outcome: 'unwritable' })
  })
})

describe('reads', () => {
  it('caches a loaded scene under the key every write updates', async () => {
    // The continuity that makes the precondition work: the file a reader put in
    // the cache and the file a writer replaces it with are the same entry, so
    // there is never a second copy holding an older stamp.
    const loaded = file({ scene: scene({ id: 's1' }) })
    h.invoke.mockResolvedValue(loaded)

    const { qc, Wrapper } = wrapper()
    const { result } = renderHook(() => useScene('s1'), { wrapper: Wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    expect(argsOf('narrative_scene_get')).toEqual({ sceneId: 's1' })
    expect(qc.getQueryData(qk.narrativeScene('s1'))).toEqual(loaded)
  })

  it('asks for nothing until there is something to ask about', async () => {
    const { Wrapper } = wrapper()
    renderHook(() => useScene(null), { wrapper: Wrapper })
    renderHook(() => useSceneDiagnostics(null), { wrapper: Wrapper })
    renderHook(() => useSceneLayout(null), { wrapper: Wrapper })
    await waitFor(() => expect(h.invoke).not.toHaveBeenCalled())
  })

  it('lists scenes and the files that could not be identified', async () => {
    h.invoke.mockResolvedValue({
      scenes: [{ id: 's1', name: 'Council hearing', slug: 'council-hearing', rel: 'r' }],
      unreadable: [{ rel: 'narrative/scenes/broken.yaml', reason: 'truncated' }],
    })
    const { Wrapper } = wrapper()
    const { result } = renderHook(() => useScenes(), { wrapper: Wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.unreadable).toHaveLength(1)
  })

  it('reads the saved scene for the Validation list, and the buffer on demand', async () => {
    // Two different questions. The list wants what is on disk; an editor with
    // unsaved edits wants its own document, and asking the cached question
    // would report a destination as broken after the writer connected it.
    h.invoke.mockResolvedValue([])
    const { Wrapper } = wrapper()

    const saved = renderHook(() => useSceneDiagnostics('s1'), { wrapper: Wrapper })
    await waitFor(() => expect(saved.result.current.isSuccess).toBe(true))
    expect(argsOf('narrative_diagnostics')).toEqual({ sceneId: 's1', scene: null })

    h.invoke.mockReset()
    h.invoke.mockResolvedValue([])
    const draft = scene({ id: 's1', summary: 'not saved yet' })
    const live = renderHook(() => useDiagnoseScene(), { wrapper: Wrapper })
    live.result.current.mutate({ sceneId: 's1', scene: draft })
    await waitFor(() => expect(live.result.current.isSuccess).toBe(true))
    expect(argsOf('narrative_diagnostics')).toEqual({ sceneId: 's1', scene: draft })
  })

  it('loads a layout under a key of its own per graph', async () => {
    const loaded = {
      layout: {
        schemaVersion: 1,
        graph: { kind: 'scene' as const, scene: 's1' },
        mode: 'automatic' as const,
        modeUpdatedAt: '1970-01-01T00:00:00Z',
        nodes: {},
        groups: {},
        annotations: {},
      },
      notices: [{ kind: 'missing' as const, blocking: false, rel: 'x' }],
    }
    h.invoke.mockResolvedValue(loaded)

    const { qc, Wrapper } = wrapper()
    const graph = { kind: 'scene' as const, scene: 's1' }
    const { result } = renderHook(() => useSceneLayout(graph), { wrapper: Wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    // A notice is information beside a usable layout, never a failed read.
    expect(result.current.isError).toBe(false)
    expect(qc.getQueryData(qk.narrativeLayout(graph))).toEqual(loaded)
  })
})

describe('declared state', () => {
  const document = { schema_version: 1, variables: [] }

  it('reads the declared variables', async () => {
    h.invoke.mockResolvedValue({ document, stamp: null })
    const { Wrapper } = wrapper()
    const { result } = renderHook(() => useNarrativeState(), { wrapper: Wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.stamp).toBeNull()
  })

  it('reruns every diagnostic when the variables change, and records no undo', async () => {
    // Every condition and effect in the project is typed against these, so what
    // is wrong with a scene just changed in files nobody has open. Undo is
    // deliberately absent: restoring a declaration would leave scenes written
    // since then failing to type, in files the user never opened.
    h.invoke.mockResolvedValue({ document, stamp: { mtime_ms: 1, size: 1, hash: 'h' } })
    const { qc, Wrapper } = wrapper()
    const invalidate = vi.spyOn(qc, 'invalidateQueries')

    const { result } = renderHook(() => useSaveNarrativeState(), { wrapper: Wrapper })
    result.current.mutate({ document, expected: { kind: 'new' } })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))

    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['narrative_diagnostics'] })
    expect(useUndoStack.getState().past).toEqual([])
  })
})

describe('running an undo against the backend', () => {
  it('asks for the version on disk rather than the one it recorded', async () => {
    // The same guarantee `node_upsert` gives — it reads its precondition out of
    // the index — rather than a weaker one. An entry recorded before three
    // later saves would otherwise present a precondition three versions stale
    // and park itself as a conflict: a ⌘Z that fails on every press but the
    // first.
    h.invoke.mockResolvedValue(file({ scene: scene({ id: 's1' }) }))
    await applyCommand({
      type: 'sceneSave',
      scene: scene({ id: 's1' }),
      slug: 'council-hearing',
    })

    expect(argsOf('narrative_scene_save')).toMatchObject({
      expected: { kind: 'current' },
      slug: 'council-hearing',
    })
  })

  it('undoes a create through the delete command, not through a second create', async () => {
    h.invoke.mockResolvedValue(undefined)
    await applyCommand({ type: 'sceneDelete', id: 's1' })
    expect(calls()).toEqual([['narrative_scene_delete', { sceneId: 's1' }]])
  })
})
