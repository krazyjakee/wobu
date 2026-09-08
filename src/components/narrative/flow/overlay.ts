import { createContext, useContext, useEffect, useMemo, useRef } from 'react'
import { create } from 'zustand'
import type {
  ExecutionTrace,
  PreviewFrame,
  PreviewState,
  PreviewTraceEvent,
  PreviewTraceSite,
} from '../../../lib/api/narrativePreview'
import type { Scenario } from '../../../lib/api/narrativeScenarios'
import type { Destination } from '../../../lib/api'
import { usePreviewSessions } from '../previewStore'
import type { FlowScene } from './model'
import { conditionText, effectRows, narrativeIdOf, nodeId } from './source'

/**
 * The route a Preview run took, as marks on canvas nodes.
 *
 * ── the one rule this file exists to keep ────────────────────────────────────
 *
 * **The overlay is a reading of the trace, never a second copy of it.** Nothing
 * below stores a route: `routeFromPreview` is a pure function of the frames the
 * runtime already returned, recomputed whenever they change. That is what makes
 * "the overlay updates on restart, checkpoint restore and step-back" fall out
 * rather than needing three code paths — a restore is simply a different list
 * of frames, and the same function reads it.
 *
 * It is also why nothing here can modify layout, source or canon: it is handed
 * a `FlowScene` and a list of frames and returns marks. There is no writer to
 * forget to leave out.
 *
 * ── no text matching, and no inference ───────────────────────────────────────
 *
 * Every mark is keyed by a **stable narrative id** the runtime put in a
 * `TraceSite` — `site.beat`, `site.choice`, `site.outcome` — mapped through
 * `nodeId` into the canvas's own spelling. Nothing compares a label, a title or
 * a line of dialogue, and nothing guesses which choice a step "probably" meant.
 * The one place that would be tempting is *why* a branch is closed, and the
 * answer arrives as recorded `TraceEvent::Condition` records carrying the
 * authored condition and the values it read, so the explanation is a rendering
 * of evidence rather than a reconstruction of it.
 *
 * A rolled-back action is skipped entirely. `ExecutionTrace.committed` is false
 * when every tentative write was undone, and drawing its records would show a
 * writer a route the run never took.
 *
 * ── what a saved scenario can and cannot say ─────────────────────────────────
 *
 * A scenario (#162) is a tape of *boundaries and actions*, not of evaluations:
 * it records which choices were offered at a branch and which one was taken,
 * and no condition at all. So `routeFromScenario` can mark the played route and
 * can mark a branch closed — a choice the scene authors and the tape did not
 * offer was, by construction, unavailable — but it cannot say which values
 * closed it. That gap is stated in `limits` and rendered verbatim, rather than
 * filled in by re-running the scenario behind the writer's back.
 */

/* ── what a node is wearing ───────────────────────────────────────────────── */

/**
 * The four things the overlay says about a node.
 *
 * Deliberately four independent marks and not a score: "offered and not taken"
 * and "could not be taken" are the distinction US-07 is entirely about, and a
 * single "not played" state would collapse exactly the two the designer opened
 * the canvas to tell apart.
 */
export type RouteMark = 'played' | 'current' | 'open' | 'blocked'

/**
 * The word, glyph and sentence each mark wears.
 *
 * Every mark carries all three, so none of them is a colour. A canvas that
 * distinguished a closed branch from an open one by tint alone would be
 * unreadable to a designer who cannot separate those tints, and #151 rules that
 * out for status generally — this is the same rule applied to a derived view.
 */
export const ROUTE_MARKS: Record<RouteMark, { label: string; icon: string; means: string }> = {
  played: { label: 'Played', icon: 'check', means: 'The run went through here, in this order.' },
  current: { label: 'Here now', icon: 'pin', means: 'Where the run is stopped.' },
  open: {
    label: 'Open, not taken',
    icon: 'chev',
    means: 'Offered at a branch the run reached, and not chosen.',
  },
  blocked: {
    label: 'Unavailable',
    icon: 'lock',
    means: 'Its condition did not hold. Select it to read the values that closed it.',
  },
}

/** One effect the route applied, as authored and as it landed. */
export interface RouteEffect {
  /** The authored effect, in the writer's own terms: `trust + 10`. */
  wrote: string
  /** Only the variables the effect touched, before and after it ran. */
  values: { name: string; before: string; after: string }[]
}

