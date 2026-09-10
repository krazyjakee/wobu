import { projectClose, projectOpen } from '../../lib/api/project'
import { qk } from '../../lib/queries/keys'
import { sceneEditKey, useScriptDrafts } from './scriptDrafts'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useUI } from '../../store/ui'
import { editorWrites } from '../../lib/editorWrites'
import { resetNarrativeDraftGuards } from '../../lib/narrativeDraftGuard'
import { NarrativeSourcePane } from './NarrativeSourcePane'
import type { NarrativeSource } from '../../lib/api/narrativeSource'

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  check: vi.fn(),
  save: vi.fn(),
  repair: vi.fn(),
  invoke: vi.fn(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('../../lib/api/narrativeSource', () => ({
  narrativeSourceGet: mocks.get,
  narrativeSourceOpen: mocks.get,
  narrativeSourceRepair: mocks.repair,
  narrativeSourceCheck: mocks.check,
}))
vi.mock('../../lib/queries', () => ({ useSaveScene: () => ({ mutateAsync: mocks.save }) }))

const original = {
  rel: 'narrative/scenes/council.yaml',
  stamp: { mtime_ms: 1, size: 5, hash: 'original' },
  sceneId: 'scene-1',
  repairBlocked: false,
  problem: null,
  yaml: 'schema_version: 1\nscene: original\n',
  file: {
    scene: { id: 'scene-1', name: 'Council' },
    slug: 'council',
    rel: 'narrative/scenes/council.yaml',
    stamp: { mtime_ms: 1, size: 5, hash: 'original' },
  },
} satisfies NarrativeSource

function mount(
  qc = new QueryClient({ defaultOptions: { queries: { retry: false } } }),
  readOnly = false,
  projectKey = 'project-a',
) {
  qc.setQueryData(qk.projectCurrent, { path: projectKey })
  return {
    qc,
    ...render(
      <QueryClientProvider client={qc}>
        <NarrativeSourcePane readOnly={readOnly} projectKey={projectKey} />
      </QueryClientProvider>,
    ),
  }
}

afterEach(resetNarrativeDraftGuards)

beforeEach(() => {
  useScriptDrafts.setState({ drafts: {} })
  resetNarrativeDraftGuards()
  vi.resetAllMocks()
  useUI.setState({ narrative: { sceneId: 'scene-1', beatId: null, lineId: null } })
  mocks.get.mockResolvedValue(original)
  mocks.check.mockResolvedValue({
    scene: original.file.scene,
    formatted: 'formatted',
    problem: null,
    diagnostics: [],
  })
  mocks.save.mockResolvedValue(original.file)
})

it('retains a dirty draft and its original precondition after unmount and a form edit', async () => {
  const { unmount, qc } = mount()
  fireEvent.change(await screen.findByRole('textbox', { name: 'Scene YAML' }), {
    target: { value: 'draft' },
  })
  unmount()
  mocks.get.mockResolvedValue({
    ...original,
    yaml: 'changed outside',
    file: { ...original.file, stamp: { ...original.file.stamp, hash: 'newer' } },
    stamp: { ...original.stamp, hash: 'newer' },
  })
  mount(qc)
  expect(await screen.findByRole('textbox', { name: 'Scene YAML' })).toHaveValue('draft')
  fireEvent.click(screen.getByRole('button', { name: 'Save source' }))
  await waitFor(() =>
    expect(mocks.save).toHaveBeenCalledWith({ file: original.file, scene: original.file.scene }),
  )
})

it('keeps invalid drafts local and offers the located syntax error', async () => {
  mocks.check.mockResolvedValue({
    scene: null,
    formatted: null,
    diagnostics: [],
    problem: { message: 'Unknown field at line 2', location: { line: 2, column: 2 } },
  })
  mount()
  fireEvent.change(await screen.findByRole('textbox', { name: 'Scene YAML' }), {
    target: { value: 'bad\nfield' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Save source' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('Unknown field')
  expect(mocks.save).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: 'Go to error' }))
  expect((screen.getByRole('textbox') as HTMLTextAreaElement).selectionStart).toBe(5)
})

it('requires explicit draft discard before reload and preserves edits after a conflict', async () => {
  mocks.save.mockRejectedValue(new Error('Scene changed; your copy is in a conflict sibling.'))
  mount()
  const editor = await screen.findByRole('textbox')
  fireEvent.change(editor, { target: { value: 'draft' } })
  fireEvent.click(screen.getByRole('button', { name: 'Save source' }))
  expect(await screen.findByRole('status')).toHaveTextContent('conflict sibling')
  expect(editor).toHaveValue('draft')
  mocks.get.mockResolvedValue({ ...original, yaml: 'latest' })
  fireEvent.click(screen.getByRole('button', { name: 'Reload' }))
  expect(editor).toHaveValue('draft')
  fireEvent.click(screen.getByRole('button', { name: 'Discard draft and reload' }))
  await waitFor(() => expect(editor).toHaveValue('latest'))
})

it('scopes drafts to the project and disables mutations on a read-only project', async () => {
  const { qc, unmount } = mount()
  fireEvent.change(await screen.findByRole('textbox'), { target: { value: 'project a draft' } })
  unmount()
  mount(qc, true, 'project-b')
  expect(await screen.findByRole('textbox')).toHaveValue(original.yaml)
  expect(screen.getByRole('textbox')).toHaveAttribute('readonly')
  expect(screen.getByRole('button', { name: 'Format' })).toBeDisabled()
  expect(screen.getByRole('button', { name: 'Save source' })).toBeDisabled()
  expect(screen.getByRole('button', { name: 'Validate' })).toBeEnabled()
})

it('formats only the draft and links semantic diagnostics to the shared beat selection', async () => {
  mocks.check.mockResolvedValue({
    scene: original.file.scene,
    formatted: 'normalized YAML',
    problem: null,
    diagnostics: [
      {
        code: 'dangling_destination',
        message: 'Choose a destination.',
        beatId: 'beat-1',
        slotId: 'line-1',
      },
    ],
  })
  mount()
  await screen.findByRole('textbox')
  fireEvent.click(screen.getByRole('button', { name: 'Format' }))
  await waitFor(() => expect(screen.getByRole('textbox')).toHaveValue('normalized YAML'))
  expect(mocks.save).not.toHaveBeenCalled()
  expect(screen.getByText('Unsaved draft')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Open Script' }))
  expect(useUI.getState().narrative).toEqual({
    sceneId: 'scene-1',
    beatId: 'beat-1',
    lineId: 'line-1',
  })
  expect(useUI.getState().narrativeTab).toBe('script')
})

it('blocks project close after an unsaved Source pane unmounts until explicitly discarded', async () => {
  const { qc, unmount } = mount(undefined, false, 'close-project')
  fireEvent.change(await screen.findByRole('textbox'), { target: { value: 'unsaved source' } })
  unmount()
  await expect(editorWrites.flushAll()).rejects.toThrow('Editor writes did not settle')
  mount(qc, false, 'close-project')
  await screen.findByRole('textbox')
  fireEvent.click(screen.getByRole('button', { name: 'Reload' }))
  fireEvent.click(screen.getByRole('button', { name: 'Discard draft and reload' }))
  await waitFor(() => expect(screen.getByRole('textbox')).toHaveValue(original.yaml))
  await expect(editorWrites.flushAll()).resolves.toBeUndefined()
})

it('does not clear a newer draft when an earlier save finishes after a tab remount', async () => {
  let finish!: (file: NarrativeSource['file']) => void
  mocks.save.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve
      }),
  )
  const { qc, unmount } = mount()
  fireEvent.change(await screen.findByRole('textbox'), { target: { value: 'first draft' } })
  fireEvent.click(screen.getByRole('button', { name: 'Save source' }))
  await waitFor(() => expect(mocks.save).toHaveBeenCalled())
  unmount()
  mount(qc)
  fireEvent.change(await screen.findByRole('textbox'), { target: { value: 'newer draft' } })
  await act(async () => {
    finish(original.file)
  })
  expect(screen.getByRole('textbox')).toHaveValue('newer draft')
  await expect(editorWrites.flushAll()).rejects.toThrow('Editor writes did not settle')
  expect(qc.getQueryData(['narrative_source_draft', 'project-a', 'scene-1'])).toMatchObject({
    yaml: 'newer draft',
    base: original,
  })
})

