import { act, fireEvent, render, renderHook, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type {
  ExecutionTrace,
  PreviewFrame,
  PreviewTraceEvent,
  PreviewTraceSite,
} from '../../../lib/api/narrativePreview'
import { usePreviewSessions } from '../previewStore'
import { PreviewTrace } from '../PreviewTrace'
import { FlowCanvas } from './FlowCanvas'
import { resetFlowStore, useFlowStore } from './flowStore'
import type { FlowScene } from './model'
import { PreviewOverlayProvider, RouteBanner, RouteDetail, RouteLegend } from './PreviewOverlay'
import type { Scenario } from '../../../lib/api/narrativeScenarios'
import {
  routeDrift,
  routeFromPreview,
  usePlayedScenes,
  usePreviewRoute,
  usePreviewRouteFocus,
  type PreviewOverlayValue,
} from './overlay'

/**
 * The overlay on a real canvas.
 *
 * jsdom cannot answer anything about geometry, so nothing here claims a box was
 * *seen*: the assertions are about the words and glyphs a marked node carries,
 * which is the encoding #188 requires precisely because colour and position are
 * not enough on their own.
 */

const SCENE = '01J00000000000000000SCENE1'
const BEAT = '01J00000000000000000BEAT01'
const TAKEN = '01J00000000000000000CHZ001'
const BLOCKED = '01J00000000000000000CHZ002'

const site = (over: Partial<PreviewTraceSite> = {}): PreviewTraceSite => ({
  scene: SCENE,
  beat: BEAT,
  choice: null,
  outcome: null,
  slot: null,
  variant: null,
  ...over,
})
type Records = { site: PreviewTraceSite; event: PreviewTraceEvent }[]

const trace = (records: Records): ExecutionTrace => ({
  committed: true,
  error: null,
  omitted: 0,
  records,
})
const gate = (choice: string, passed: boolean, value: boolean): Records => [
  {
    site: site({ choice }),
    event: {
      kind: 'condition' as const,
      expression: { compare: { var: 'has_logbook', op: 'eq' as const, value: { literal: true } } },
      path: [],
      passed,
      inputs: { has_logbook: value },
    },
  },
]

function scene(): FlowScene {
  return {
    id: SCENE,
    name: 'Council hearing',
    groups: [],
    entryId: `beat:${BEAT}`,
    elements: [
      {
        kind: 'beat',
        id: `beat:${BEAT}`,
        title: 'Evidence',
        beatId: BEAT,
        participants: [],
        lines: 1,
        variants: 1,
        out: [
          { id: `choice:${TAKEN}`, label: 'Show it', to: `choice:${TAKEN}`, fixed: true },
          {
            id: `choice:${BLOCKED}`,
            label: 'Show logbook',
            requires: 'has_logbook = true',
            to: `choice:${BLOCKED}`,
            fixed: true,
          },
        ],
      },
      {
        kind: 'choice',
        id: `choice:${TAKEN}`,
        title: 'Show it',
        beatId: BEAT,
        effects: [],
        out: [],
      },
      {
        kind: 'choice',
        id: `choice:${BLOCKED}`,
        title: 'Show logbook',
        beatId: BEAT,
        effects: [],
        out: [],
      },
    ],
  }
}

function frame(): PreviewFrame {
  return {
    site: site(),
    snapshot: {},
    current: { choices: { scene: SCENE, beat: BEAT, choices: [{ id: TAKEN, label: 'Show it' }] } },
    state: { has_logbook: false },
    trace: trace([]),
    branch: [
      { id: TAKEN, label: 'Show it', available: true, records: gate(TAKEN, true, true) },
      {
        id: BLOCKED,
        label: 'Show logbook',
        available: false,
        records: gate(BLOCKED, false, false),
      },
    ],
    build: 'buildabcdef0123456',
  }
}

function overlayValue(edited = false): PreviewOverlayValue {
  const level = scene()
  const overlay = routeFromPreview(
    SCENE,
    [{ label: 'Started', execution: trace([...gate(TAKEN, true, true)]) }],
    frame(),
  )
  return { overlay, drift: routeDrift(overlay, level, edited), key: `project:${SCENE}` }
}

const nodeEl = (container: HTMLElement, id: string) =>
  container.querySelector<HTMLElement>(`[data-testid="flow-node-${id}"]`)!

beforeEach(() => {
  resetFlowStore()
  usePreviewRouteFocus.setState({ step: null, node: null })
  usePreviewSessions.setState({ sessions: {}, diagnostics: {}, busy: {}, opened: {} })
})

describe('the route on the canvas', () => {
  it('marks the current position, the open branch and the closed one in words', () => {
    const value = overlayValue()
    const { container } = render(
      <PreviewOverlayProvider value={value}>
        <FlowCanvas scene={scene()} onChange={() => {}} layout={() => new Promise(() => {})} />
      </PreviewOverlayProvider>,
    )

    // The three marks the acceptance criterion names, each carried as a word on
    // the box rather than as a tint: this assertion passes in greyscale.
    expect(nodeEl(container, `beat:${BEAT}`)).toHaveAttribute('data-route', 'current')
    expect(nodeEl(container, `beat:${BEAT}`)).toHaveTextContent('Here now')
    expect(nodeEl(container, `choice:${TAKEN}`)).toHaveAttribute('data-route', 'open')
    expect(nodeEl(container, `choice:${TAKEN}`)).toHaveTextContent('Open, not taken')
    expect(nodeEl(container, `choice:${BLOCKED}`)).toHaveAttribute('data-route', 'blocked')
    expect(nodeEl(container, `choice:${BLOCKED}`)).toHaveTextContent('Unavailable')

    // A legend, and only for the marks that are on this canvas.
    const legend = screen.getByRole('list', { name: 'Preview route legend' })
    expect(legend).toHaveTextContent('Here now')
    expect(legend).toHaveTextContent('Unavailable')
    expect(legend).not.toHaveTextContent('Played')
  })

  it('draws nothing at all when no run has been played', () => {
    const { container } = render(
      <PreviewOverlayProvider value={null}>
        <FlowCanvas scene={scene()} onChange={() => {}} layout={() => new Promise(() => {})} />
      </PreviewOverlayProvider>,
    )
    expect(nodeEl(container, `beat:${BEAT}`)).not.toHaveAttribute('data-route')
    expect(screen.queryByRole('list', { name: 'Preview route legend' })).toBeNull()
  })

  it('never writes the scene or a position while it is drawn', () => {
    const onChange = vi.fn()
    const onPositionsChange = vi.fn()
    render(
      <PreviewOverlayProvider value={overlayValue()}>
        <FlowCanvas
          scene={scene()}
          onChange={onChange}
          onPositionsChange={onPositionsChange}
          layout={() => new Promise(() => {})}
        />
      </PreviewOverlayProvider>,
    )
    // Reading a route is a read. Nothing about drawing it reaches the document
    // the canvas is editing, or the arrangement sidecar beside it.
    expect(onChange).not.toHaveBeenCalled()
    expect(onPositionsChange).not.toHaveBeenCalled()
  })

  it('points the Preview transcript at a step when a marked node is opened', () => {
    const value = overlayValue()
    const { container } = render(
      <PreviewOverlayProvider value={value}>
        <FlowCanvas scene={scene()} onChange={() => {}} layout={() => new Promise(() => {})} />
      </PreviewOverlayProvider>,
    )
    fireEvent.click(nodeEl(container, `beat:${BEAT}`).querySelector('button.nrt-route-cap')!)
    expect(usePreviewRouteFocus.getState().step).toMatchObject({
      key: `project:${SCENE}`,
      step: 0,
    })
    // The cap's click is the cap's: the authoring cursor did not move with it.
    expect(useFlowStore.getState().selectedId).toBeNull()
  })
})

describe('why a branch was closed', () => {
  it('names the authored field, the test that failed and the value it read', () => {
    const { rerender } = render(
      <PreviewOverlayProvider value={overlayValue()}>
        <RouteDetail />
      </PreviewOverlayProvider>,
    )
    // Nothing until a box is chosen: the panel answers a question about a
    // selection, and an unsolicited explanation of an arbitrary node is noise.
    expect(screen.queryByRole('table')).toBeNull()

    act(() => useFlowStore.getState().select(`choice:${BLOCKED}`))
    rerender(
      <PreviewOverlayProvider value={overlayValue()}>
        <RouteDetail />
      </PreviewOverlayProvider>,
    )
    expect(screen.getByText(`choice:${BLOCKED}.requires`)).toBeTruthy()
    expect(screen.getAllByText('has_logbook = true').length).toBeGreaterThan(0)
    const values = screen.getByRole('table', { name: 'Values when this branch was evaluated' })
    expect(values).toHaveTextContent('has_logbook')
    expect(values).toHaveTextContent('false')
  })
})

describe('what the overlay says about itself', () => {
  it('names the build it is pinned to, and what changed since', () => {
    render(
      <PreviewOverlayProvider value={overlayValue(true)}>
        <RouteBanner />
      </PreviewOverlayProvider>,
    )
    expect(screen.getByText(/Pinned to build buildabcdef0\./)).toBeTruthy()
    expect(screen.getByText(/unsaved edits/)).toBeTruthy()
    // The limitation the arc view lives under is stated in the pane, not only
    // in a comment.
    expect(screen.getByRole('list', { name: 'What this overlay does not show' })).toHaveTextContent(
      'one scene at a time',
    )
  })

  it('renders a legend row only for marks that are present', () => {
    render(
      <PreviewOverlayProvider value={overlayValue()}>
        <RouteLegend />
      </PreviewOverlayProvider>,
    )
    expect(screen.getByRole('list', { name: 'Preview route legend' })).toHaveTextContent('Here now')
  })
})

describe('the transcript side of the cursor', () => {
  it('scrolls to and marks the step the canvas asked for', () => {
    const scroll = vi.fn()
    Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
      configurable: true,
      value: scroll,
    })
    const centre = vi.fn()
    render(
      <PreviewTrace
        entries={[
          { label: 'Started', execution: trace([]) },
          { label: 'Chose Show it', execution: trace([...gate(TAKEN, true, true)]) },
        ]}
        openSource={() => {}}
        focusStep={1}
        onCentre={centre}
      />,
    )
    expect(scroll).toHaveBeenCalledWith({ block: 'center' })
    expect(screen.getByText('Chose Show it').closest('li')).toHaveAttribute('aria-current', 'step')

    fireEvent.click(screen.getByRole('button', { name: 'Centre in Flow' }))
    expect(centre).toHaveBeenCalledWith(expect.objectContaining({ choice: TAKEN }))
  })
})