/**
 * Why a branch was closed, in the words of the field that closed it.
 *
 * `field` is the canvas's own `element.port` spelling, the same one
 * `FlowDiagnostic` uses, so "take me to the thing I have to change" is one
 * lookup rather than a translation table. Nothing here names a compiled
 * artefact: `gate` and `failed` are renderings of the authored `Condition` the
 * runtime recorded, and `values` are the variables it actually read.
 */
export interface RouteBlock {
  field: string
  /** The whole gate, as written. Null when the record carried no condition. */
  gate: string | null
  /** The innermost part of the gate that did not hold. */
  failed: string | null
  /** Every variable the gate read, with the value it read there. */
  values: { name: string; value: string }[]
  /**
   * False when availability is recorded but its reason is not.
   *
   * A saved scenario is the case: it knows a choice was not offered and cannot
   * know why. Saying so is the point of the field — an explanation panel that
   * simply showed nothing would read as a rendering fault.
   */
  explained: boolean
}

/** One node, and everything the overlay has to say about it. */
export interface RouteNode {
  /** The canvas node id: `beat:…`, `choice:…`, `outcome:…`, `end:…`, `link:…`. */
  id: string
  mark: RouteMark
  /** Position along the played route, 1-based. Null on a node never entered. */
  order: number | null
  /** How many times the route passed through. Above one means a loop. */
  visits: number
  /** The recorded step this node belongs to — the row in the Preview list. */
  step: number
  block?: RouteBlock
  effects?: RouteEffect[]
}

/** Where an overlay came from, which decides what it is allowed to claim. */
export type RouteOrigin = 'preview' | 'scenario' | 'arc'

export interface RouteOverlay {
  origin: RouteOrigin
  /** The name of the saved scenario, when that is what is being drawn. */
  name?: string
  /**
   * The compiled content the run is pinned to.
   *
   * Null for a scenario and for the arc, which are read from a tape and from a
   * set of scene ids respectively and belong to no particular compilation.
   */
  build: string | null
  /** The scene this route belongs to. An overlay is never drawn over another. */
  sceneId: string
  /** Every marked node, by canvas node id. What the node faces read. */
  nodes: ReadonlyMap<string, RouteNode>
  /** The played nodes alone, first visit first. What the ordered list draws. */
  route: readonly RouteNode[]
  /** Where the run is stopped, or null when the route is a finished tape. */
  currentId: string | null
  /** What this overlay cannot say. Rendered verbatim, never summarised away. */
  limits: readonly string[]
}

/* ── the preview run ──────────────────────────────────────────────────────── */

/** One recorded action and everything the runtime observed while performing it. */
export interface RouteEntry {
  label: string
  execution: ExecutionTrace
}

/**
 * The holder a site names, as a canvas node id.
 *
 * A choice wins over an outcome because a site never carries both, and the
 * order is only a tie-break that cannot happen. Null at a site that names
 * neither — a scene entry check, or a dialogue line.
 */
function holderOf(site: PreviewTraceSite): string | null {
  if (site.choice) return nodeId.choice(site.choice)
  if (site.outcome) return nodeId.outcome(site.outcome)
  return null
}

/**
 * The canvas node a recorded step happened at.
 *
 * The choice or outcome when the step names one, and the beat otherwise — the
 * same precedence the marks use, so "centre on this step" lands on the box the
 * step is drawn on rather than on the beat around it. Null at a scene-entry
 * check, which happens at no box.
 */
export function nodeForSite(site: PreviewTraceSite): string | null {
  return holderOf(site) ?? (site.beat ? nodeId.beat(site.beat) : null)
}

/** The authored field a gate lives on, in the canvas's `element.port` spelling. */
function gateField(site: PreviewTraceSite): string {
  if (site.choice) return `${nodeId.choice(site.choice)}.requires`
  if (site.outcome) return `${nodeId.outcome(site.outcome)}.when`
  return `${site.beat ? nodeId.beat(site.beat) : site.scene}.entry`
}

/**
 * The derived box a destination is drawn as, or null when it is a beat.
 *
 * The same three cases `sceneToFlow` draws, read from the same holder id, so an
 * ending the route reached is the very box on the canvas rather than a second
 * one this file invented.
 */
