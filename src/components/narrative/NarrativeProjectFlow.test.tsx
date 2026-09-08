import { groupIdentity, draftArcScene } from './flow/arc/projectArcModel'
import type { ArcScene } from '../../lib/api/narrativeArc'
import { projectArcSession } from './flow/arc/projectArcSession'
import { resetNarrativeDraftGuards } from '../../lib/narrativeDraftGuard'
import { qk } from '../../lib/queries/keys'
import { useScriptDrafts, sceneEditKey } from './scriptDrafts'
import { NarrativeScriptPane } from './NarrativeScriptPane'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { NarrativeDiagnostic, Scene, SceneFile, Layout } from '../../lib/api'
import type { Quest } from '../../lib/api/narrativeWorld'
import { useUndoStack } from '../../lib/undo'
import { useUI } from '../../store/ui'
import { NarrativeProjectFlow } from './NarrativeProjectFlow'
import { resetFlowStore, useFlowStore } from './flow/flowStore'
import { mintId, nodeId } from './flow/source'

/* Actual mounted components with mocked IPC. Full-document assertions catch
 * lossy source reconstruction; native evidence is recorded separately. */

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const PROJECT = '/fixture/ashfall.wobu'
const SCENE = mintId()
const OTHER = mintId()
const ARRIVAL = mintId()
const VERDICT = mintId()
const CHOICE = mintId()
const OUTCOME = mintId()
const SLOT = mintId()
const VARIANT = mintId()
const PLAYER_SLOT = mintId()

/** A scene with real dialogue in it, so a lossy save has something to destroy. */
function council(): Scene {
  return {
    id: SCENE,
    name: 'Council hearing',
    summary: 'The council hears the beacon evidence.',
    beats: [
      {
        id: ARRIVAL,
        title: 'Arrival at the hearing',
        intents: [{ subject: 'narrator', intent: 'set the room' }],
        must_not_reveal: ['Mira was there'],
        dialogue: [
          {
            id: SLOT,
            speaker: 'narrator',
            policy: 'locked',
            variants: [
              {
                id: VARIANT,
                text: {
                  revision: 'rev-one',
                  body: 'The hall smells of wet ash.',
                  provenance: { generated: { fingerprint: 'fp' } },
                  lifecycle: { policy: 'locked', review: 'approved', freshness: 'out_of_date' },
                },
              },
            ],
          },
          { id: PLAYER_SLOT, speaker: 'player' },
        ],
        choices: [{ id: CHOICE, label: 'Show the logbook', to: { beat: VERDICT } }],
      },
      {
        id: VERDICT,
        title: 'The verdict',
        outcomes: [{ id: OUTCOME, to: { scene: OTHER } }],
      },
    ],
  }
}

function file(scene: Scene): SceneFile {
  return {
    scene,
    slug: 'council-hearing',
    rel: 'narrative/scenes/council-hearing.yaml',
    stamp: { mtime_ms: 1, size: 2, hash: 'first' },
  }
}

function arcScene(scene: Scene): ArcScene {
  return draftArcScene(scene, {
    summary: { id: scene.id, name: scene.name, slug: 'scene', rel: 'scene.yaml' },
    actId: null,
    arcId: null,
    tagIds: [],
    participants: [],
    slots: 0,
    filled: 0,
    beats: 0,
    counts: { generated: 0, edited: 0, locked: 0, needsReview: 0, outOfDate: 0 },
    arc: { exits: [], needsText: 0 },
  })
}

let diagnostics: NarrativeDiagnostic[] = []
let layoutSave: unknown = { outcome: 'written' }
const layouts = new Map<string, unknown>()
let layoutLoad: unknown
let quests: Quest[] = []

function answer(command: string, args: Record<string, unknown>): unknown {
  switch (command) {
    case 'narrative_arc':
      return {
        revision: 'first',
        scenes: [arcScene(council()), arcScene({ id: OTHER, name: 'The long road', beats: [] })],
        unreadable: [],
      }
    case 'narrative_scenes':
      return {
        scenes: [
          { id: SCENE, name: 'Council hearing', slug: 'council-hearing', rel: 'a.yaml' },
          { id: OTHER, name: 'The long road', slug: 'long-road', rel: 'b.yaml' },
        ],
        unreadable: [],
      }
    case 'narrative_scene_get':
      return args.sceneId === SCENE
        ? file(council())
        : { ...file({ id: OTHER, name: 'The long road', beats: [] }), slug: 'long-road' }
    case 'narrative_scene_save':
      return { ...file(args.scene as Scene), stamp: { mtime_ms: 9, size: 3, hash: 'second' } }
    case 'narrative_diagnostics':
      return diagnostics
    case 'narrative_layout_get': {
      const loaded = layoutLoad as { layout: Layout; notices: unknown[] }
      return (
        layouts.get(JSON.stringify(args.graph)) ??
        (JSON.stringify(loaded.layout.graph) === JSON.stringify(args.graph)
          ? loaded
          : {
              layout: {
                ...loaded.layout,
                graph: args.graph,
                nodes: {},
                groups: {},
                annotations: {},
              },
              notices: [],
            })
      )
    }
    case 'narrative_layout_save':
      if ((layoutSave as { outcome: string }).outcome === 'written')
        layouts.set(JSON.stringify((args.layout as Layout).graph), {
          layout: args.layout,
          notices: [],
        })
      return layoutSave
    case 'narrative_world_get':
      return {
        document: {
          schema_version: 1,
          facts: [],
          knowledge: [],
          relationships: [],
          events: [],
          quests,
          restrictions: [],
        },
        stamp: null,
        diagnostics: [],
      }
    case 'node_list':
      return []
    default:
      return []
  }
}

function calls(command: string): Record<string, unknown>[] {
  return (h.invoke.mock.calls as [string, Record<string, unknown>][])
    .filter(([name]) => name === command)
    .map(([, args]) => args)
}

/** A World quest, with only the fields the arc groups by made interesting. */
function worldQuest(id: string, name: string, initial: string, scenes: string[]): Quest {
  return { id, name, summary: '', stages: [initial], initial, transitions: [], scene_ids: scenes }
}

/** A layout runner that answers at once. The real one starts a Web Worker. */
const layout = () => Promise.resolve({ positions: {} })

function open(readOnly = false, active = true) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  qc.setQueryData(qk.projectCurrent, { path: PROJECT })
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  )
  const view = render(
    <Wrapper>
      <NarrativeProjectFlow
        readOnly={readOnly}
        active={active}
        projectKey={PROJECT}
        layout={layout}
      />
    </Wrapper>,
  )
  return {
    qc,
    setActive: (next: boolean) =>
      view.rerender(
        <Wrapper>
          <NarrativeProjectFlow
            readOnly={readOnly}
            active={next}
            projectKey={PROJECT}
            layout={layout}
          />
        </Wrapper>,
      ),
    showScript: () =>
      view.rerender(
        <Wrapper>
          <NarrativeScriptPane projectKey={PROJECT} readOnly={readOnly} />
        </Wrapper>,
      ),
  }
}

/** Open the scene the way the Library does: by writing the shared selection. */
async function enterCouncil(readOnly = false) {
  open(readOnly)
  useUI.getState().selectNarrative({ sceneId: SCENE }, 'library')
  await screen.findByRole('button', { name: 'Outline list' })
  fireEvent.click(screen.getByRole('button', { name: 'Outline list' }))
  await screen.findByLabelText('Council hearing outline')
}

beforeEach(() => {
  const arcSession = projectArcSession(PROJECT)
  layouts.clear()
  arcSession.scope = ''
  arcSession.views.clear()
  arcSession.stores.clear()
  arcSession.scroll = 0
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  h.invoke.mockReset()
  h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
    Promise.resolve(answer(command, args)),
  )
  diagnostics = []
  quests = []
  layoutSave = { outcome: 'written' }
  layoutLoad = {
    layout: {
      schemaVersion: 1,
      graph: { kind: 'scene', scene: SCENE },
      mode: 'manual',
      modeUpdatedAt: 'then',
      nodes: {},
      groups: {},
      annotations: {},
    },
    notices: [],
  }
  resetFlowStore()
  resetNarrativeDraftGuards()
  useScriptDrafts.setState({ drafts: {} })
  useUndoStack.setState({ projectId: 'proj', past: [], future: [], busy: false })
  useUI.setState({
    narrative: { sceneId: null, beatId: null, lineId: null },
    narrativeReveal: null,
    narrativeFilters: { needsText: false, needsReview: false, outOfDate: false },
  })
})