it('reloads a clean source view after another editor saves the scene', async () => {
  const { qc, unmount } = mount()
  await screen.findByRole('textbox')
  unmount()
  mocks.get.mockResolvedValue({
    ...original,
    yaml: 'latest form source',
    file: { ...original.file, stamp: { mtime_ms: 2, size: 8, hash: 'form-write' } },
    stamp: { mtime_ms: 2, size: 8, hash: 'form-write' },
  })
  mount(qc)
  await waitFor(() => expect(screen.getByRole('textbox')).toHaveValue('latest form source'))
})

it('repairs already malformed disk source using its original path and stamp', async () => {
  const malformed: NarrativeSource = {
    ...original,
    file: null,
    yaml: 'schema_version: 1\nscene: [',
    problem: { message: 'Unclosed sequence', location: { line: 2, column: 8 } },
  }
  mocks.get.mockResolvedValue(malformed)
  mocks.repair.mockResolvedValue({
    source: original,
    recoveryRel: 'narrative/recovery/original.yaml',
  })
  mount()
  expect(await screen.findByRole('textbox')).toHaveValue(malformed.yaml)
  fireEvent.change(screen.getByRole('textbox'), { target: { value: original.yaml } })
  fireEvent.click(screen.getByRole('button', { name: 'Save source' }))
  await waitFor(() => expect(mocks.repair).toHaveBeenCalledWith(malformed, original.yaml))
  expect(mocks.save).not.toHaveBeenCalled()
  await waitFor(() => expect(screen.getByRole('textbox')).toHaveValue(original.yaml))
})