function destinationNode(holder: string, to: Destination): string | null {
  if ('end' in to) return nodeId.end(holder)
  if ('scene' in to) return nodeId.link(holder)
  return null
}

/** One gate evaluation: the contiguous run of records for a single condition. */
interface Gate {
  node: string
  field: string
  passed: boolean
  records: { site: PreviewTraceSite; event: PreviewTraceEvent }[]
  step: number
}

/**
 * The failing part of a gate, and every value it read.
 *
 * The failing record chosen is the one with the **longest path**, ties broken
 * by the last recorded. `evaluate_recording` writes children before the
 * composite that contains them, so the longest failing path is the innermost
 * thing that actually did not hold — `has_logbook = true` rather than the `all`
 * around it. A `not(…)` that failed because its child *passed* has no failing
 * child at all, so the root is correctly the most specific answer there.
 *
 * Values are merged across the whole gate because only comparison leaves claim
 * reads: the composite records carry no inputs, and a designer asking "why" is
 * asking about the leaf's variables.
 */
function explain(gate: Gate): RouteBlock {
  let failed: { path: number[]; text: string | null } | null = null
  const values = new Map<string, string>()
  let root: string | null = null
  for (const { event } of gate.records) {
    if (event.kind !== 'condition') continue
    for (const [name, value] of Object.entries(event.inputs)) values.set(name, String(value))
    if (event.path.length === 0) root = conditionText(event.expression)
    if (event.passed) continue
    if (failed === null || event.path.length >= failed.path.length) {
      failed = { path: event.path, text: conditionText(event.expression) }
    }
  }
  return {
    field: gate.field,
    gate: root,
    failed: failed?.text ?? null,
    values: [...values].map(([name, value]) => ({ name, value })),
    explained: true,
  }
}

/** The value a state map holds for a name, as a word. Absent reads as `—`. */
function valueOf(state: PreviewState, name: string): string {
  return name in state ? String(state[name]) : '—'
}

/**
 * The route a Preview session played, from the frames it already produced.
 *
 * Pure and independent of React: this is the function the tests pin, so it has
 * to be callable with nothing but a scene id, a list of recorded actions and
 * the latest frame.
 */
export function routeFromPreview(
  sceneId: string,
  entries: readonly RouteEntry[],
  frame: PreviewFrame,
): RouteOverlay {
  const nodes = new Map<string, RouteNode>()
  const route: RouteNode[] = []
  const gates = new Map<string, Gate>()
  let open: Gate | null = null

  const play = (id: string, step: number) => {
    const seen = nodes.get(id)
    if (seen) {
      seen.visits += 1
      return
    }
    const node: RouteNode = { id, mark: 'played', order: route.length + 1, visits: 1, step }
    nodes.set(id, node)
    route.push(node)
  }
  const closeGate = () => {
    if (open) gates.set(open.node, open)
    open = null
  }

  entries.forEach((entry, step) => {
    // A rolled-back action changed nothing, so it went nowhere. Drawing its
    // records would show a writer a route the run did not take.
    if (!entry.execution.committed) return
    for (const record of entry.execution.records) {
      const { site, event } = record
      if (site.beat) play(nodeId.beat(site.beat), step)
      const holder = holderOf(site)

      if (event.kind === 'condition' && holder) {
        if (open === null || open.node !== holder) {
          closeGate()
          open = { node: holder, field: gateField(site), passed: false, records: [], step }
        }
        const gate: Gate = open
        gate.records.push(record)
        // The composite root is recorded last, so its result is the gate's.
        if (event.path.length === 0) gate.passed = event.passed
        continue
      }
      closeGate()

      if (event.kind === 'transition' && holder) {
        play(holder, step)
        const destination = destinationNode(holder, event.to)
        if (destination) play(destination, step)
      }
      if (event.kind === 'effect' && holder) {
        const node = nodes.get(holder)
        if (!node) continue
        const wrote = effectRows([event.effect])[0]
        node.effects = [
          ...(node.effects ?? []),
          {
            wrote: wrote ? `${wrote.variable} ${wrote.operator} ${wrote.value}` : '',
            values: Object.keys(event.after).map((name) => ({
              name,
              before: valueOf(event.before, name),
              after: valueOf(event.after, name),
            })),
          },
        ]
      }
    }
    closeGate()
  })

  /*
   * The current branch, last and authoritative.
   *
   * `frame.branch` is evaluated against the state the run is *stopped* in,
   * where the records above were written on the way there. After a checkpoint
   * restore it is the only evidence there is: the restore performed no action,
   * so its trace is empty.
   */
  const step = Math.max(entries.length - 1, 0)
  for (const status of frame.branch) {
    const node = nodeId.choice(status.id)
    gates.set(node, {
      node,
      field: `${node}.requires`,
      passed: status.available,
      records: status.records,
      step,
    })
  }

  // Played wins over offered: a choice the run took is on the route, whatever
  // its gate says about it now.
  for (const [id, gate] of gates) {
    if (nodes.has(id)) continue
    nodes.set(id, {
      id,
      mark: gate.passed ? 'open' : 'blocked',
      order: null,
      visits: 0,
      step: gate.step,
      ...(gate.passed ? {} : { block: explain(gate) }),
    })
  }

  /*
   * Where the run is stopped.
   *
   * The frame's own site when it is still in this scene, and the last box the
   * route reached when it is not — an ending, or a way out into another scene.
   * Both are facts the runtime stated; neither is a guess about where a player
   * "probably" is.
   */
  const here =
    frame.site && frame.site.scene === sceneId && frame.site.beat
      ? nodeId.beat(frame.site.beat)
      : (route[route.length - 1]?.id ?? null)
  const current = here === null ? null : nodes.get(here)
  if (current) current.mark = 'current'

  return {
    origin: 'preview',
    build: frame.build,
    sceneId,
    nodes,
    route,
    currentId: current ? current.id : null,
    limits: [ARC_LIMIT],
  }
}