describe('a structural edit cannot lose a line of dialogue', () => {
  it('saves a patch of the loaded document, with every revision untouched', async () => {
    await enterCouncil()

    // Re-point the choice from the verdict back at the arrival beat.
    fireEvent.change(screen.getByLabelText('Show the logbook — Then leads to'), {
      target: { value: nodeId.beat(ARRIVAL) },
    })

    expect(calls('narrative_scene_save')).toHaveLength(0)
    fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
    await waitFor(() => expect(calls('narrative_scene_save')).toHaveLength(1))
    const sent = calls('narrative_scene_save')[0]!.scene as Scene
    const before = council()

    // The intended change, and only it — asserted by rebuilding the expected
    // document from the original. A container that reconstructed a scene from
    // the canvas would fail here with the dialogue missing.
    const expected = structuredClone(before)
    expected.beats![0]!.choices![0]!.to = { beat: ARRIVAL }
    expect(sent).toEqual(expected)

    // Said again on the fields a lossy save destroys silently.
    const variant = sent.beats![0]!.dialogue![0]!.variants![0]!
    expect(variant.text.body).toBe('The hall smells of wet ash.')
    expect(variant.text.revision).toBe('rev-one')
    expect(variant.text.lifecycle).toEqual({
      policy: 'locked',
      review: 'approved',
      freshness: 'out_of_date',
    })
    expect(sent.beats![0]!.must_not_reveal).toEqual(['Mira was there'])
  })

  it('sends the precondition off the file it loaded, and records one undo entry', async () => {
    // Both come from `useSaveScene` rather than from anything in the canvas —
    // which is the point of routing a canvas edit through the one write hook.
    await enterCouncil()
    fireEvent.change(screen.getByLabelText('Show the logbook — Then leads to'), {
      target: { value: nodeId.beat(ARRIVAL) },
    })
    expect(calls('narrative_scene_save')).toHaveLength(0)
    fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
    await waitFor(() => expect(calls('narrative_scene_save')).toHaveLength(1))
    expect(calls('narrative_scene_save')[0]!.expected).toEqual({
      kind: 'stamp',
      stamp: { mtime_ms: 1, size: 2, hash: 'first' },
    })
    await waitFor(() => expect(useUndoStack.getState().past).toHaveLength(1))
  })

  it('disconnects into an explicit unresolved draft without deleting route content', async () => {
    await enterCouncil()
    const picker = screen.getByLabelText('Show the logbook — Then leads to')
    expect(within(picker).getByText('Nothing yet')).toBeInTheDocument()
    fireEvent.change(picker, { target: { value: '' } })
    expect(calls('narrative_scene_save')).toHaveLength(0)
    const draft = useScriptDrafts.getState().drafts[sceneEditKey(PROJECT, SCENE)]!.scene
    expect(draft.beats![0]!.choices![0]).toEqual({
      ...council().beats![0]!.choices![0],
      to: { unresolved: {} },
    })
    expect(screen.getByText('Unsaved scene — shared by Script and Flow.')).toBeInTheDocument()
  })

  it('refuses to rewire a beat to its own choices, and says why', async () => {
    // A beat leads to its choices and outcomes because they belong to it. There
    // is no field behind that wire, so it is read rather than edited.
    await enterCouncil()
    expect(
      screen.getByLabelText(`Arrival at the hearing — Show the logbook leads to`),
    ).toBeDisabled()
  })

  it('disconnects a derived scene link while retaining the owning outcome', async () => {
    await enterCouncil()
    const link = screen
      .getByRole('button', { name: 'Scene linkThe long road' })
      .closest('.nrt-outline-row') as HTMLElement
    fireEvent.click(within(link).getByRole('button', { name: 'Delete' }))
    expect(calls('narrative_scene_save')).toHaveLength(0)
    const draft = useScriptDrafts.getState().drafts[sceneEditKey(PROJECT, SCENE)]!.scene
    expect(draft.beats![1]!.outcomes).toEqual([{ id: OUTCOME, to: { unresolved: {} } }])
    expect(screen.queryByRole('button', { name: 'Scene linkThe long road' })).toBeNull()
  })

  it('adds a beat with a real identity and no invented content', async () => {
    await enterCouncil()
    fireEvent.click(screen.getByRole('button', { name: /Arrival at the hearing/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Add beat after' }))

    expect(calls('narrative_scene_save')).toHaveLength(0)
    fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
    await waitFor(() => expect(calls('narrative_scene_save')).toHaveLength(1))
    const sent = calls('narrative_scene_save')[0]!.scene as Scene
    expect(sent.beats).toHaveLength(3)
    expect(sent.beats![1]!.id).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/)
    expect(sent.beats![1]!.dialogue).toBeUndefined()
    // The first two beats are byte-identical to what was loaded.
    expect([sent.beats![0], sent.beats![2]]).toEqual(council().beats)
  })
})

it('uses identical canonical operations from actual Flow and Script controls', async () => {
  const view = open()
  useUI.getState().selectNarrative({ sceneId: SCENE, beatId: ARRIVAL }, 'library')
  fireEvent.click(await screen.findByRole('button', { name: 'Outline list' }))
  fireEvent.change(await screen.findByLabelText('Show the logbook — Then leads to'), {
    target: { value: nodeId.beat(ARRIVAL) },
  })
  const flow = structuredClone(
    useScriptDrafts.getState().drafts[sceneEditKey(PROJECT, SCENE)]!.scene,
  )
  expect(calls('narrative_scene_save')).toHaveLength(0)
  fireEvent.click(screen.getByRole('button', { name: 'Discard changes' }))
  view.showScript()
  fireEvent.change(await screen.findByLabelText('Choice 1 destination'), {
    target: { value: `beat:${ARRIVAL}` },
  })
  const script = useScriptDrafts.getState().drafts[sceneEditKey(PROJECT, SCENE)]!.scene
  expect(script).toEqual(flow)
  expect(calls('narrative_scene_save')).toHaveLength(0)
  fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
  await waitFor(() => expect(calls('narrative_scene_save')).toHaveLength(1))
  expect(calls('narrative_scene_save')[0]!.scene).toEqual(flow)
})

// Independent authoring gestures correctly mint independent identities/times.
// Normalize only those newly authored identities and tombstone timestamps when
// comparing complete documents; existing source identities must match exactly.
function comparableEdit(scene: Scene, original: Scene) {
  const existing = new Set<string>()
  JSON.stringify(original, (key, value) => {
    if (key === 'id') existing.add(value)
    return value
  })
  const fresh = new Map<string, string>()
  JSON.stringify(scene, (key, value) => {
    if (key === 'id' && !existing.has(value)) fresh.set(value, `new-identity-${fresh.size}`)
    return value
  })
  return JSON.parse(
    JSON.stringify(scene, (key, value) =>
      key === 'deleted_at'
        ? 'deletion time'
        : typeof value === 'string'
          ? (fresh.get(value) ?? value)
          : value,
    ),
  ) as Scene
}

it.each(['Add beat', 'Duplicate beat', 'Delete beat'])(
  'produces equivalent complete source and shared undo for %s in Flow and Script',
  async (action) => {
    const target = action === 'Delete beat' ? VERDICT : ARRIVAL
    useUI.getState().selectNarrative({ sceneId: SCENE, beatId: target }, 'library')
    const view = open()
    await screen.findByTestId(`flow-node-${nodeId.beat(target)}`)
    fireEvent.click(screen.getByRole('button', { name: action }))
    const flow = structuredClone(workingScene())
    expect(flow.beats![0]).toEqual(council().beats![0])
    if (action === 'Add beat') {
      expect(flow.beats!.map((beat) => beat.title)).toEqual([
        'Arrival at the hearing',
        'New beat',
        'The verdict',
      ])
      expect(flow.beats![1]!.id).not.toBe(ARRIVAL)
    } else if (action === 'Duplicate beat') {
      const copy = flow.beats![1]!
      expect(copy.title).toBe('Arrival at the hearing (copy)')
      expect(copy.id).not.toBe(ARRIVAL)
      expect(copy.dialogue![0]!.id).not.toBe(SLOT)
      expect(copy.dialogue![0]!.variants![0]!.id).not.toBe(VARIANT)
      expect(copy.dialogue![0]!.variants![0]!.text).toEqual({
        ...council().beats![0]!.dialogue![0]!.variants![0]!.text,
        lifecycle: { policy: 'locked', review: 'draft', freshness: 'out_of_date' },
      })
      expect(screen.getByRole('button', { name: 'Delete beat' })).toBeDisabled()
    } else {
      expect(flow.beats).toHaveLength(1)
      expect(flow.beats![0]!.choices![0]!.to).toEqual({ beat: VERDICT })
      expect(flow.tombstones).toEqual([expect.objectContaining({ target: { beat: VERDICT } })])
    }
    fireEvent.click(screen.getByRole('button', { name: 'Undo draft' }))
    expect(workingScene()).toEqual(council())
    fireEvent.click(screen.getByRole('button', { name: 'Redo draft' }))
    expect(workingScene()).toEqual(flow)
    fireEvent.click(screen.getByRole('button', { name: 'Discard changes' }))
    useUI.getState().selectNarrative({ sceneId: SCENE, beatId: target }, 'script')
    view.showScript()
    fireEvent.click(await screen.findByRole('button', { name: action }))
    expect(comparableEdit(workingScene(), council())).toEqual(comparableEdit(flow, council()))
    if (action === 'Duplicate beat')
      expect(screen.getByRole('button', { name: 'Delete beat' })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: 'Undo draft' }))
    expect(workingScene()).toEqual(council())
    expect(calls('narrative_scene_save')).toHaveLength(0)
  },
)

