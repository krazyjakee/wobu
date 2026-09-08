import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { useState } from 'react'
import { beforeEach, expect, it, vi } from 'vitest'
import type { SceneFile, StateFile } from '../../lib/api'
import { projectClose, projectOpen } from '../../lib/api'
import { useScene } from '../../lib/queries'
import { qk } from '../../lib/queries/keys'
import { useUndoStack } from '../../lib/undo'
import { NarrativeExample } from './NarrativeExample'
import { SceneEditControls } from './SceneEditControls'
import { useSceneEditSession } from './useSceneEditSession'
import { useScriptDrafts } from './scriptDrafts'
import { useWorldDrafts } from './worldDrafts'
import { ashfallScene, ashfallState } from './ashfallExample'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
const project = { id: 'project', name: 'Ashfall', path: '/project', readOnly: false }
let state: StateFile
let file: SceneFile
let client: QueryClient
const onOpen = vi.fn()

beforeEach(() => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  onOpen.mockReset()
  useScriptDrafts.setState({ drafts: {} })
  useWorldDrafts.setState({ world: {}, variables: {} })
  useUndoStack.setState({ projectId: project.id, past: [], future: [], busy: false })
  state = { document: { schema_version: 1, variables: [] }, stamp: null }
  file = {
    scene: { id: 'scene', name: 'Ashfall council hearing', beats: [] },
    slug: 'ashfall',
    rel: 'narrative/scenes/ashfall.yaml',
    stamp: null,
  }
  client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity }, mutations: { retry: false } },
  })
  client.setQueryData(qk.projectCurrent, project)
  h.invoke.mockReset()
  h.invoke.mockImplementation(async (command: string, value: Record<string, unknown>) => {
    switch (command) {
      case 'project_open':
        return project
      case 'project_close':
        return null
      case 'narrative_state_get':
        return structuredClone(state)
      case 'narrative_state_save':
        state = { ...state, document: value.document as StateFile['document'] }
        return structuredClone(state)
      case 'narrative_scene_create':
      case 'narrative_scene_get':
        return structuredClone(file)
      case 'narrative_text_written':
        return {
          revision: 'a'.repeat(64),
          body: value.body,
          provenance: 'human',
          lifecycle: { policy: 'edited', review: 'draft', freshness: 'current' },
        }
      case 'narrative_scene_save':
        file = { ...file, scene: value.scene as SceneFile['scene'] }
        return structuredClone(file)
      default:
        throw new Error(`Unexpected command ${command}`)
    }
  })
})
function Editing({ sceneId }: { sceneId: string }) {
  const query = useScene(sceneId)
  return query.data ? <Saved file={query.data} /> : <p>Loading saved example</p>
}
function Saved({ file }: { file: SceneFile }) {
  const session = useSceneEditSession(file, project.path)
  return (
    <>
      <p>{session.scene.name}</p>
      <SceneEditControls session={session} />
    </>
  )
}
function Harness() {
  const [sceneId, setSceneId] = useState<string | null>(null)
  return sceneId ? (
    <Editing sceneId={sceneId} />
  ) : (
    <NarrativeExample
      projectKey={project.path}
      readOnly={false}
      onClose={() => {}}
      onOpen={(id) => {
        onOpen(id)
        setSceneId(id)
      }}
    />
  )
}
function renderExample() {
  return render(
    <QueryClientProvider client={client}>
      <Harness />
    </QueryClientProvider>,
  )
}

it('creates a draft through public bridge commands, saves canonical wording, and reopens stable identities', async () => {
  const view = renderExample()
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Create example draft' })).toBeEnabled(),
  )
  fireEvent.click(screen.getByRole('button', { name: 'Create example draft' }))
  await waitFor(() => expect(onOpen).toHaveBeenCalledWith('scene'))
  expect(h.invoke.mock.calls.some(([command]) => command === 'narrative_scene_save')).toBe(false)
  fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
  await waitFor(() => expect(screen.getByRole('button', { name: 'Save scene' })).toBeDisabled())
  expect(file.scene.beats).toHaveLength(2)
  const choices = file.scene.beats![0]!.choices!
  expect(choices).toHaveLength(3)
  expect(choices[0]!.requires).toMatchObject({
    all: [{ compare: { var: 'ashfall_knows_logbook' } }, { compare: { var: 'ashfall_trust' } }],
  })
  expect(choices[0]).not.toHaveProperty('when')
  expect(choices.map((choice) => choice.to)).toEqual(
    Array(3).fill({ beat: file.scene.beats![1]!.id }),
  )
  expect(choices[0]!.effects).toContainEqual({ command: { name: 'ashfall_file_record', args: [] } })
  expect(state.document.variables).toHaveLength(3)
  expect(useUndoStack.getState().past).toHaveLength(3)
  const canonical = structuredClone(file)
  view.unmount()
  await projectClose()
  await projectOpen(project.path)
  client.clear()
  client.setQueryData(qk.projectCurrent, project)
  render(
    <QueryClientProvider client={client}>
      <Editing sceneId="scene" />
    </QueryClientProvider>,
  )
  await screen.findByText('Ashfall council hearing')
  expect(client.getQueryData(qk.narrativeScene('scene'))).toEqual(canonical)
  expect(h.invoke).toHaveBeenCalledWith('narrative_scene_get', { sceneId: 'scene' })
})

it('rejects incompatible declarations before any example writes and retains authored defaults', () => {
  const declarations = ashfallState(state.document)
  declarations.variables[0]!.default = 20
  expect(ashfallState(declarations).variables[0]!.default).toBe(20)
  expect(ashfallState(state.document).variables[0]!.default).toBe(40)
  expect(() =>
    ashfallState({ ...declarations, variables: [{ ...declarations.variables[0]!, type: 'bool' }] }),
  ).toThrow('different type or owner')
  const first = ashfallScene('one')
  const second = ashfallScene('two')
  expect(first.beats![0]!.id).not.toBe(second.beats![0]!.id)
})

it('keeps completed variables visible when scene creation fails', async () => {
  const normal = h.invoke.getMockImplementation()!
  h.invoke.mockImplementation((command, args) =>
    command === 'narrative_scene_create'
      ? Promise.reject(new Error('Disk unavailable'))
      : normal(command, args),
  )
  renderExample()
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Create example draft' })).toBeEnabled(),
  )
  fireEvent.click(screen.getByRole('button', { name: 'Create example draft' }))
  expect(await screen.findByText(/Disk unavailable.*Any completed writes/)).toBeInTheDocument()
  expect(screen.getByText(/example variables were saved/)).toBeInTheDocument()
  expect(state.document.variables).toHaveLength(3)
  expect(onOpen).not.toHaveBeenCalled()
})

it('does not create a scene after a same-path close and reopen during variable save', async () => {
  let release!: (value: StateFile) => void
  const normal = h.invoke.getMockImplementation()!
  h.invoke.mockImplementation((command, args) =>
    command === 'narrative_state_save'
      ? new Promise<StateFile>((resolve) => {
          release = resolve
        })
      : normal(command, args),
  )
  renderExample()
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Create example draft' })).toBeEnabled(),
  )
  fireEvent.click(screen.getByRole('button', { name: 'Create example draft' }))
  await waitFor(() => expect(release).toBeDefined())
  await act(async () => {
    await projectClose()
    await projectOpen(project.path)
    release(state)
  })
  await screen.findByRole('alert')
  expect(h.invoke.mock.calls.some(([command]) => command === 'narrative_scene_create')).toBe(false)
  expect(onOpen).not.toHaveBeenCalled()
})