describe('where the overlay comes from', () => {
  const key = `project:${SCENE}`
  const scenario: Scenario = {
    version: 1,
    scene: SCENE,
    initial_state: {},
    seed: 0,
    commands: {},
    steps: [
      {
        action: { kind: 'advance' },
        expect: { boundary: { kind: 'choices', scene: SCENE, beat: BEAT, ids: [TAKEN] } },
      },
    ],
  }

  it('reads the live run, and lets an opened scenario take its place', () => {
    act(() =>
      usePreviewSessions.getState().put(key, {
        graph: {},
        frame: frame(),
        trace: [{ label: 'Started', execution: trace([...gate(TAKEN, true, true)]) }],
      }),
    )
    const level = scene()
    const { result, rerender } = renderHook(() => usePreviewRoute('project', SCENE, level, false))
    expect(result.current?.overlay.origin).toBe('preview')
    expect(result.current?.overlay.build).toBe('buildabcdef0123456')

    // Opening a saved scenario replaces the live overlay and starts nothing:
    // the session it was drawn from is still exactly where it was.
    act(() => usePreviewSessions.getState().open(key, { name: 'High trust', scenario }))
    rerender()
    expect(result.current?.overlay.origin).toBe('scenario')
    expect(result.current?.overlay.name).toBe('High trust')
    expect(usePreviewSessions.getState().sessions[key]?.frame).toEqual(frame())

    act(() => usePreviewSessions.getState().open(key, null))
    rerender()
    expect(result.current?.overlay.origin).toBe('preview')
  })

  it('gives the arc the played scenes and nothing else', () => {
    const { result, rerender } = renderHook(() => usePlayedScenes('project'))
    expect(result.current).toBeNull()
    act(() =>
      usePreviewSessions.getState().put(key, {
        graph: {},
        frame: frame(),
        trace: [],
      }),
    )
    rerender()
    expect(result.current?.overlay.origin).toBe('arc')
    expect(result.current?.overlay.nodes.get(SCENE)?.mark).toBe('played')
  })
})