it('reconverges three routes identically from Flow and Script while preserving locked dialogue', async () => {
  const original = council()
  original.beats![0]!.choices = ['Show logbook', 'Appeal to duty', 'Threaten council'].map(
    (label) => ({ id: mintId(), label, to: { unresolved: {} } }),
  )
  h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
    Promise.resolve(
      command === 'narrative_scene_get' && args.sceneId === SCENE
        ? file(original)
        : answer(command, args),
    ),
  )
  useUI.getState().selectNarrative({ sceneId: SCENE, beatId: ARRIVAL }, 'library')
  const view = open()
  for (const route of original.beats![0]!.choices!) {
    fireEvent.click(await screen.findByTestId(`flow-node-${nodeId.choice(route.id)}`))
    fireEvent.change(screen.getByLabelText('Route destination'), {
      target: { value: `beat:${VERDICT}` },
    })
  }
  const flow = structuredClone(workingScene())
  expect(flow.beats![0]!.choices!.map((route) => route.to)).toEqual(
    Array(3).fill({ beat: VERDICT }),
  )
  expect(flow.beats![0]!.dialogue).toEqual(original.beats![0]!.dialogue)
  fireEvent.click(screen.getByRole('button', { name: 'Discard changes' }))
  view.showScript()
  for (let index = 1; index <= 3; index++)
    fireEvent.change(await screen.findByLabelText(`Choice ${index} destination`), {
      target: { value: `beat:${VERDICT}` },
    })
  expect(workingScene()).toEqual(flow)
  expect(screen.getByRole('button', { name: 'Delete beat' })).toBeDisabled()
  for (let index = 0; index < 3; index++)
    fireEvent.click(screen.getByRole('button', { name: 'Undo draft' }))
  expect(workingScene()).toEqual(original)
  expect(calls('narrative_scene_save')).toHaveLength(0)
})

describe('the canvas draws what is in the file', () => {
  it('shows a beat’s counts, never its lines', async () => {
    await enterCouncil()
    const rows = screen.getByLabelText('Council hearing outline')
    expect(rows).toHaveTextContent('2 lines · 1 variants')
    expect(rows).not.toHaveTextContent('wet ash')
  })

  it('never says “Demonstration data” over a writer’s own scene', async () => {
    // The banner claiming a fixture while a real scene is on screen would be
    // worse than no banner at all.
    await enterCouncil()
    expect(screen.queryByText(/Demonstration data/)).toBeNull()
  })

  it('says the workspace owns undo, rather than offering a second one', async () => {
    await enterCouncil()
    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull()
    expect(screen.getByText(/Undo and redo are the workspace’s own/)).toBeInTheDocument()
  })
})

describe('diagnostics, attached by id', () => {
  it('lands a destination problem on the choice responsible for it', async () => {
    diagnostics = [
      {
        kind: 'choice',
        code: 'dangling_beat',
        message: 'this destination names beat 01J, which is not in this scene',
        destination: true,
        beatId: ARRIVAL,
        choiceId: CHOICE,
      },
    ]
    await enterCouncil()
    const rows = screen.getByLabelText('Council hearing outline')
    const problems = await within(rows).findByLabelText('Problems with Show the logbook')
    expect(problems).toHaveTextContent('Error')
    expect(problems).toHaveTextContent('which is not in this scene')
  })

  it('rolls a slot’s missing text up onto the beat that holds it', async () => {
    diagnostics = [
      {
        kind: 'dialogueSlot',
        code: 'missing_text',
        message: 'this slot has no text yet',
        destination: false,
        beatId: ARRIVAL,
        slotId: SLOT,
      },
    ]
    await enterCouncil()
    const problems = await screen.findByLabelText('Problems with Arrival at the hearing')
    expect(problems).toHaveTextContent('Warning')
  })

  it('reports a problem with the scene itself, which has no box to sit on', async () => {
    diagnostics = [
      { kind: 'scene', code: 'no_beats', message: 'this scene has no beats', destination: false },
    ]
    await enterCouncil()
    expect(await screen.findByLabelText('Problems with this scene')).toHaveTextContent(
      'this scene has no beats',
    )
  })
})

describe('the badge filters, and the one thing they may not do', () => {
  it('hides a warning and keeps the error, and writes nothing either way', async () => {
    /*
     * #189's guarantee, at the surface a person actually touches.
     *
     * Turning warnings off is a real filter. Turning errors off is not offered,
     * and there is no state in which one is hidden — `badgeShown` answers before
     * it reads the filter at all. A canvas that looked clean while a route led
     * nowhere is the failure this exists to prevent.
     */
    diagnostics = [
      {
        kind: 'choice',
        code: 'dangling_beat',
        message: 'this destination names a beat that is not in this scene',
        destination: true,
        beatId: ARRIVAL,
        choiceId: CHOICE,
      },
      {
        kind: 'dialogueSlot',
        code: 'missing_text',
        message: 'this slot has no text yet',
        destination: false,
        beatId: ARRIVAL,
        slotId: SLOT,
      },
    ]
    open()
    useUI.getState().selectNarrative({ sceneId: SCENE }, 'library')
    await screen.findByRole('button', { name: /Auto layout/ })
    await screen.findByText('1 destination error')
    expect(screen.getByText('1 text warning')).toBeInTheDocument()

    fireEvent.click(screen.getByText('Badges'))
    fireEvent.click(screen.getByRole('button', { name: /^Warnings/ }))

    expect(screen.queryByText('1 text warning')).toBeNull()
    // Still there, with every chip off.
    expect(screen.getByText('1 destination error')).toBeInTheDocument()
    // The Errors chip is a marker that explains itself, not a switch.
    expect(screen.getByRole('button', { name: /^Errors/ })).toHaveAttribute('aria-disabled', 'true')

    // And nothing was written by any of it.
    expect(calls('narrative_scene_save')).toHaveLength(0)
    expect(calls('narrative_layout_save')).toHaveLength(0)
    expect(useUndoStack.getState().past).toHaveLength(0)
  })

  it('counts the three lifecycle dimensions separately on the beat that holds them', async () => {
    // The locked, approved, out-of-date line in the fixture is *all three* at
    // once. A single status would have to pick one and hide the other two.
    open()
    useUI.getState().selectNarrative({ sceneId: SCENE }, 'library')
    await screen.findByRole('button', { name: /Auto layout/ })
    const beat = await screen.findByTestId(`flow-node-${nodeId.beat(ARRIVAL)}`)
    expect(beat).toHaveTextContent('1 needs text')
    expect(beat).toHaveTextContent('1 out of date')
    expect(beat).toHaveTextContent('1 locked')
    // Approved, so it is not waiting for review — the dimensions are read
    // independently rather than derived from one another.
    expect(beat).not.toHaveTextContent('needs review')
  })
})