/**
 * Said on every overlay, because it is true of every overlay.
 *
 * Preview plays one scene at a time, so a route that leaves through a scene
 * link stops at the link. Until cross-scene preview exists the arc can only
 * report *that* a scene was played, and a designer who was not told would read
 * an unmarked scene as one the story never reaches.
 */
export const ARC_LIMIT =
  'Preview plays one scene at a time, so the arc view shows only which scenes were played, not the route between them.'

/* ── a saved scenario, opened rather than replayed ────────────────────────── */

/** Said on a scenario overlay: a tape records decisions, not evaluations. */
export const SCENARIO_LIMITS = [
  'This is a saved scenario opened as an overlay, not a run. Its conditions were not re-evaluated, so a closed branch is shown without the values that closed it. Start the preview from this scenario to see them.',
  'A scenario records the boundaries it stopped at, so automatic outcomes between beats are not marked.',
]

/**
 * A saved scenario, drawn as a route without playing it.
 *
 * The tape carries stable ids at every step — the beat it stopped in, the ids
 * offered at a branch, the choice taken — so this needs the scene only to know
 * which choices *exist* at a branch. A choice the beat authors and the tape did
 * not offer was unavailable at that moment, by construction rather than by
 * inference: `assertFrame` records the runtime's own `Yield::Choices` list.
 */
export function routeFromScenario(
  name: string,
  scenario: Scenario,
  scene: FlowScene,
): RouteOverlay {
  const nodes = new Map<string, RouteNode>()
  const route: RouteNode[] = []
  const play = (id: string, step: number) => {
    const seen = nodes.get(id)
    if (seen) {
      seen.visits += 1
      return
    }
    const node: RouteNode = { id, mark: 'played', order: route.length + 1, visits: 1, step }
    nodes.set(id, node)
    route.push(node)
  }
  const offered = new Map<string, { ids: string[]; step: number }>()

  scenario.steps.forEach((step, index) => {
    const boundary = step.expect.boundary
    if (boundary?.kind === 'choices' && boundary.beat) {
      offered.set(boundary.beat, { ids: boundary.ids ?? [], step: index })
    }
    if (step.action?.kind === 'choose') play(nodeId.choice(step.action.choice), index)
    if (boundary && 'beat' in boundary && boundary.beat) play(nodeId.beat(boundary.beat), index)
  })

  for (const element of scene.elements) {
    if (element.kind !== 'choice' || nodes.has(element.id)) continue
    const branch = element.beatId ? offered.get(element.beatId) : undefined
    if (!branch) continue
    const id = narrativeIdOf(element.id)?.id
    if (!id) continue
    const available = branch.ids.includes(id)
    nodes.set(element.id, {
      id: element.id,
      mark: available ? 'open' : 'blocked',
      order: null,
      visits: 0,
      step: branch.step,
      ...(available
        ? {}
        : {
            block: {
              field: `${element.id}.requires`,
              gate: null,
              failed: null,
              values: [],
              explained: false,
            },
          }),
    })
  }

  return {
    origin: 'scenario',
    name,
    build: null,
    sceneId: scenario.scene,
    nodes,
    route,
    // A tape has an end, not a cursor: the run that made it is over.
    currentId: null,
    limits: [...SCENARIO_LIMITS, ARC_LIMIT],
  }
}

