import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Scene, SceneFile, Text } from '../../lib/api'
import { useUndoStack } from '../../lib/undo'
import { useUI } from '../../store/ui'
import { NarrativeScriptPane } from './NarrativeScriptPane'
import { useScriptDrafts } from './scriptDrafts'
import { useSceneLibrary } from './sceneLibraryStore'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

let saved: SceneFile
function initial(): SceneFile {
  return {
    scene: {
      id: 'scene',
      name: 'Council',
      beats: [
        {
          id: 'beat',
          title: 'Evidence',
          must_not_reveal: ['The hidden fleet'],
          dialogue: [
            {
              id: 'slot',
              speaker: 'player',
              policy: 'edited',
              variants: [
                {
                  id: 'variant',
                  text: {
                    body: 'Old words',
                    revision: 'old-revision',
                    lifecycle: { policy: 'edited', review: 'approved', freshness: 'out_of_date' },
                  },
                },
              ],
            },
          ],
          choices: [
            {
              id: 'choice',
              label: 'Ask',
              requires: { compare: { var: 'trust', op: 'ge', value: { literal: 10 } } },
              effects: [{ add: { var: 'trust', by: 1 } }],
              to: { end: {} },
            },
          ],
        },
      ],
    },
    slug: 'council',
    rel: 'narrative/scenes/council.yaml',
    stamp: { mtime_ms: 1, size: 1, hash: 'old' },
  }
}
function mount(projectKey = 'project', readOnly = false) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  return render(
    <QueryClientProvider client={qc}>
      <NarrativeScriptPane projectKey={projectKey} readOnly={readOnly} />
    </QueryClientProvider>,
  )
}
beforeEach(() => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  saved = initial()
  useUI.getState().selectNarrative({ sceneId: 'scene', beatId: 'beat' }, 'flow')
  useScriptDrafts.setState({ drafts: {} })
  useSceneLibrary.setState({ searchVariant: null })
  useUndoStack.setState({ projectId: 'project', past: [], future: [], busy: false })
  h.invoke.mockReset()
  h.invoke.mockImplementation(async (command: string, args: Record<string, unknown>) => {
    if (command === 'narrative_scene_get') return saved
    if (command === 'narrative_scenes')
      return { scenes: [{ id: 'scene', name: 'Council' }], unreadable: [] }
    if (command === 'node_list') return []
    if (command === 'narrative_text_written')
      return {
        body: args.body as string,
        revision: 'new-revision',
        provenance: 'human',
        lifecycle: {
          policy: args.locked ? 'locked' : 'edited',
          review: 'draft',
          freshness: 'current',
        },
      } satisfies Text
    if (command === 'narrative_scene_save') {
      saved = { ...saved, scene: args.scene as Scene, stamp: { mtime_ms: 2, size: 2, hash: 'new' } }
      return saved
    }
    return []
  })
})