describe('the arrangement', () => {
  it('is saved to its own file, with no key the layout format cannot name', async () => {
    await enterCouncil()
    fireEvent.click(screen.getByRole('button', { name: 'Canvas' }))
    fireEvent.click(await screen.findByRole('button', { name: /Auto layout/ }))

    await waitFor(() => expect(calls('narrative_layout_save')).toHaveLength(1))
    const sent = calls('narrative_layout_save')[0]!.layout as { nodes: Record<string, unknown> }
    for (const key of Object.keys(sent.nodes)) {
      expect(key).toMatch(/^(beat|choice|outcome):[0-9A-HJKMNP-TV-Z]{26}$/)
    }
    // And nothing about the story was written.
    expect(calls('narrative_scene_save')).toHaveLength(0)
    expect(useUndoStack.getState().past).toHaveLength(0)
  })

  it('treats a refused arrangement as information, never as an error', async () => {
    // #185's invariant at the UI: a read-only share must not put an alert on
    // screen for a drag, and it must not stop a source save.
    layoutSave = { outcome: 'unwritable', reason: 'the folder is read-only' }
    await enterCouncil()
    fireEvent.click(screen.getByRole('button', { name: 'Canvas' }))
    fireEvent.click(await screen.findByRole('button', { name: /Auto layout/ }))

    const said = await screen.findByText(/only in this session/)
    expect(said).toHaveAttribute('role', 'status')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('says when a sidecar came from a newer Wobu, without treating it as a failure', async () => {
    layoutLoad = {
      layout: {
        schemaVersion: 1,
        graph: { kind: 'scene', scene: SCENE },
        mode: 'automatic',
        modeUpdatedAt: 'then',
        nodes: {},
        groups: {},
        annotations: {},
      },
      notices: [
        {
          kind: 'newerSchema',
          blocking: false,
          rel: 'narrative/layout/a.yaml',
          found: 9,
          supported: 1,
        },
      ],
    }
    await enterCouncil()
    expect(await screen.findByText(/written by a newer Wobu/)).toHaveAttribute('role', 'status')
  })
})

describe('what is honestly out of reach', () => {
  it('says a badge cannot open a witness scenario, and why', async () => {
    await enterCouncil()
    expect(
      screen.getByText(/Opening a witness needs generated reachability scenarios/),
      // The Flow overlay half of this landed in #188, so the sentence now
      // claims only what is still missing — and says so without implying that
      // an unplayed scene has been shown unreachable.
    ).toHaveTextContent(/do not establish reachability either way/)
  })

  it('says an affected-build scope cannot be highlighted, and why', async () => {
    await enterCouncil()
    const note = screen.getByText(/Opening a witness needs generated reachability scenarios/)
    // #168 landed the tracker, so the sentence no longer claims that part is
    // missing — it points at where the answer actually is and names the one
    // thing this canvas still cannot do.
    expect(note).toHaveTextContent(/Context → Why affected/)
    expect(note).toHaveTextContent(/need the build planner \(#169\)/)
  })
})

describe('going in and coming back out', () => {
  it('keeps the scene selected after leaving, because leaving is not deselecting', async () => {
    // Escape goes back to the arc. Script and the inspector are still reading
    // the same scene, so the selection has to survive the trip — a level is not
    // a selection.
    await enterCouncil()
    fireEvent.click(screen.getByRole('button', { name: /Every scene/ }))
    await screen.findByRole('navigation', { name: 'Flow level' })
    expect(useUI.getState().narrative.sceneId).toBe(SCENE)
    expect(screen.queryByLabelText('Council hearing outline')).toBeNull()
  })

  it('goes back in when the Library asks for the same scene again', async () => {
    // The second click on a row that is already selected changes no value —
    // only the reveal's sequence number rises — so following the reveal rather
    // than the selection is what makes it work at all.
    await enterCouncil()
    fireEvent.click(screen.getByRole('button', { name: /Every scene/ }))
    await screen.findByRole('navigation', { name: 'Flow level' })

    useUI.getState().selectNarrative({ sceneId: SCENE }, 'library')
    // The crumb back up to the arc is a button only when a scene is open: the
    // arc draws itself as the current level, not as a way to reach one.
    expect(await screen.findByRole('button', { name: /Every scene/ })).toBeInTheDocument()
  })
})

describe('a read-only project', () => {
  it('draws the scene and refuses to write it', async () => {
    await enterCouncil(true)
    fireEvent.change(screen.getByLabelText('Show the logbook — Then leads to'), {
      target: { value: nodeId.beat(ARRIVAL) },
    })
    expect(calls('narrative_scene_save')).toHaveLength(0)
    expect(useFlowStore.getState().announcement.text).toMatch(/read-only/)
  })
})

describe('shared presentation controls', () => {
  it('hydrates stored collapsed groups on the first editor mount', async () => {
    const id = mintId()
    ;(layoutLoad as { layout: Layout }).layout.groups[id] = {
      id,
      label: 'Saved evidence group',
      members: [nodeId.beat(ARRIVAL)],
      collapsed: true,
      updatedAt: '2026-09-08T00:00:00Z',
    }
    open()
    useUI.getState().selectNarrative({ sceneId: SCENE }, 'library')
    await screen.findByTestId(`flow-node-${id}`)
    expect(screen.queryByTestId(`flow-node-${nodeId.beat(ARRIVAL)}`)).toBeNull()
    expect(calls('narrative_layout_save')).toHaveLength(0)
  })

  it.each(['canvas', 'outline'])(
    'authors a group and note in %s without changing dialogue, source or undo',
    async (mode) => {
      await enterCouncil()
      if (mode === 'canvas') {
        fireEvent.click(screen.getByRole('button', { name: 'Canvas' }))
        fireEvent.click(await screen.findByTestId(`flow-node-${nodeId.beat(ARRIVAL)}`))
      } else fireEvent.click(screen.getByRole('button', { name: /BeatArrival at the hearing/ }))
      fireEvent.click(screen.getByRole('button', { name: 'Groups & notes' }))
      const previousY = (screen.getByLabelText('Node Y') as HTMLInputElement).valueAsNumber
      fireEvent.change(screen.getByLabelText('Node X'), { target: { value: '999' } })
      await waitFor(() =>
        expect(
          (calls('narrative_layout_save').at(-1)!.layout as Layout).nodes[`beat:${ARRIVAL}`],
        ).toMatchObject({ x: 999, y: previousY }),
      )
      fireEvent.change(screen.getByLabelText('Group name'), {
        target: { value: 'Evidence branch' },
      })
      fireEvent.click(screen.getByRole('button', { name: 'Create group' }))
      const close = await screen.findByRole('button', { name: 'Close Evidence branch' })
      fireEvent.click(close)
      await screen.findByRole('button', { name: 'Open Evidence branch' })
      fireEvent.click(screen.getByRole('button', { name: 'Add pinned note' }))
      const text = await screen.findByRole('textbox', { name: 'Pinned note text' })
      fireEvent.change(text, { target: { value: 'Keep the exit visible for review.' } })
      fireEvent.blur(text)
      await waitFor(() =>
        expect((calls('narrative_layout_save').at(-1)!.layout as Layout).annotations).toEqual(
          expect.objectContaining({
            [Object.keys(
              (calls('narrative_layout_save').at(-1)!.layout as Layout).annotations,
            )[0]!]: expect.objectContaining({ body: 'Keep the exit visible for review.' }),
          }),
        ),
      )
      fireEvent.click(screen.getByRole('button', { name: 'Delete pinned note' }))
      await waitFor(() =>
        expect(
          Object.keys(
            (calls('narrative_layout_save').at(-1)!.layout as Layout).removedAnnotations ?? {},
          ),
        ).toHaveLength(1),
      )
      const saved = calls('narrative_layout_save').at(-1)!.layout as Layout
      expect(Object.values(saved.groups)[0]).toMatchObject({
        members: [nodeId.beat(ARRIVAL)],
        collapsed: true,
      })
      expect(saved.annotations).toEqual({})
      expect(calls('narrative_scene_save')).toHaveLength(0)
      expect(useUndoStack.getState().past).toHaveLength(0)
    },
  )

  it.each(['canvas', 'outline'])(
    'retries a refused arrangement in %s without a source edit',
    async (mode) => {
      layoutSave = { outcome: 'unwritable', reason: 'temporary folder outage' }
      await enterCouncil()
      if (mode === 'canvas') fireEvent.click(screen.getByRole('button', { name: 'Canvas' }))
      fireEvent.change(await screen.findByLabelText('Arrangement mode'), {
        target: { value: 'automatic' },
      })
      const retry = await screen.findByRole('button', { name: 'Retry arrangement save' })
      expect(screen.getByLabelText('Arrangement mode')).toHaveValue('automatic')
      layoutSave = { outcome: 'written' }
      fireEvent.click(retry)
      await waitFor(() =>
        expect(screen.queryByRole('button', { name: 'Retry arrangement save' })).toBeNull(),
      )
      expect(calls('narrative_layout_save')).toHaveLength(2)
      expect(calls('narrative_scene_save')).toHaveLength(0)
    },
  )

  it('opens the actual World quest under its stable identity', async () => {
    const id = mintId()
    quests = [
      {
        id,
        name: 'Ashfall inquiry',
        summary: 'Find the witness',
        stages: ['open'],
        initial: 'open',
        transitions: [],
        scene_ids: [SCENE],
      },
    ]
    open()
    await screen.findByRole('option', { name: 'Ashfall inquiry' })
    fireEvent.change(screen.getByLabelText('Flow scope'), { target: { value: id } })
    await waitFor(() =>
      expect(calls('narrative_layout_get')).toContainEqual({
        graph: { kind: 'arc', arc: `${id}-group-quest` },
      }),
    )
    await screen.findByTestId(`flow-node-${SCENE}`)
    expect(screen.getByTestId(`flow-node-${OTHER}`)).toBeInTheDocument()
    expect(calls('narrative_scene_save')).toHaveLength(0)
    expect(calls('narrative_world_save')).toHaveLength(0)
  })

  it('groups the arc by the quests World records, and writes nothing to do it', async () => {
    const beacon = mintId()
    const aftermath = mintId()
    quests = [worldQuest(beacon, 'Ashfall inquiry', 'open', [SCENE])]
    quests.push(worldQuest(aftermath, 'Aftermath', 'closed', [OTHER]))
    open()
    await screen.findByTestId(`flow-node-${SCENE}`)

    fireEvent.change(await screen.findByLabelText('Group'), { target: { value: 'quest' } })
    fireEvent.click(screen.getByRole('button', { name: 'Close all groups' }))

    // Membership is the project's: each scene is inside the quest whose
    // scene_ids name it, and nothing was derived from either scene's name.
    expect(
      await screen.findByTestId(`flow-node-${groupIdentity(`quest:["${beacon}"]`)}`),
    ).toHaveTextContent('2 elements')
    expect(projectArcSession(PROJECT).stores.get(':quest')!.getState().closedGroups).toContain(
      groupIdentity(`quest:["${aftermath}"]`),
    )
    expect(screen.queryByTestId(`flow-node-${SCENE}`)).toBeNull()
    // Collapse persists presentation only; canonical source remains unchanged.
    expect(calls('narrative_scene_save')).toHaveLength(0)
    expect(calls('narrative_world_save')).toHaveLength(0)
    await waitFor(() => expect(calls('narrative_layout_save').length).toBeGreaterThan(0))
  })

  it('keeps a large quest group open after Rust omits its false collapse flag', async () => {
    const quest = mintId()
    const scenes = [
      council(),
      ...Array.from({ length: 300 }, (_, i) => ({
        id: mintId(),
        name: `Scene ${i}`,
        beats: [{ id: mintId(), title: 'Entry' }],
      })),
    ]
    quests = [
      worldQuest(
        quest,
        'Large inquiry',
        'open',
        scenes.map((scene) => scene.id),
      ),
    ]
    h.invoke.mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === 'narrative_arc')
        return { revision: 'large', scenes: scenes.map(arcScene), unreadable: [] }
      const result = answer(command, args)
      if (command === 'narrative_layout_save') {
        const saved = layouts.get(JSON.stringify((args.layout as Layout).graph)) as {
          layout: Layout
        }
        saved.layout = structuredClone(saved.layout)
        for (const group of Object.values(saved.layout.groups)) {
          if (!group.collapsed) delete group.collapsed
        }
      }
      return result
    })
    open()
    await screen.findByTestId(`flow-node-${groupIdentity(`quest:["${quest}"]`)}`)
    fireEvent.click(screen.getByRole('button', { name: 'Open all groups' }))
    await waitFor(() => expect(calls('narrative_layout_save').length).toBeGreaterThan(0))
    await screen.findByRole('button', { name: 'Close all groups' })
    expect(projectArcSession(PROJECT).stores.get(':quest')!.getState().closedGroups).toEqual([])
    expect(calls('narrative_scene_save')).toHaveLength(0)
  })

  it('groups by the stage each quest starts in, folding two quests into one box', async () => {
    quests = [
      worldQuest(mintId(), 'Ashfall inquiry', 'open', [SCENE]),
      worldQuest(mintId(), 'Aftermath', 'open', [OTHER]),
    ]
    open()
    await screen.findByTestId(`flow-node-${SCENE}`)

    fireEvent.change(await screen.findByLabelText('Group'), { target: { value: 'questState' } })
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Close all groups' })).toBeEnabled(),
    )
    fireEvent.click(screen.getByRole('button', { name: 'Close all groups' }))

    expect(
      await screen.findByTestId(`flow-node-${groupIdentity('questState:["open"]')}`),
    ).toHaveTextContent('4 elements')
    await waitFor(() => expect(calls('narrative_layout_save').length).toBeGreaterThan(0))
  })

  it('leaves the saved arrangement folded while the arc is read by quest', async () => {
    const group = mintId()
    quests = [worldQuest(mintId(), 'Ashfall inquiry', 'open', [SCENE, OTHER])]
    layoutLoad = {
      layout: {
        schemaVersion: 2,
        graph: { kind: 'arc', arc: 'project' },
        mode: 'manual',
        modeUpdatedAt: 'then',
        nodes: {},
        groups: {
          [group]: {
            id: group,
            label: 'Evidence',
            members: [`scene:${SCENE}`],
            collapsed: true,
            updatedAt: 'then',
          },
        },
        annotations: {},
      },
      notices: [],
    }
    open()
    fireEvent.change(await screen.findByLabelText('Group'), { target: { value: 'arrangement' } })
    // The stored arrangement retains its own collapsed groups.
    await screen.findByTestId(`flow-node-${group}`)
    expect(screen.queryByTestId(`flow-node-${SCENE}`)).toBeNull()

    fireEvent.change(await screen.findByLabelText('Group'), { target: { value: 'quest' } })
    await screen.findByTestId(`flow-node-${SCENE}`)
    expect(screen.queryByTestId(`flow-node-${group}`)).toBeNull()

    fireEvent.change(screen.getByLabelText('Group'), { target: { value: 'arrangement' } })
    // Back to the saved arrangement, still folded: looking at the quests did
    // not quietly reopen every group somebody had closed, and it did not
    // rewrite the sidecar to say so.
    await screen.findByTestId(`flow-node-${group}`)
    expect(calls('narrative_layout_save')).toHaveLength(0)
    expect(calls('narrative_scene_save')).toHaveLength(0)
  })
})

