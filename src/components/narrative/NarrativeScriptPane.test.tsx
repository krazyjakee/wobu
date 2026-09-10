import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Scene, SceneFile, Text } from '../../lib/api'
import type { ReviewRequest, ReviewSceneView } from '../../lib/api/narrativeReview'
import { qk } from '../../lib/queries/keys'
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
function reviewed(): ReviewSceneView {
  return {
    scene_id: saved.scene.id,
    guard: { stamp: saved.stamp, head: saved.scene.editorial_head ?? null },
    state_json: '{}',
    history: [],
    context_summary: 'Scene intent and world facts',
    lines: (saved.scene.beats ?? []).flatMap((beat) =>
      (beat.dialogue ?? []).flatMap((slot) =>
        (slot.variants?.length ? slot.variants : [null]).map((variant) => ({
          target: {
            scene: saved.scene.id,
            beat: beat.id,
            slot: slot.id,
            variant: variant?.id ?? null,
          },
          speaker: slot.speaker,
          text: variant?.text ?? null,
          slot_policy: slot.policy ?? 'edited',
          review: 'draft' as const,
          freshness: 'out_of_date' as const,
          approval_valid: false,
          reason: 'No verified review',
          context_revision: 'context',
          proposals: [],
        })),
      ),
    ),
  }
}
function mount(projectKey = 'project', readOnly = false) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  qc.setQueryData(qk.projectCurrent, { path: projectKey })
  return {
    qc,
    ...render(
      <QueryClientProvider client={qc}>
        <NarrativeScriptPane projectKey={projectKey} readOnly={readOnly} />
      </QueryClientProvider>,
    ),
  }
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
    if (command === 'narrative_review_get') return reviewed()
    if (command === 'narrative_review_context')
      return {
        version: 1,
        revision: 'context',
        state: {},
        inputs: { intent: 'Ask about the attack' },
      }
    if (command === 'narrative_review_apply') {
      const request = args.request as ReviewRequest
      saved = structuredClone(saved)
      const slot = saved.scene.beats
        ?.find((b) => b.id === request.target.beat)
        ?.dialogue?.find((s) => s.id === request.target.slot)
      if (slot && request.action.kind === 'policy') {
        if (request.action.scope === 'slot') slot.policy = request.action.policy
        else {
          const variant = slot.variants?.find((v) => v.id === request.target.variant)
          if (variant)
            variant.text.lifecycle = { ...variant.text.lifecycle, policy: request.action.policy }
        }
      }
      saved.stamp = { hash: `${saved.stamp?.hash}-review`, size: 3, mtime_ms: 3 }
      saved.scene.editorial_head = `${saved.stamp.hash}-event`
      return { file: saved, review: reviewed() }
    }
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
  it('keeps a late policy reply out of a newly opened project with the same scene ID', async () => {
    let finish!: (value: unknown) => void
    const original = h.invoke.getMockImplementation()!
    h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
      command === 'narrative_review_apply'
        ? new Promise((resolve) => {
            finish = resolve
          })
        : original(command, args),
    )
    const { qc } = mount()
    fireEvent.change(await screen.findByRole('combobox', { name: 'Wording policy' }), {
      target: { value: 'locked' },
    })
    await waitFor(() => expect(finish).toBeDefined())
    const other = { ...initial(), scene: { id: 'scene', name: 'Different project' } }
    qc.setQueryData(qk.projectCurrent, { path: 'other-project' })
    qc.setQueryData(qk.narrativeScene('scene'), other)
    await act(async () => finish({ file: saved, review: reviewed() }))
    await waitFor(() =>
      expect(screen.queryByText('Applying review decision…')).not.toBeInTheDocument(),
    )
    expect(qc.getQueryData(qk.narrativeScene('scene'))).toEqual(other)
  })

  it('saves manual text with canonical revisions, retained identity, freshness and undo', async () => {
    mount()
    fireEvent.change(await screen.findByRole('textbox', { name: /Dialogue 1, variant 1/ }), {
      target: { value: 'New words' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
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
      { type: 'sceneSave', scene: initial().scene, slug: 'council', expected: saved.scene },
    ])
  })
  it('copies generated text without rewriting provenance and reorders without changing identities', async () => {
    const original = saved.scene.beats![0]!.dialogue![0]!.variants![0]!.text
    original.provenance = { generated: { fingerprint: 'compiler-input' } }
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Duplicate beat' }))
    fireEvent.click(screen.getByRole('button', { name: 'Move up' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
    await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(1))
    expect(saved.scene.beats?.[0]?.id).not.toBe('beat')
    expect(saved.scene.beats?.[1]?.id).toBe('beat')
    expect(saved.scene.beats?.[0]?.dialogue?.[0]?.variants?.[0]?.text).toEqual({
      ...original,
      lifecycle: { ...original.lifecycle, review: 'draft', policy: 'edited' },
    })
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
    const slotId = useUI.getState().narrative.lineId
    fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
    await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(1))
    await waitFor(() => expect(screen.getAllByLabelText('Slot policy')).toHaveLength(2))
    fireEvent.change(screen.getAllByLabelText('Slot policy')[1]!, { target: { value: 'locked' } })
    await waitFor(() => expect(saved.scene.beats?.[0]?.dialogue?.[1]?.policy).toBe('locked'))
    const slot = saved.scene.beats?.[0]?.dialogue?.[1]
    expect(slot?.id).toBe(slotId)
    expect(slot?.policy).toBe('locked')
    expect(slot?.variants?.[0]?.text).toMatchObject({
      body: 'An authored line.',
      lifecycle: { policy: 'edited', review: 'draft' },
    })
  })

  it('requires explicit unlock and keeps read-only projects immutable', async () => {
    saved.scene.beats![0]!.dialogue![0]!.policy = 'locked'
    const view = mount()
    expect(await screen.findByRole('textbox', { name: /Dialogue 1, variant 1/ })).toBeDisabled()
    fireEvent.change(await screen.findByLabelText('Slot policy'), { target: { value: 'edited' } })
    await waitFor(() =>
      expect(screen.getByRole('textbox', { name: /Dialogue 1, variant 1/ })).toBeEnabled(),
    )
    expect(screen.getByRole('textbox', { name: /Dialogue 1, variant 1/ })).toBeEnabled()
    view.unmount()
    mount('project', true)
    expect(await screen.findByRole('textbox', { name: /Dialogue 1, variant 1/ })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Save scene' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Discard changes' })).toBeDisabled()
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
    fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Your draft is kept')
    expect(screen.getByLabelText('Beat title')).toHaveValue('Kept draft')
    expect(useUndoStack.getState().past).toHaveLength(0)
  })
  it('blocks source edits across a tab remount while a shared save is pending', async () => {
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
    fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
    await waitFor(() => expect(release).toBeDefined())
    first.unmount()
    mount()
    const title = await screen.findByLabelText('Beat title')
    expect(title).toBeDisabled()
    fireEvent.change(title, { target: { value: 'Newer title' } })
    expect(useScriptDrafts.getState().drafts['project:scene']?.scene.beats?.[0]?.title).toBe(
      'Submitted title',
    )
    await act(async () => release?.())
    await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(1))
    expect(useScriptDrafts.getState().drafts['project:scene']).toBeUndefined()
    expect(saved.scene.beats?.[0]?.title).toBe('Submitted title')
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

describe('Script variant authoring', () => {
  it('adds conditional variants, preserves existing provenance and tombstones a removed variant', async () => {
    const original = saved.scene.beats![0]!.dialogue![0]!.variants![0]!
    original.text.provenance = { generated: { fingerprint: 'original-input' } }
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Add dialogue 1 variant' }))
    fireEvent.change(screen.getByRole('textbox', { name: /Dialogue 1, variant 2/ }), {
      target: { value: 'An alternative' },
    })
    fireEvent.change(screen.getByLabelText('Dialogue 1 variant 2 rule'), {
      target: { value: 'never' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
    await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(1))
    const variants = saved.scene.beats![0]!.dialogue![0]!.variants!
    expect(variants[0]).toEqual(original)
    expect(variants[1]).toMatchObject({
      when: 'never',
      text: { body: 'An alternative', provenance: 'human' },
    })
    expect(variants[1]!.id).not.toBe(original.id)
    fireEvent.click(screen.getByRole('button', { name: 'Delete dialogue 1 variant 2' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
    await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(2))
    expect(saved.scene.tombstones).toEqual([
      expect.objectContaining({ target: { variant: variants[1]!.id }, label: 'An alternative' }),
    ])
    expect(saved.scene.beats![0]!.dialogue![0]!.variants).toEqual([original])
  })

  it('requires unlocking before deleting a slot or changing a variant condition', async () => {
    saved.scene.beats![0]!.dialogue![0]!.policy = 'locked'
    mount()
    expect(await screen.findByRole('button', { name: 'Delete dialogue 1 slot' })).toBeDisabled()
    expect(screen.getByLabelText('Dialogue 1 variant 1 rule')).toBeDisabled()
    fireEvent.change(await screen.findByLabelText('Slot policy'), { target: { value: 'edited' } })
    await waitFor(() =>
      expect(screen.getByRole('textbox', { name: /Dialogue 1, variant 1/ })).toBeEnabled(),
    )
    fireEvent.click(screen.getByRole('button', { name: 'Delete dialogue 1 slot' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
    await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(1))
    expect(saved.scene.beats![0]!.dialogue).toEqual([])
    expect(saved.scene.tombstones?.map((item) => item.target)).toEqual([
      { dialogue_slot: 'slot' },
      { variant: 'variant' },
    ])
  })
})

it('reorders automatic outcomes and dialogue without changing IDs or branch logic', async () => {
  saved.scene.beats![0]!.outcomes = [
    { id: 'first', when: 'never', to: { end: { label: 'not reached' } } },
    {
      id: 'second',
      when: 'always',
      effects: [{ command: { name: 'award_badge' } }],
      to: { end: {} },
    },
  ]
  const originalOutcomes = structuredClone(saved.scene.beats![0]!.outcomes)
  mount()
  fireEvent.click(await screen.findByRole('button', { name: 'Move outcome 2 up' }))
  fireEvent.click(screen.getByRole('button', { name: 'Add dialogue 1 variant' }))
  fireEvent.change(screen.getByRole('textbox', { name: /Dialogue 1, variant 2/ }), {
    target: { value: 'Second wording' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Move dialogue 1 variant 2 up' }))
  fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
  await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(1))
  expect(saved.scene.beats![0]!.outcomes).toEqual([originalOutcomes[1], originalOutcomes[0]])
  expect(saved.scene.beats![0]!.dialogue![0]!.variants![1]!.id).toBe('variant')
  expect(saved.scene.beats![0]!.dialogue![0]!.variants![0]!.text.body).toBe('Second wording')
})

it('checks the unsaved script and focuses a broken destination by stable ID', async () => {
  const prior = h.invoke.getMockImplementation()!
  h.invoke.mockImplementation((command, args) =>
    command === 'narrative_diagnostics'
      ? Promise.resolve([
          {
            kind: 'choice',
            code: 'dangling_beat',
            message: 'Missing target beat',
            destination: true,
            beatId: 'beat',
            choiceId: 'choice',
          },
        ])
      : prior(command, args),
  )
  mount()
  fireEvent.change(await screen.findByLabelText('Beat title'), {
    target: { value: 'Unsaved title' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Check script' }))
  fireEvent.click(await screen.findByRole('button', { name: 'Missing target beat' }))
  expect(screen.getByLabelText('Choice 1 destination')).toHaveFocus()
  expect(h.invoke).toHaveBeenCalledWith(
    'narrative_diagnostics',
    expect.objectContaining({
      scene: expect.objectContaining({
        beats: [expect.objectContaining({ title: 'Unsaved title' })],
      }),
    }),
  )
  fireEvent.change(screen.getByLabelText('Beat title'), { target: { value: 'Updated title' } })
  expect(screen.queryByRole('button', { name: 'Missing target beat' })).not.toBeInTheDocument()
  expect(screen.getByText('Script changed. Check again for current problems.')).toBeInTheDocument()
})

it('refuses to save a loaded scene whose integer would be rounded by the desktop bridge', async () => {
  saved.scene.entry = { compare: { var: 'trust', op: 'eq', value: { literal: 2 ** 53 } } }
  mount()
  await screen.findByText(/outside the desktop editor’s exact range/)
  fireEvent.change(screen.getByLabelText('Scene name'), { target: { value: 'Renamed council' } })
  expect(screen.getByRole('button', { name: 'Save scene' })).toBeDisabled()
  expect(h.invoke.mock.calls.some(([command]) => command === 'narrative_scene_save')).toBe(false)
})

it('manual editing a Generated slot preserves slot policy and protects only the edited wording', async () => {
  saved.scene.beats![0]!.dialogue![0]!.policy = 'generated'
  saved.scene.beats![0]!.dialogue![0]!.variants![0]!.text.lifecycle = {
    policy: 'generated',
    review: 'draft',
    freshness: 'current',
  }
  mount()
  fireEvent.change(await screen.findByRole('textbox', { name: /Dialogue 1, variant 1/ }), {
    target: { value: 'My corrected wording' },
  })
  expect(screen.getByLabelText('Slot policy')).toBeDisabled()
  fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
  await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(1))
  expect(saved.scene.beats![0]!.dialogue![0]!.policy).toBe('generated')
  expect(saved.scene.beats![0]!.dialogue![0]!.variants![0]!.text.lifecycle?.policy).toBe('edited')
})