it('does not resurrect a reverted draft when an earlier format finishes after remount', async () => {
  let finish!: (value: unknown) => void
  mocks.check.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve
      }),
  )
  const view = mount()
  fireEvent.change(await screen.findByRole('textbox'), { target: { value: 'format this' } })
  fireEvent.click(screen.getByRole('button', { name: 'Format' }))
  await waitFor(() => expect(finish).toBeDefined())
  view.unmount()
  mount(view.qc)
  fireEvent.change(await screen.findByRole('textbox'), { target: { value: original.yaml } })
  await act(async () =>
    finish({
      scene: original.file.scene,
      formatted: 'stale formatting',
      problem: null,
      diagnostics: [],
    }),
  )
  expect(screen.getByRole('textbox')).toHaveValue(original.yaml)
  expect(screen.getByRole('button', { name: 'Save source' })).toBeDisabled()
})

it('selects a semantic diagnostic’s exact source range and invalidates it after typing', async () => {
  const yaml = 'scene: {beats: [{choices: [{to: {beat: missing}}]}]}\n'
  mocks.get.mockResolvedValue({ ...original, yaml })
  mocks.check.mockResolvedValue({
    scene: original.file.scene,
    formatted: yaml,
    problem: null,
    diagnostics: [
      {
        kind: 'choice',
        code: 'dangling_beat',
        message: 'Missing beat',
        destination: true,
        beatId: 'b',
        choiceId: 'c',
        sourcePath: ['scene', 'beats', 0, 'choices', 0, 'to', 'beat'],
      },
    ],
  })
  mount()
  await screen.findByRole('textbox')
  fireEvent.click(screen.getByRole('button', { name: 'Validate' }))
  fireEvent.click(await screen.findByRole('button', { name: /^Source 1:/ }))
  const editor = screen.getByRole('textbox') as HTMLTextAreaElement
  expect(editor.value.slice(editor.selectionStart, editor.selectionEnd)).toBe('missing')
  fireEvent.change(editor, { target: { value: yaml + '# changed' } })
  expect(screen.queryByRole('button', { name: /^Source 1:/ })).not.toBeInTheDocument()
})

it('keeps unsupported saved schemas visible but refuses destructive downgrade controls', async () => {
  mocks.get.mockResolvedValue({
    ...original,
    file: null,
    yaml: 'schema_version: 999\nscene: future\n',
    repairBlocked: true,
    problem: { message: 'Unsupported schema version 999', location: null },
  })
  mount()
  expect(await screen.findByRole('textbox')).toHaveValue('schema_version: 999\nscene: future\n')
  expect(screen.getByRole('textbox')).toHaveAttribute('readonly')
  expect(screen.getByRole('button', { name: 'Format' })).toBeDisabled()
  expect(screen.getByRole('button', { name: 'Save source' })).toBeDisabled()
  expect(screen.getByRole('button', { name: 'Validate' })).toBeEnabled()
})

it('keeps raw Source readable but prevents writes while a shared scene draft exists', async () => {
  useScriptDrafts.getState().put(sceneEditKey('project-a', 'scene-1'), {
    file: original.file,
    scene: { ...original.file.scene, name: 'Unfinished Script title' },
  })
  mount()
  fireEvent.change(await screen.findByRole('textbox', { name: 'Scene YAML' }), {
    target: { value: 'raw edits' },
  })
  expect(screen.getByRole('button', { name: 'Save source' })).toBeDisabled()
  expect(
    screen.getByText('Save or discard the shared scene draft before saving Source.'),
  ).toBeVisible()
  expect(mocks.save).not.toHaveBeenCalled()
})