function workingScene() {
  return useScriptDrafts.getState().drafts[sceneEditKey(PROJECT, SCENE)]!.scene
}
it('creates choices, guarded outcomes and explicit labelled endings from the outline', async () => {
  await enterCouncil()
  fireEvent.click(screen.getByRole('button', { name: /BeatArrival at the hearing/ }))
  fireEvent.click(screen.getByRole('button', { name: 'Add choice after' }))
  fireEvent.change(screen.getByLabelText('Choice label'), {
    target: { value: 'Ask about the tide' },
  })
  fireEvent.change(screen.getByLabelText('Route destination'), {
    target: { value: `beat:${VERDICT}` },
  })
  fireEvent.change(screen.getByLabelText('Choice requirement rule'), { target: { value: 'never' } })
  fireEvent.click(screen.getByRole('button', { name: 'Add Route effect' }))
  fireEvent.change(screen.getByLabelText('Route effect 1 command name'), {
    target: { value: 'journal_add' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Add condition after' }))
  fireEvent.change(screen.getByLabelText('Outcome condition rule'), { target: { value: 'not' } })
  fireEvent.click(screen.getByRole('button', { name: 'Add end after' }))
  fireEvent.change(screen.getByLabelText('Route destination ending label'), {
    target: { value: 'hearing_complete' },
  })
  const beat = workingScene().beats![0]!
  expect(beat.choices![1]).toMatchObject({
    label: 'Ask about the tide',
    to: { beat: VERDICT },
    requires: 'never',
    effects: [{ command: { name: 'journal_add', args: [] } }],
  })
  expect(beat.outcomes!.map(({ to, when }) => ({ to, when }))).toEqual([
    { to: { unresolved: {} }, when: { not: 'always' } },
    { to: { end: { label: 'hearing_complete' } }, when: undefined },
  ])
  expect(beat.dialogue).toEqual(council().beats![0]!.dialogue)
  expect(calls('narrative_scene_save')).toHaveLength(0)
  fireEvent.click(screen.getByRole('button', { name: 'Undo draft' }))
  expect(workingScene().beats![0]!.outcomes![1]!.to).toEqual({ end: {} })
  fireEvent.click(screen.getByRole('button', { name: 'Redo draft' }))
  expect(workingScene().beats![0]!.outcomes![1]!.to).toEqual({ end: { label: 'hearing_complete' } })
})

it('refuses locked deletion aloud and returns keyboard focus to a surviving beat after deletion', async () => {
  await enterCouncil()
  const arrival = screen.getByRole('button', { name: /BeatArrival at the hearing/ })
  fireEvent.click(within(arrival.closest('li')!).getByRole('button', { name: 'Delete' }))
  expect(screen.getByText(/Unlock protected dialogue/)).toBeInTheDocument()
  expect(useScriptDrafts.getState().drafts[sceneEditKey(PROJECT, SCENE)]).toBeUndefined()
  const verdict = screen.getByRole('button', { name: /BeatThe verdict/ })
  fireEvent.click(verdict)
  const remove = within(verdict.closest('li')!).getByRole('button', { name: 'Delete' })
  remove.focus()
  fireEvent.click(remove)
  await waitFor(() => expect(arrival).toHaveFocus())
  expect(workingScene().beats![0]!.choices![0]!.to).toEqual({ beat: VERDICT })
  expect(workingScene().tombstones).toEqual([
    expect.objectContaining({ target: { beat: VERDICT } }),
  ])
  expect(calls('narrative_scene_save')).toHaveLength(0)
})

it('links an unresolved destination diagnostic to the selected route field', async () => {
  diagnostics = [
    {
      kind: 'choice',
      code: 'unresolved_destination',
      message: 'Choose a destination for this route',
      destination: true,
      beatId: ARRIVAL,
      choiceId: CHOICE,
    },
  ]
  await enterCouncil()
  fireEvent.click(screen.getByRole('button', { name: 'ChoiceShow the logbook' }))
  fireEvent.click(
    await screen.findByRole('button', {
      name: 'Choose a destination for this route — Edit destination',
    }),
  )
  await waitFor(() => expect(screen.getByLabelText('Route destination')).toHaveFocus())
  expect(useUI.getState().narrativeReveal).toMatchObject({
    sceneId: SCENE,
    beatId: ARRIVAL,
    choiceId: CHOICE,
    field: 'destination',
    projectKey: PROJECT,
  })
  expect(calls('narrative_scene_save')).toHaveLength(0)
})

it.each(['library', 'script', 'diagnostic', 'preview'] as const)(
  'reads only the selected scene when Flow opens from a %s reveal',
  async (origin) => {
    useUI.getState().selectNarrative({ sceneId: SCENE }, origin)
    open()
    await screen.findByRole('button', { name: 'Outline list' })
    expect(calls('narrative_scene_get').map((call) => call.sceneId)).toEqual([SCENE])
  },
)
it('reads one compact arc when activated and pages the outline without fetching full scenes', async () => {
  const scenes = Array.from({ length: 120 }, (_, index) =>
    arcScene({ id: mintId(), name: `Scene ${index}`, beats: [] }),
  )
  h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
    Promise.resolve(
      command === 'narrative_arc'
        ? { revision: 'scale', scenes, unreadable: [] }
        : answer(command, args),
    ),
  )
  const view = open(false, false)
  expect(calls('narrative_arc')).toHaveLength(0)
  expect(calls('narrative_scene_get')).toHaveLength(0)
  view.setActive(true)
  fireEvent.click(await screen.findByRole('button', { name: 'Outline list' }))
  expect(screen.getByRole('navigation', { name: 'Arc outline pages' })).toHaveTextContent(
    'Page 1 of 5',
  )
  expect(document.querySelectorAll('.nrt-outline-row')).toHaveLength(25)
  fireEvent.click(screen.getByRole('button', { name: 'Next scenes' }))
  expect(screen.getByRole('navigation', { name: 'Arc outline pages' })).toHaveTextContent(
    'Page 2 of 5',
  )
  expect(calls('narrative_arc')).toHaveLength(1)
  expect(calls('narrative_scene_get')).toHaveLength(0)
})

it('keeps canvas keyboard focus and the refusal announcement when locked deletion is refused', async () => {
  useUI.getState().selectNarrative({ sceneId: SCENE, beatId: ARRIVAL }, 'library')
  open()
  const arrival = (
    await screen.findByTestId(`flow-node-${nodeId.beat(ARRIVAL)}`)
  ).closest<HTMLElement>('.react-flow__node')!
  arrival.focus()
  fireEvent.keyDown(arrival, { key: 'Delete' })
  await waitFor(() =>
    expect(useFlowStore.getState().announcement.text).toMatch(/Unlock protected dialogue/),
  )
  expect(arrival).toHaveFocus()
  expect(useScriptDrafts.getState().drafts[sceneEditKey(PROJECT, SCENE)]).toBeUndefined()
})
it('lets the canonical deletion choose the surviving canvas focus and announcement', async () => {
  useUI.getState().selectNarrative({ sceneId: SCENE, beatId: VERDICT }, 'library')
  open()
  const verdict = (
    await screen.findByTestId(`flow-node-${nodeId.beat(VERDICT)}`)
  ).closest<HTMLElement>('.react-flow__node')!
  verdict.focus()
  fireEvent.keyDown(verdict, { key: 'Delete' })
  const arrival = (
    await screen.findByTestId(`flow-node-${nodeId.beat(ARRIVAL)}`)
  ).closest<HTMLElement>('.react-flow__node')!
  await waitFor(() => expect(arrival).toHaveFocus())
  expect(useUI.getState().narrative.beatId).toBe(ARRIVAL)
  expect(useUI.getState().narrative.choiceId).toBeFalsy()
  expect(useFlowStore.getState().announcement.text).toContain(
    'Incoming routes retain the deleted destination',
  )
  expect(workingScene().beats).toHaveLength(1)
  expect(calls('narrative_scene_save')).toHaveLength(0)
})

it('authors three reconverging canvas routes with keyboard connections and shared Script undo', async () => {
  useUI.getState().selectNarrative({ sceneId: SCENE, beatId: ARRIVAL }, 'library')
  const view = open()
  const canvasNode = async (id: string) =>
    (await screen.findByTestId(`flow-node-${id}`)).closest<HTMLElement>('.react-flow__node')!
  const arrival = await canvasNode(nodeId.beat(ARRIVAL))
  const scrollCanvas = vi.fn()
  arrival.closest<HTMLElement>('.nrt-flow-canvas')!.scrollIntoView = scrollCanvas
  const routeIds: string[] = []
  for (let index = 0; index < 3; index++) {
    fireEvent.click(await canvasNode(nodeId.beat(ARRIVAL)))
    fireEvent.click(screen.getByRole('button', { name: 'Add outcome' }))
    const routeId = workingScene().beats![0]!.outcomes!.at(-1)!.id
    routeIds.push(routeId)
    const route = await canvasNode(nodeId.outcome(routeId))
    await waitFor(() => expect(route).toHaveFocus())
    fireEvent.keyDown(route, { key: 'c' })
    const verdict = await canvasNode(nodeId.beat(VERDICT))
    verdict.focus()
    fireEvent.keyDown(verdict, { key: 'c' })
    expect(workingScene().beats![0]!.outcomes!.at(-1)!.to).toEqual({ beat: VERDICT })
  }
  expect(
    document.querySelectorAll(`[data-testid="flow-node-${nodeId.beat(VERDICT)}"]`),
  ).toHaveLength(1)
  expect(workingScene().beats![0]!.dialogue).toEqual(council().beats![0]!.dialogue)
  expect(scrollCanvas).toHaveBeenCalledWith({ block: 'nearest' })
  const routeId = routeIds[2]!
  const edge = document.querySelector<HTMLElement>(
    `.react-flow__edge[data-id="${nodeId.outcome(routeId)}:then"]`,
  )!
  expect(edge).not.toBeNull()
  edge.focus()
  fireEvent.keyDown(edge, { key: 'Delete' })
  await waitFor(() => expect(screen.getByLabelText('Route destination')).toHaveFocus())
  expect(workingScene().beats![0]!.outcomes!.at(-1)!.to).toEqual({ unresolved: {} })
  expect(screen.getByLabelText('Route destination')).toHaveValue('unresolved')
  expect(useUI.getState().narrative).toMatchObject({ beatId: ARRIVAL, outcomeId: routeId })
  view.showScript()
  fireEvent.click(await screen.findByRole('button', { name: 'Undo draft' }))
  expect(workingScene().beats![0]!.outcomes!.map((route) => route.to)).toEqual(
    routeIds.map(() => ({ beat: VERDICT })),
  )
  expect(calls('narrative_scene_save')).toHaveLength(0)
})

it('edits and collapses outline groups, positions notes and reveals a folded beat without source writes', async () => {
  await enterCouncil()
  fireEvent.click(screen.getByRole('button', { name: /BeatArrival at the hearing/ }))
  fireEvent.click(screen.getByRole('button', { name: 'Groups & notes' }))
  fireEvent.change(screen.getByLabelText('Node X'), { target: { value: '240' } })
  fireEvent.change(screen.getByLabelText('Node Y'), { target: { value: '160' } })
  await waitFor(() =>
    expect(
      (calls('narrative_layout_save').at(-1)!.layout as Layout).nodes[`beat:${ARRIVAL}`],
    ).toMatchObject({
      x: 240,
      y: 160,
    }),
  )
  fireEvent.change(screen.getByLabelText('Group name'), { target: { value: 'Evidence' } })
  fireEvent.click(screen.getByRole('button', { name: 'Create group' }))
  const rename = await screen.findByLabelText('Rename group Evidence')
  fireEvent.change(rename, { target: { value: 'Witnesses' } })
  fireEvent.click(await screen.findByRole('button', { name: 'Close group Witnesses' }))
  await waitFor(() =>
    expect(screen.queryByRole('button', { name: /BeatArrival at the hearing/ })).toBeNull(),
  )
  useUI.getState().selectNarrative({ sceneId: SCENE, beatId: ARRIVAL }, 'diagnostic', {
    projectKey: PROJECT,
    focus: true,
  })
  await waitFor(() =>
    expect(screen.getByRole('button', { name: /BeatArrival at the hearing/ })).toHaveFocus(),
  )
  fireEvent.click(screen.getByRole('button', { name: 'Add pinned note' }))
  fireEvent.change(await screen.findByLabelText('Note X'), { target: { value: '120' } })
  fireEvent.change(screen.getByLabelText('Note Y'), { target: { value: '80' } })
  await waitFor(() =>
    expect(
      Object.values((calls('narrative_layout_save').at(-1)!.layout as Layout).annotations)[0],
    ).toMatchObject({ x: 120, y: 80 }),
  )
  fireEvent.click(screen.getByRole('button', { name: 'Delete group Witnesses' }))
  await waitFor(() =>
    expect(
      Object.keys((calls('narrative_layout_save').at(-1)!.layout as Layout).groups),
    ).toHaveLength(0),
  )
  expect(calls('narrative_scene_save')).toHaveLength(0)
  expect(useScriptDrafts.getState().drafts[sceneEditKey(PROJECT, SCENE)]).toBeUndefined()
  expect(useUndoStack.getState().past).toHaveLength(0)
})
it('applies shared participant, status and warning filters in the outline while retaining errors', async () => {
  diagnostics = [
    {
      kind: 'choice',
      code: 'dangling_beat',
      message: 'Missing destination beat',
      destination: true,
      beatId: ARRIVAL,
      choiceId: CHOICE,
    },
    {
      kind: 'dialogueSlot',
      code: 'missing_text',
      message: 'Wording missing',
      destination: false,
      beatId: ARRIVAL,
      slotId: SLOT,
    },
  ]
  await enterCouncil()
  fireEvent.change(screen.getByLabelText('Participant'), { target: { value: 'Player' } })
  const verdict = screen.getByRole('button', { name: /BeatThe verdict/ }).closest('li')!
  expect(within(verdict).getByText('Filtered')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Needs text' }))
  fireEvent.click(screen.getByText('Badges'))
  fireEvent.click(screen.getByRole('button', { name: /^Warnings/ }))
  expect(screen.queryByText('Wording missing')).toBeNull()
  expect(screen.getByText('Missing destination beat')).toBeInTheDocument()
  const choice = screen.getByRole('button', { name: 'ChoiceShow the logbook' }).closest('li')!
  expect(within(choice).queryByText('Filtered')).toBeNull()
  expect(calls('narrative_scene_save')).toHaveLength(0)
  expect(calls('narrative_layout_save')).toHaveLength(0)
})

it.each(['canvas', 'outline'])(
  'keeps a mixed-work beat visible for each independent status filter in %s',
  async (mode) => {
    const mixed = council()
    mixed.beats![0]!.dialogue![0]!.variants![0]!.text.lifecycle!.review = 'draft'
    h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
      Promise.resolve(
        command === 'narrative_scene_get' && args.sceneId === SCENE
          ? file(mixed)
          : answer(command, args),
      ),
    )
    await enterCouncil()
    if (mode === 'canvas') fireEvent.click(screen.getByRole('button', { name: 'Canvas' }))
    const target =
      mode === 'canvas'
        ? await screen.findByTestId(`flow-node-${nodeId.beat(ARRIVAL)}`)
        : screen.getByRole('button', { name: /BeatArrival at the hearing/ }).closest('li')!
    for (const label of ['Needs text', 'Needs review', 'Out of date']) {
      const toggle = screen.getByRole('button', { name: label })
      fireEvent.click(toggle)
      expect(toggle).toHaveAttribute('aria-pressed', 'true')
      expect(target).not.toHaveClass('is-muted')
      expect(within(target).queryByText(/Filtered/)).toBeNull()
      fireEvent.click(toggle)
    }
    expect(calls('narrative_scene_save')).toHaveLength(0)
    expect(calls('narrative_layout_save')).toHaveLength(0)
  },
)

it('edits authored arc exits through the shared draft and retains them across scene drill-down, Escape and undo', async () => {
  let saved = council()
  h.invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
    if (command === 'narrative_scene_save') saved = args.scene as Scene
    if (command === 'narrative_arc')
      return Promise.resolve({
        revision: 'current',
        scenes: [arcScene(saved), arcScene({ id: OTHER, name: 'The long road', beats: [] })],
        unreadable: [],
      })
    return Promise.resolve(answer(command, args))
  })
  open()
  fireEvent.click(await screen.findByRole('button', { name: 'Outline list' }))
  fireEvent.click(screen.getByRole('button', { name: 'SceneCouncil hearing' }))
  await screen.findByRole('region', { name: 'Selected scene exits' })
  const exit = screen.getByLabelText('Council hearing — Outcome leads to')
  fireEvent.change(exit, { target: { value: '' } })
  expect(workingScene().beats![1]!.outcomes![0]!.to).toEqual({ unresolved: {} })
  expect(workingScene().beats![0]!.dialogue).toEqual(council().beats![0]!.dialogue)
  expect(calls('narrative_scene_save')).toHaveLength(0)
  fireEvent.click(screen.getByRole('button', { name: 'Open selected scene' }))
  await screen.findByTestId(`flow-node-${nodeId.beat(ARRIVAL)}`)
  fireEvent.click(screen.getByRole('button', { name: /Every scene/ }))
  expect(await screen.findByRole('button', { name: 'Canvas' })).toBeInTheDocument()
  expect(screen.getByLabelText('Council hearing — Outcome leads to')).toHaveValue('')
  fireEvent.click(screen.getByRole('button', { name: 'Undo draft' }))
  expect(screen.getByLabelText('Council hearing — Outcome leads to')).toHaveValue(OTHER)
  fireEvent.click(screen.getByRole('button', { name: 'Redo draft' }))
  fireEvent.click(screen.getByRole('button', { name: 'Save scene' }))
  await waitFor(() => expect(calls('narrative_scene_save')).toHaveLength(1))
  await waitFor(() => expect(screen.getByText('Saved scene')).toBeInTheDocument())
  expect(screen.getByLabelText('Council hearing — Outcome leads to')).toHaveValue('')
  expect((calls('narrative_scene_save')[0]!.scene as Scene).beats![1]!.outcomes![0]!.to).toEqual({
    unresolved: {},
  })
})