/* ── the arc, which gets one badge and is told why ────────────────────────── */

/**
 * Which scenes have been played, and nothing else.
 *
 * The arc's nodes are scenes, and a scene is exactly as much as Preview can
 * honestly say about one: a run covers a single scene, so there is no route
 * *between* scenes to draw. Reusing `RouteOverlay` rather than inventing a
 * second shape means the arc's badge is drawn by the same node component and
 * the same legend as the scene's, so neither can drift from the other.
 */
export function arcRoute(playedSceneIds: readonly string[]): RouteOverlay {
  const nodes = new Map<string, RouteNode>()
  for (const id of playedSceneIds) {
    nodes.set(id, { id, mark: 'played', order: null, visits: 1, step: 0 })
  }
  return {
    origin: 'arc',
    build: null,
    sceneId: '',
    nodes,
    route: [],
    currentId: null,
    limits: [ARC_LIMIT],
  }
}

/* ── pinning the overlay to the build it describes ────────────────────────── */

/**
 * Why this overlay may no longer describe what is on the canvas.
 *
 * Preview compiles *saved* source, so an overlay is pinned to a build the
 * moment it is made and the scene under it can move afterwards. Two things are
 * checked, and both are facts rather than heuristics:
 *
 * - **Unsaved edits.** The canvas is drawing a draft the build never saw.
 * - **Missing boxes.** A node the route names is not in the scene any more, so
 *   part of the route cannot be drawn at all.
 *
 * The overlay is not withdrawn for either. A route over a scene that has since
 * been edited is still the route that ran, and hiding it would answer a
 * designer's question with nothing; saying what changed lets them decide.
 */
export function routeDrift(
  overlay: RouteOverlay,
  scene: FlowScene,
  edited: boolean,
): readonly string[] {
  const said: string[] = []
  if (edited) {
    said.push(
      'This scene has unsaved edits. The overlay is pinned to the build Preview compiled from saved source, so it does not describe them.',
    )
  }
  const ids = new Set(scene.elements.map((element) => element.id))
  const gone = [...overlay.nodes.keys()].filter((id) => !ids.has(id)).length
  if (gone > 0) {
    said.push(
      `${gone} element${gone === 1 ? '' : 's'} on this route ${
        gone === 1 ? 'is' : 'are'
      } no longer in the scene, so ${gone === 1 ? 'it is' : 'they are'} not drawn.`,
    )
  }
  return said
}

/* ── the cursor shared between the two panes ──────────────────────────────── */

/**
 * Which step and which node the two panes are pointing at.
 *
 * A latched request with a rising sequence number, exactly as `narrativeReveal`
 * is, and for the same reason: Flow and Preview are separate tabs and only one
 * of them is mounted, so "scroll the transcript to this step" cannot be a call
 * — the transcript may not exist yet. It is a request the other pane honours
 * when it opens, and the sequence number is what makes asking twice for the
 * same step a second request rather than a value that did not change.
 *
 * A cursor, not a source of truth: nothing here is the route. Losing all of it
 * loses a scroll position.
 */
export interface RouteFocus {
  /** The recorded step the Preview list should scroll to. */
  step: { key: string; step: number; seq: number } | null
  /** The canvas node the Flow canvas should centre on. */
  node: { key: string; id: string; seq: number } | null
  showStep: (key: string, step: number) => void
  showNode: (key: string, id: string) => void
}

let focusSeq = 0

export const usePreviewRouteFocus = create<RouteFocus>((set) => ({
  step: null,
  node: null,
  showStep: (key, step) => set({ step: { key, step, seq: ++focusSeq } }),
  showNode: (key, id) => set({ node: { key, id, seq: ++focusSeq } }),
}))