describe('Script authoring', () => {
  it('saves manual text with canonical revisions, retained identity, freshness and undo', async () => {
    mount()
    fireEvent.change(await screen.findByRole('textbox', { name: /Dialogue 1, variant 1/ }), {
      target: { value: 'New words' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save script' }))
    await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(1))
    const variant = saved.scene.beats?.[0]?.dialogue?.[0]?.variants?.[0]
    expect(variant).toMatchObject({
      id: 'variant',
      text: {
        body: 'New words',
        revision: 'new-revision',
        lifecycle: { policy: 'edited', review: 'draft', freshness: 'out_of_date' },
      },
    })
    expect(saved.scene.beats?.[0]?.choices).toEqual(initial().scene.beats?.[0]?.choices)
    expect(saved.scene.beats?.[0]?.must_not_reveal).toEqual(['The hidden fleet'])
    expect(h.invoke).toHaveBeenCalledWith(
      'narrative_scene_save',
      expect.objectContaining({ expected: { kind: 'stamp', stamp: initial().stamp } }),
    )
    expect(useUndoStack.getState().past[0]?.undo).toEqual([
      { type: 'sceneSave', scene: initial().scene, slug: 'council' },
    ])
  })
  it('copies generated text without rewriting provenance and reorders without changing identities', async () => {
    const original = saved.scene.beats![0]!.dialogue![0]!.variants![0]!.text
    original.provenance = { generated: { fingerprint: 'compiler-input' } }
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Duplicate beat' }))
    fireEvent.click(screen.getByRole('button', { name: 'Move up' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save script' }))
    await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(1))
    expect(saved.scene.beats?.[0]?.id).not.toBe('beat')
    expect(saved.scene.beats?.[1]?.id).toBe('beat')
    expect(saved.scene.beats?.[0]?.dialogue?.[0]?.variants?.[0]?.text).toEqual(original)
    expect(h.invoke.mock.calls.some(([command]) => command === 'narrative_text_written')).toBe(
      false,
    )
  })

  it('creates an empty stable slot, then writes and locks its first wording', async () => {
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Add dialogue slot' }))
    expect(screen.getByText(/Missing text/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Write dialogue 2' }))
    fireEvent.change(screen.getByRole('textbox', { name: /Dialogue 2, variant 1/ }), {
      target: { value: 'An authored line.' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Lock dialogue 2' }))
    const slotId = useUI.getState().narrative.lineId
    fireEvent.click(screen.getByRole('button', { name: 'Save script' }))
    await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(1))
    const slot = saved.scene.beats?.[0]?.dialogue?.[1]
    expect(slot?.id).toBe(slotId)
    expect(slot?.policy).toBe('locked')
    expect(slot?.variants?.[0]?.text).toMatchObject({
      body: 'An authored line.',
      lifecycle: { policy: 'locked', review: 'draft' },
    })
  })

  it('requires explicit unlock and keeps read-only projects immutable', async () => {
    saved.scene.beats![0]!.dialogue![0]!.policy = 'locked'
    const view = mount()
    expect(await screen.findByRole('textbox', { name: /Dialogue 1, variant 1/ })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: 'Unlock dialogue 1' }))
    expect(screen.getByRole('textbox', { name: /Dialogue 1, variant 1/ })).toBeEnabled()
    view.unmount()
    mount('project', true)
    expect(await screen.findByRole('textbox', { name: /Dialogue 1, variant 1/ })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Save script' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Discard changes' })).toBeEnabled()
  })
  it('retains unsaved edits across tab remounts and isolates projects', async () => {
    const view = mount()
    fireEvent.change(await screen.findByLabelText('Beat title'), {
      target: { value: 'My unsaved beat' },
    })
    view.unmount()
    const again = mount()
    expect(await screen.findByLabelText('Beat title')).toHaveValue('My unsaved beat')
    again.unmount()
    mount('another-project')
    expect(await screen.findByLabelText('Beat title')).toHaveValue('Evidence')
  })
  it('keeps the draft after a failed guarded save', async () => {
    mount()
    fireEvent.change(await screen.findByLabelText('Beat title'), {
      target: { value: 'Kept draft' },
    })
    const prior = h.invoke.getMockImplementation()!
    h.invoke.mockImplementation((command, args) =>
      command === 'narrative_scene_save' ? Promise.reject('write.conflict') : prior(command, args),
    )
    fireEvent.click(screen.getByRole('button', { name: 'Save script' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Your draft is kept')
    expect(screen.getByLabelText('Beat title')).toHaveValue('Kept draft')
    expect(useUndoStack.getState().past).toHaveLength(0)
  })
  it('does not clear newer typing after a save completes across a tab remount', async () => {
    let release: (() => void) | undefined
    const prior = h.invoke.getMockImplementation()!
    h.invoke.mockImplementation(async (command, args) => {
      if (command === 'narrative_scene_save')
        await new Promise<void>((resolve) => {
          release = resolve
        })
      return prior(command, args)
    })
    const first = mount()
    fireEvent.change(await screen.findByLabelText('Beat title'), {
      target: { value: 'Submitted title' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save script' }))
    await waitFor(() => expect(release).toBeDefined())
    first.unmount()
    mount()
    fireEvent.change(await screen.findByLabelText('Beat title'), {
      target: { value: 'Newer title' },
    })
    await act(async () => release?.())
    await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(1))
    expect(useScriptDrafts.getState().drafts['project:scene']?.scene.beats?.[0]?.title).toBe(
      'Newer title',
    )
    expect(screen.getByLabelText('Beat title')).toHaveValue('Newer title')
  })

  it('reveals the exact searched dialogue variant', async () => {
    mount()
    const field = await screen.findByRole('textbox', { name: /Dialogue 1, variant 1/ })
    act(() =>
      useSceneLibrary.setState({
        searchVariant: { sceneId: 'scene', slotId: 'slot', variantId: 'variant' },
      }),
    )
    await waitFor(() => expect(field).toHaveFocus())
    expect(useUI.getState().narrative.lineId).toBe('slot')
  })
})