it('refuses arc exit mutation when unrelated authored integers cannot be represented exactly', async () => {
  const unsafe = {
    ...council(),
    variables: [{ name: 'large', type: 'int', initial: Number.MAX_SAFE_INTEGER + 1 }],
  } as unknown as Scene
  h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
    Promise.resolve(
      command === 'narrative_scene_get' && args.sceneId === SCENE
        ? file(unsafe)
        : answer(command, args),
    ),
  )
  open()
  fireEvent.click(await screen.findByRole('button', { name: 'Outline list' }))
  fireEvent.click(screen.getByRole('button', { name: 'SceneCouncil hearing' }))
  await screen.findByRole('region', { name: 'Selected scene exits' })
  fireEvent.change(screen.getByLabelText('Council hearing — Outcome leads to'), {
    target: { value: '' },
  })
  expect(useScriptDrafts.getState().drafts[sceneEditKey(PROJECT, SCENE)]).toBeUndefined()
  expect(calls('narrative_scene_save')).toHaveLength(0)
  expect(screen.getByText(/integer the webview cannot represent exactly/)).toBeInTheDocument()
})

it('creates a native-ID scene in the arc and connects its first authored beat without entering another editor', async () => {
  const created = file({ id: mintId(), name: 'New scene', beats: [] })
  let made = false
  h.invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
    if (command === 'narrative_scene_create') {
      made = true
      return Promise.resolve(created)
    }
    if (command === 'narrative_scene_get' && args.sceneId === created.scene.id)
      return Promise.resolve(created)
    if (command === 'narrative_arc')
      return Promise.resolve({
        revision: made ? 'created' : 'original',
        scenes: [
          arcScene(council()),
          arcScene({ id: OTHER, name: 'The long road', beats: [] }),
          ...(made ? [arcScene(created.scene)] : []),
        ],
        unreadable: [],
      })
    return Promise.resolve(answer(command, args))
  })
  open()
  fireEvent.click(await screen.findByRole('button', { name: 'Outline list' }))
  fireEvent.click(screen.getByRole('button', { name: 'Add scene' }))
  await screen.findByRole('region', { name: 'Selected scene exits' })
  await waitFor(() => expect(screen.getByRole('button', { name: 'SceneNew scene' })).toHaveFocus())
  expect(calls('narrative_scene_create')).toEqual([{ name: 'New scene' }])
  expect(useUI.getState().narrative.sceneId).toBe(created.scene.id)
  fireEvent.click(await screen.findByRole('button', { name: 'Add first beat for scene exits' }))
  fireEvent.change(screen.getByLabelText('New scene — Exit 1 leads to'), {
    target: { value: SCENE },
  })
  const draft = useScriptDrafts.getState().drafts[sceneEditKey(PROJECT, created.scene.id)]!.scene
  expect(draft.beats![0]!.outcomes![0]!.to).toEqual({ scene: SCENE })
  expect(draft.beats![0]!.id).toMatch(/^[0-7][0-9A-HJKMNP-TV-Z]{25}$/)
  expect(draft.beats![0]!.outcomes![0]!.id).toMatch(/^[0-7][0-9A-HJKMNP-TV-Z]{25}$/)
  expect(calls('narrative_scene_save')).toHaveLength(0)
})