/* ── the overlay, as the two canvases read it ─────────────────────────────── */

export interface PreviewOverlayValue {
  overlay: RouteOverlay
  /** Why the overlay may not describe what is drawn. See `routeDrift`. */
  drift: readonly string[]
  /** The Preview session key both panes address the focus cursor with. */
  key: string
}

export const PreviewOverlayContext = createContext<PreviewOverlayValue | null>(null)

/**
 * The overlay for the level below, or null when no run has been played.
 *
 * Null is the ordinary case — a designer authoring a scene has no Preview
 * running — and every consumer draws nothing for it rather than an empty frame
 * announcing that there is no route.
 */
export function usePreviewOverlay(): PreviewOverlayValue | null {
  return useContext(PreviewOverlayContext)
}

/* ── building the value ───────────────────────────────────────────────────── */

/**
 * The route for one scene: the saved scenario opened over it, or the live run.
 *
 * An opened scenario wins while it is open, and says so. Two overlays at once
 * would be two answers to "what happened here", and the honest resolution is
 * that a designer inspecting a saved run asked for that run.
 */
export function usePreviewRoute(
  projectKey: string,
  sceneId: string,
  scene: FlowScene,
  edited: boolean,
): PreviewOverlayValue | null {
  const key = `${projectKey}:${sceneId}`
  const session = usePreviewSessions((state) => state.sessions[key])
  const opened = usePreviewSessions((state) => state.opened[key])
  return useMemo(() => {
    const overlay = opened
      ? routeFromScenario(opened.name, opened.scenario, scene)
      : session
        ? routeFromPreview(sceneId, session.trace, session.frame)
        : null
    if (!overlay) return null
    return { overlay, drift: routeDrift(overlay, scene, edited), key }
  }, [opened, session, scene, sceneId, edited, key])
}

/**
 * Which scenes in this project have been played, for the arc's one badge.
 *
 * Deliberately the whole of what the arc gets. Preview runs a single scene, so
 * there is no cross-scene route to draw, and a canvas that drew arrows between
 * scenes from single-scene runs would be inventing the connections it claimed
 * to have observed.
 */
export function usePlayedScenes(projectKey: string): PreviewOverlayValue | null {
  const sessions = usePreviewSessions((state) => state.sessions)
  return useMemo(() => {
    const prefix = `${projectKey}:`
    const played = Object.keys(sessions)
      .filter((key) => key.startsWith(prefix))
      .map((key) => key.slice(prefix.length))
    if (played.length === 0) return null
    return { overlay: arcRoute(played), drift: [], key: prefix }
  }, [sessions, projectKey])
}

/* ── the shared cursor, honoured on each side ─────────────────────────────── */

/**
 * Point the Preview transcript at the step a node belongs to.
 *
 * Called on every canvas click and a no-op on an unmarked node, so selecting a
 * beat while authoring costs nothing. It deliberately does *not* change tab:
 * a click on the canvas is a canvas gesture, and throwing a writer into another
 * view for it would make the Flow tab unusable while a preview exists. The cap
 * on the node is the control that goes there, and this is what makes the
 * transcript already be at the right row when it does.
 */
export function useRouteTranscriptCursor(): (nodeId: string) => void {
  const value = usePreviewOverlay()
  return (nodeId) => {
    const node = value?.overlay.nodes.get(nodeId)
    if (value && node) usePreviewRouteFocus.getState().showStep(value.key, node.step)
  }
}

/**
 * Centre the canvas on a node the Preview transcript asked for.
 *
 * Latched rather than called, because Flow and Preview are separate tabs and
 * only the open one is mounted: "centre on this node" has to survive the trip
 * across a tab switch, which a function call cannot. `honoured` is what stops
 * the request being replayed every time the canvas re-renders — and the rising
 * sequence number is what makes asking twice for the same node a second
 * request rather than a value that did not change.
 */
export function useRouteCentring(center: (id: string) => void) {
  const value = usePreviewOverlay()
  const request = usePreviewRouteFocus((state) => state.node)
  const honoured = useRef(0)
  const key = value?.key
  useEffect(() => {
    if (!request || request.seq === honoured.current || request.key !== key) return
    honoured.current = request.seq
    center(request.id)
  }, [request, key, center])
}
