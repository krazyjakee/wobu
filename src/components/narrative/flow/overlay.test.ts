import { describe, expect, it } from 'vitest'
import type {
  ExecutionTrace,
  PreviewFrame,
  PreviewTraceEvent,
  PreviewTraceSite,
} from '../../../lib/api/narrativePreview'
import type { Scenario } from '../../../lib/api/narrativeScenarios'
import type { FlowScene } from './model'
import {
  arcRoute,
  nodeForSite,
  routeDrift,
  routeFromPreview,
  routeFromScenario,
  type RouteEntry,
} from './overlay'

/**
 * The overlay, read out of traces the runtime actually emits.
 *
 * Every fixture below is shaped like a real `ExecutionTrace`: the runtime
 * records a choice's gate *before* it takes one, records the composite root of
 * a condition last, and rolls a failed action back with `committed: false`.
 * Testing against a tidier invented shape would pass while the real one drew
 * the wrong route.
 */

/* ── ids, in the spelling both sides use ──────────────────────────────────── */

// Real ULIDs, in Crockford base32 and 26 characters: `narrativeIdOf` parses
// them, and an id that only looked like one would make the scenario overlay
// silently mark nothing.
const SCENE = '01J00000000000000000SCENE1'
const BEAT = '01J00000000000000000BEAT01'
const BEAT2 = '01J00000000000000000BEAT02'
const TAKEN = '01J00000000000000000CHZ001'
const BLOCKED = '01J00000000000000000CHZ002'
const OUTCOME = '01J00000000000000000TRN001'

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

/** The gate of the choice that could be taken: `has_logbook = true`, passing. */
const openGate: Records = [
  {
    site: site({ choice: TAKEN }),
    event: {
      kind: 'condition' as const,
      expression: { compare: { var: 'has_logbook', op: 'eq' as const, value: { literal: true } } },
      path: [],
      passed: true,
      inputs: { has_logbook: true },
    },
  },
]

/**
 * The gate of the choice that could not: `all[ has_logbook = true, always ]`,
 * short-circuiting on the first leaf. The composite root is recorded last, as
 * `evaluate_recording` writes it.
 */
const closedGate: Records = [
  {
    site: site({ choice: BLOCKED }),
    event: {
      kind: 'condition' as const,
      expression: { compare: { var: 'has_logbook', op: 'eq' as const, value: { literal: true } } },
      path: [0],
      passed: false,
      inputs: { has_logbook: false },
    },
  },
  {
    site: site({ choice: BLOCKED }),
    event: {
      kind: 'condition' as const,
      expression: {
        all: [
          { compare: { var: 'has_logbook', op: 'eq' as const, value: { literal: true } } },
          'always' as const,
        ],
      },
      path: [],
      passed: false,
      inputs: {},
    },
  },
]

function frame(over: Partial<PreviewFrame> = {}): PreviewFrame {
  return {
    site: site(),
    snapshot: {},
    current: { choices: { scene: SCENE, beat: BEAT, choices: [{ id: TAKEN, label: 'Show it' }] } },
    state: { has_logbook: false },
    trace: trace([]),
    branch: [],
    build: 'build-abcdef123456',
    ...over,
  }
}