it('restores arc scroll after the first asynchronous read and keeps a Library-selected scene on return', async () => {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  qc.setQueryData(qk.projectCurrent, { path: PROJECT })
  const view = render(
    <QueryClientProvider client={qc}>
      <div className="nrt-panel" role="tabpanel">
        <NarrativeProjectFlow projectKey={PROJECT} readOnly={false} layout={layout} />
      </div>
    </QueryClientProvider>,
  )
  const panel = view.container.querySelector<HTMLElement>('.nrt-panel')!
  fireEvent.click(await screen.findByRole('button', { name: 'Outline list' }))
  fireEvent.click(screen.getByRole('button', { name: 'SceneCouncil hearing' }))
  await screen.findByRole('button', { name: 'Open selected scene' })
  panel.scrollTop = 350
  fireEvent.click(screen.getByRole('button', { name: 'Open selected scene' }))
  await screen.findByTestId(`flow-node-${nodeId.beat(ARRIVAL)}`)
  panel.scrollTop = 800
  await waitFor(() =>
    expect(document.activeElement).toHaveAttribute('data-id', nodeId.beat(ARRIVAL)),
  )
  fireEvent.keyDown(document.activeElement!, { key: 'Escape' })
  await screen.findByRole('region', { name: 'Selected scene exits' })
  expect(panel.scrollTop).toBe(350)
  useUI.getState().selectNarrative({ sceneId: OTHER }, 'library', { projectKey: PROJECT })
  await screen.findByRole('button', { name: /Every scene/ })
  fireEvent.click(screen.getByRole('button', { name: /Every scene/ }))
  expect(await screen.findByRole('button', { name: 'SceneThe long road' })).toHaveAttribute(
    'aria-current',
    'true',
  )
})

