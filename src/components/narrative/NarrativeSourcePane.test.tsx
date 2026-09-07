import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useUI } from '../../store/ui'
import { editorWrites } from '../../lib/editorWrites'
import { resetNarrativeDraftGuards } from '../../lib/narrativeDraftGuard'
import { NarrativeSourcePane } from './NarrativeSourcePane'
import type { NarrativeSource } from '../../lib/api/narrativeSource'

const mocks = vi.hoisted(() => ({ get: vi.fn(), check: vi.fn(), save: vi.fn() }))
vi.mock('../../lib/api/narrativeSource', () => ({
  narrativeSourceGet: mocks.get,
  narrativeSourceCheck: mocks.check,
}))
vi.mock('../../lib/queries', () => ({ useSaveScene: () => ({ mutateAsync: mocks.save }) }))

const original: NarrativeSource = {
  yaml: 'schema_version: 1\nscene: original\n',
  file: {
    scene: { id: 'scene-1', name: 'Council' },
    slug: 'council',
    rel: 'narrative/scenes/council.yaml',
    stamp: { mtime_ms: 1, size: 5, hash: 'original' },
  },
}

function mount(
  qc = new QueryClient({ defaultOptions: { queries: { retry: false } } }),
  readOnly = false,
  projectKey = 'project-a',
) {
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
  })
  mount(qc)
  await waitFor(() => expect(screen.getByRole('textbox')).toHaveValue('latest form source'))
})