describe('the route a preview played', () => {
  it('marks the visited route in order from stable ids alone', () => {
    const entries: RouteEntry[] = [
      { label: 'Started', execution: trace([...openGate, ...closedGate]) },
      {
        label: 'Chose Show it',
        execution: trace([
          ...openGate,
          {
            site: site({ choice: TAKEN }),
            event: { kind: 'transition', to: { beat: BEAT2 } },
          },
          {
            site: site({ choice: TAKEN }),
            event: {
              kind: 'effect',
              index: 0,
              effect: { add: { var: 'trust', by: 10 } },
              before: { trust: 40 },
              after: { trust: 50 },
            },
          },
          {
            site: site({ beat: BEAT2, outcome: OUTCOME }),
            event: { kind: 'transition', to: { end: { label: 'Supported' } } },
          },
        ]),
      },
    ]
    const overlay = routeFromPreview(
      SCENE,
      entries,
      frame({ site: site({ beat: BEAT2 }), current: { end: { label: 'Supported' } } }),
    )

    expect(overlay.route.map((node) => node.id)).toEqual([
      `beat:${BEAT}`,
      `choice:${TAKEN}`,
      `beat:${BEAT2}`,
      `outcome:${OUTCOME}`,
      `end:outcome:${OUTCOME}`,
    ])
    expect(overlay.route.map((node) => node.order)).toEqual([1, 2, 3, 4, 5])
    // The cursor is where the frame says it is, not where the walk happened to
    // stop: the two agree here, and a restore is the case where they would not.
    expect(overlay.currentId).toBe(`beat:${BEAT2}`)
    expect(overlay.nodes.get(`beat:${BEAT2}`)?.mark).toBe('current')
    expect(overlay.build).toBe('build-abcdef123456')

    // What taking the choice wrote, from the effect record and nowhere else.
    expect(overlay.nodes.get(`choice:${TAKEN}`)?.effects).toEqual([
      { wrote: 'trust + 10', values: [{ name: 'trust', before: '40', after: '50' }] },
    ])
  })

  it('separates a branch that was open and not taken from one that was closed', () => {
    const entries: RouteEntry[] = [
      { label: 'Started', execution: trace([...openGate, ...closedGate]) },
    ]
    const overlay = routeFromPreview(SCENE, entries, frame())

    expect(overlay.nodes.get(`choice:${TAKEN}`)?.mark).toBe('open')
    const blocked = overlay.nodes.get(`choice:${BLOCKED}`)
    expect(blocked?.mark).toBe('blocked')
    // Named at the authored field, in the writer's own words, with the value
    // the runtime read there — and nothing about expression paths or compiled
    // artefacts, which is the difference the acceptance criterion turns on.
    expect(blocked?.block).toEqual({
      field: `choice:${BLOCKED}.requires`,
      gate: 'has_logbook = true and always',
      failed: 'has_logbook = true',
      values: [{ name: 'has_logbook', value: 'false' }],
      explained: true,
    })
  })

  it('reads the current branch from the frame when a restore left no trace', () => {
    // Exactly what `narrative_preview_step` returns for a checkpoint restore:
    // the action performed nothing, so the trace is empty.
    const overlay = routeFromPreview(
      SCENE,
      [{ label: 'Restored snapshot', execution: trace([]) }],
      frame({
        branch: [
          { id: TAKEN, label: 'Show it', available: true, records: openGate },
          { id: BLOCKED, label: 'Show logbook', available: false, records: closedGate },
        ],
      }),
    )
    expect(overlay.nodes.get(`choice:${TAKEN}`)?.mark).toBe('open')
    expect(overlay.nodes.get(`choice:${BLOCKED}`)?.block?.failed).toBe('has_logbook = true')
  })

  it('draws nothing for an action that was rolled back', () => {
    const rolled: ExecutionTrace = {
      committed: false,
      error: 'choice X is unavailable',
      omitted: 0,
      records: [
        { site: site({ choice: BLOCKED }), event: { kind: 'transition', to: { beat: BEAT2 } } },
      ],
    }
    const overlay = routeFromPreview(SCENE, [{ label: 'Refused', execution: rolled }], frame())
    expect(overlay.route).toHaveLength(0)
    expect(overlay.nodes.has(`choice:${BLOCKED}`)).toBe(false)
  })

  it('counts a loop rather than reordering the route', () => {
    const pass = (beat: string) =>
      trace([{ site: site({ beat }), event: { kind: 'transition', to: { beat } } }])
    const overlay = routeFromPreview(
      SCENE,
      [
        { label: 'One', execution: pass(BEAT) },
        { label: 'Two', execution: pass(BEAT2) },
        { label: 'Three', execution: pass(BEAT) },
      ],
      frame(),
    )
    expect(overlay.route.map((node) => node.id)).toEqual([`beat:${BEAT}`, `beat:${BEAT2}`])
    expect(overlay.nodes.get(`beat:${BEAT}`)?.visits).toBe(2)
  })

  it('names the box a recorded step happened at', () => {
    expect(nodeForSite(site({ choice: TAKEN }))).toBe(`choice:${TAKEN}`)
    expect(nodeForSite(site({ outcome: OUTCOME }))).toBe(`outcome:${OUTCOME}`)
    expect(nodeForSite(site({ slot: 'slot' }))).toBe(`beat:${BEAT}`)
    expect(nodeForSite({ ...site(), beat: null })).toBeNull()
  })
})

/* ── a saved scenario, opened rather than replayed ────────────────────────── */

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
        out: [],
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

const scenario: Scenario = {
  version: 1,
  scene: SCENE,
  initial_state: {},
  seed: 0,
  commands: {},
  steps: [
    { action: null, expect: { boundary: { kind: 'line', scene: SCENE, beat: BEAT } } },
    {
      action: { kind: 'advance' },
      expect: { boundary: { kind: 'choices', scene: SCENE, beat: BEAT, ids: [TAKEN] } },
    },
    {
      action: { kind: 'choose', choice: TAKEN },
      expect: { boundary: { kind: 'end', scene: SCENE, beat: BEAT } },
    },
  ],
}

describe('a saved scenario opened as an overlay', () => {
  it('marks the route and the branch it was not offered, and says what it cannot say', () => {
    const overlay = routeFromScenario('High trust', scenario, scene())
    expect(overlay.origin).toBe('scenario')
    expect(overlay.route.map((node) => node.id)).toEqual([`beat:${BEAT}`, `choice:${TAKEN}`])
    // Not offered by the tape, and authored by the beat: unavailable by
    // construction rather than by inference.
    expect(overlay.nodes.get(`choice:${BLOCKED}`)?.mark).toBe('blocked')
    expect(overlay.nodes.get(`choice:${BLOCKED}`)?.block?.explained).toBe(false)
    expect(
      overlay.limits.some(
        (limit) => limit.includes('not being replayed') || limit.includes('not a run'),
      ),
    ).toBe(true)
  })
})

describe('what the overlay refuses to claim', () => {
  it('gives the arc a played badge per scene, and says why that is all', () => {
    const overlay = arcRoute([SCENE])
    expect(overlay.nodes.get(SCENE)?.mark).toBe('played')
    expect(overlay.route).toHaveLength(0)
    expect(overlay.limits.join(' ')).toContain('one scene at a time')
  })

  it('says the scene moved on since the build it is pinned to', () => {
    const overlay = routeFromPreview(
      SCENE,
      [{ label: 'Started', execution: trace([...openGate, ...closedGate]) }],
      frame(),
    )
    const trimmed = scene()
    trimmed.elements = trimmed.elements.filter((one) => one.id !== `choice:${BLOCKED}`)
    const drift = routeDrift(overlay, trimmed, true)
    expect(drift.join(' ')).toContain('unsaved edits')
    expect(drift.join(' ')).toContain('1 element on this route is no longer in the scene')
    // A scene that has not moved raises nothing at all.
    expect(routeDrift(overlay, scene(), false)).toEqual([])
  })
})