it('keeps parsed World quest findings discoverable while arc groups and status filters change', async () => {
  const q = worldQuest(mintId(), 'Inquiry', 'undeclared', [SCENE])
  quests = [q]
  h.invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
    const value = answer(command, args)
    if (command === 'narrative_world_get')
      return Promise.resolve({
        ...(value as object),
        diagnostics: [
          {
            recordId: q.id,
            field: 'initial',
            message: 'The initial stage is not declared in this quest.',
          },
        ],
      })
    return Promise.resolve(value)
  })
  open()
  const summary = await screen.findByText(
    '1 World quest findings. Includes filtered and collapsed quests.',
  )
  fireEvent.click(summary)
  fireEvent.click(screen.getByRole('button', { name: 'Close all groups' }))
  fireEvent.click(screen.getByRole('button', { name: 'Needs review' }))
  expect(screen.getByRole('list', { name: 'World quest findings' })).toHaveTextContent(
    'initial: The initial stage is not declared',
  )
  fireEvent.click(screen.getByRole('button', { name: 'Open quest' }))
  expect(useUI.getState().narrativeWorldTarget).toMatchObject({
    projectKey: PROJECT,
    collection: 'quests',
    recordId: q.id,
  })
  expect(calls('narrative_world_save')).toHaveLength(0)
})