it('reads the authoritative v2 source bytes after saving checked v1 YAML', async () => {
  const { qc } = mount()
  fireEvent.change(await screen.findByRole('textbox', { name: 'Scene YAML' }), {
    target: { value: 'schema_version: 1\nscene: changed\n' },
  })
  const actual = {
    ...original,
    yaml: 'schema_version: 2\nscene: canonical\n',
    stamp: { ...original.stamp, hash: 'v2' },
  }
  mocks.get.mockResolvedValue(actual)
  fireEvent.click(screen.getByRole('button', { name: 'Save source' }))
  await waitFor(() =>
    expect(screen.getByRole('textbox', { name: 'Scene YAML' })).toHaveValue(actual.yaml),
  )
  expect(qc.getQueryData(['narrative_source', 'project-a', 'scene-1'])).toEqual(actual)
})
it('does not write another project after asynchronous Source validation', async () => {
  let finish!: (value: unknown) => void
  mocks.check.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve
    }),
  )
  const { qc } = mount()
  fireEvent.change(await screen.findByRole('textbox', { name: 'Scene YAML' }), {
    target: { value: 'retained draft' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Save source' }))
  qc.setQueryData(qk.projectCurrent, { path: 'other' })
  await act(async () =>
    finish({
      scene: original.file.scene,
      formatted: 'parsed draft',
      diagnostics: [],
      problem: null,
    }),
  )
  expect(mocks.save).not.toHaveBeenCalled()
  expect(mocks.repair).not.toHaveBeenCalled()
  expect(screen.getByRole('textbox', { name: 'Scene YAML' })).toHaveValue('retained draft')
})
it('does not install a repair reply into another project or clear its original draft', async () => {
  const malformed = { ...original, file: null, problem: { message: 'Broken source' } }
  mocks.get.mockResolvedValue(malformed)
  let finish!: (value: unknown) => void
  mocks.repair.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve
    }),
  )
  const { qc } = mount()
  fireEvent.change(await screen.findByRole('textbox', { name: 'Scene YAML' }), {
    target: { value: 'repaired draft' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Save source' }))
  await waitFor(() => expect(mocks.repair).toHaveBeenCalled())
  qc.setQueryData(qk.projectCurrent, { path: 'other' })
  const other = { ...original.file, scene: { ...original.file.scene, name: 'Other project' } }
  qc.setQueryData(qk.narrativeScene('scene-1'), other)
  await act(async () => finish({ source: original, recoveryRel: 'original.bak' }))
  expect(qc.getQueryData(qk.narrativeScene('scene-1'))).toBe(other)
  expect(screen.getByRole('textbox', { name: 'Scene YAML' })).toHaveValue('repaired draft')
})

it('retains an old Source draft when a reload finishes after changing projects', async () => {
  const { qc } = mount()
  fireEvent.change(await screen.findByRole('textbox', { name: 'Scene YAML' }), {
    target: { value: 'keep this draft' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Reload' }))
  let finish!: (value: NarrativeSource) => void
  mocks.get.mockReturnValueOnce(
    new Promise((resolve) => {
      finish = resolve
    }),
  )
  fireEvent.click(screen.getByRole('button', { name: 'Discard draft and reload' }))
  qc.setQueryData(qk.projectCurrent, { path: 'other' })
  await act(async () => finish({ ...original, yaml: 'other project source' }))
  expect(screen.getByRole('textbox', { name: 'Scene YAML' })).toHaveValue('keep this draft')
  const calls = mocks.check.mock.calls.length
  fireEvent.click(screen.getByRole('button', { name: 'Validate' }))
  expect(mocks.check).toHaveBeenCalledTimes(calls)
})

it('keeps Source validation tied to its original public project session even after same-path reopen', async () => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  mocks.invoke.mockResolvedValue(null)
  let finish!: (value: unknown) => void
  mocks.check.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve
    }),
  )
  mount()
  fireEvent.change(await screen.findByRole('textbox', { name: 'Scene YAML' }), {
    target: { value: 'retained draft' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Save source' }))
  await projectClose()
  await projectOpen('project-a')
  await act(async () =>
    finish({
      scene: original.file.scene,
      formatted: 'parsed draft',
      diagnostics: [],
      problem: null,
    }),
  )
  expect(mocks.save).not.toHaveBeenCalled()
  expect(mocks.repair).not.toHaveBeenCalled()
  expect(screen.getByRole('textbox', { name: 'Scene YAML' })).toHaveValue('retained draft')
  expect(await screen.findByText(/project session changed/)).toBeVisible()
})
