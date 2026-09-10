import type { NarrativeStatus } from '../narrativeModel'

/**
 * The shapes the Flow canvas draws, and the only shapes it draws.
 *
 * This is a **view model**, deliberately not the narrative source model. The
 * Rust `wobu-narrative` crate owns what a scene *is*; this file owns what a
 * canvas needs in order to put a box on screen and let a writer wire it to
 * another box. Keeping the two apart buys three things:
 *
 *   1. The canvas can be built, rendered and tested before a single Tauri
 *      command exists — which is the situation it was built in.
 *   2. A change to the source model is a change to one adapter function, not a
 *      change to seven React components.
 *   3. The canvas cannot accidentally depend on something the source model
 *      happens to expose but the export contract does not promise.
 *
 * Two rules this file keeps and the source model must never be asked to keep:
 *
 * - **No coordinates.** A node's position is presentation metadata, stored and
 *   loaded separately (#185). Nothing below has an `x`. `FlowPositions` is the
 *   seam, and it is a separate value passed alongside a scene, never inside it.
 * - **No text.** A beat has a *count* of lines and variants, never the lines.
 *   See `FlowBeat`.
 */

/**
 * The six node kinds from the spike, plus the two the *arc* level needs.
 *
 * Groups are containers rather than a kind of their own — a group is not
 * something you can connect a port to — so they are `FlowGroup` below, not a
 * member here.
 *
 * There is exactly **one kind of edge**. Conditions and effects live inside
 * elements (`FlowPort.requires`, `FlowOutcome.effects`), so a writer wires
 * control flow and never a data graph. Adding an edge kind here is how this
 * canvas would turn into a visual programming language, which is the thing
 * #151 says it must not be.
 *
 * `scene` and `missing` are the arc level's vocabulary (#187). They live here
 * rather than in a second parallel model because Flow is *one* editor drawn at
 * two zoom levels: the same graph builder, layout, keyboard layer, node shell
 * and outline serve both, and a second set of types would mean a second set of
 * all of those, drifting apart from the first commit onwards.
 */
export type FlowKind =
  | 'beat'
  | 'choice'
  | 'condition'
  | 'outcome'
  | 'end'
  | 'sceneLink'
  | 'scene'
  | 'questStage'
  | 'missing'

/** The label each kind wears, for its node face and its accessible name. */
export const FLOW_KIND_LABEL: Record<FlowKind, string> = {
  beat: 'Beat',
  choice: 'Choice',
  condition: 'Condition',
  outcome: 'Outcome',
  end: 'End',
  sceneLink: 'Scene link',
  scene: 'Scene',
  questStage: 'Quest stage',
  missing: 'Unresolved destination',
}

/**
 * A glyph per kind, because shape and colour alone are not a status or a type.
 * Names are ids in `IconSprite`; a missing one renders nothing rather than
 * throwing, but every name here exists today.
 */
export const FLOW_KIND_ICON: Record<FlowKind, string> = {
  beat: 'character',
  choice: 'layers',
  condition: 'chev',
  outcome: 'spark',
  end: 'x',
  sceneLink: 'link',
  scene: 'place',
  questStage: 'folder',
  missing: 'x',
}

/**
 * One way out of an element.
 *
 * The port *is* the edge. A destination is a field on the source element rather
 * than a row in an edge table, which is what makes "disconnect leaves an
 * explicit unresolved-destination diagnostic" fall out for free: an edge that
 * is deleted from an edge list simply stops existing, whereas a port whose `to`
 * is `null` is a visible hole with a name, and `sceneDiagnostics` can point at
 * it.
 */
export interface FlowPort {
  /** Stable inside its element: `then`, `true`, `false`, or an option id. */
  id: string
  label: string
  /**
   * What gates this port, in words — `has_logbook`. Drawn on the port row
   * itself rather than floating on the wire, so the requirement stays beside
   * the thing it gates however the graph is laid out.
   */
  requires?: string | null
  /** The element this leads to. `null` is an unresolved destination. */
  to: string | null
  /**
   * True when this port's destination is *structural* rather than authored.
   *
   * A beat leads to its own choices and outcomes because they belong to it, not
   * because somebody drew a wire — so that wire cannot be moved, and offering
   * it as a connection source would be offering a gesture that appears to work
   * and changes nothing. The handle is still rendered, so the edge is drawn and
   * the keyboard can travel along it; it simply refuses to start a connection.
   */
  fixed?: boolean
  /**
   * The element one level *down* that authored this destination — the outcome
   * or choice a writer has to open to change it.
   *
   * Only the arc level fills this in, and only so a dangling-destination
   * diagnostic can offer to take somebody to the responsible field rather than
   * to the scene it happens to sit in. Unset everywhere else, because inside a
   * scene the port already is the field.
   */
  via?: string | null
  routeId?: string
  choice?: boolean
}

/** One typed effect on an outcome: `support` `+` `10`. */
export interface FlowEffect {
  variable: string
  operator: string
  value: string
}

interface FlowElementBase {
  /** The narrative id. The same id appears in Script, Source and diagnostics. */
  id: string
  title: string
  /** The container this sits in, if any. */
  groupId?: string | null
  /**
   * The beat this element belongs to, for the shared selection.
   *
   * A choice or an outcome is not a beat, but selecting one still has to open
   * *something* in Script, and the useful something is the beat it hangs off.
   * A beat's own `beatId` is its own id. Elements with no owning beat — an end
   * point, a scene link — leave it null and select the scene alone.
   */
  beatId?: string | null
  /**
   * A single-valued display summary and fallback when work counts are absent.
   *
   * Lossy by construction, which is why `counts` exists beside it and why the
   * badges and work filters read `counts` instead. Nothing should present this as "the state of
   * this element": the three lifecycle dimensions are independent (see
   * `lifecycle.rs`) and a beat can be locked *and* out of date at once.
   */
  status?: NarrativeStatus
  /**
   * The outstanding work on this element, one count per dimension.
   *
   * Absent on an element whose text nothing has read — the demonstration
   * fixture, and any element with no dialogue under it — which is a different
   * claim from every count being zero.
   */
  counts?: FlowWorkCounts
  /**
   * Problems the backend attributed to this element, joined by stable id.
   *
   * Derived and never stored: they arrive from `narrative_diagnostics`, are
   * attached by id in `badges.ts`, and are recomputed whenever the scene is.
   */
  diagnostics?: readonly FlowElementDiagnostic[]
  /**
   * True when this box is a *picture of a field* on another element rather than
   * an element of its own — an ending, or a way out of the scene. It has no
   * identity in the source document, so it cannot be deleted or re-pointed
   * here, and it has no layout key to store a coordinate under.
   */
  derived?: boolean
  /** Ways out, in authored order. Order is meaningful: elk is told to keep it. */
  out: FlowPort[]
}

/**
 * One unit of played content — and **one node, always**.
 *
 * `lines` and `variants` are counts, and that is the whole of what the canvas
 * knows about a beat's text. This is the rule the Flow view lives or dies by:
 * generation multiplies variants, and if a variant were a node then one Build
 * would turn a 40-box scene into a 300-box scene and spend the entire node
 * budget on prose the writer reads in Script anyway. It is also why the node
 * has a fixed size — a box that grew with its line count would make *layout* a
 * function of generation output, so re-running Build would move every box on
 * the canvas.
 *
 * If you are ever tempted to add `lines: FlowLine[]` here, that is the bug this
 * type exists to prevent.
 */
export interface FlowBeat extends FlowElementBase {
  kind: 'beat'
  participants: string[]
  lines: number
  variants: number
}

/**
 * One option offered to the player.
 *
 * It carries effects for the same reason an outcome does: the source model's
 * `Choice` has them, and a canvas that drew the consequences of an automatic
 * transition but not of a player's decision would be hiding the half a writer
 * cares about most.
 */
export interface FlowChoice extends FlowElementBase {
  kind: 'choice'
  effects: FlowEffect[]
}

/** A branch the player never sees. Exactly two ports: `true` and `false`. */
export interface FlowCondition extends FlowElementBase {
  kind: 'condition'
  /** The test, in words: `trust >= 40`. */
  test: string
}

/** Typed effects applied to state, then one way on. */
export interface FlowOutcome extends FlowElementBase {
  kind: 'outcome'
  effects: FlowEffect[]
}

/** Terminal. No ports out, on purpose. */
export interface FlowEnd extends FlowElementBase {
  kind: 'end'
}

/** Leaves this scene. In the scene view it is terminal; the arc view joins up. */
export interface FlowSceneLink extends FlowElementBase {
  kind: 'sceneLink'
  targetSceneId: string | null
}

/**
 * Outstanding work, one count per lifecycle dimension.
 *
 * Four counts and not one status, because `lifecycle.rs` keeps four independent
 * facts and any summary of them loses one. A locked line whose context moved is
 * counted in `locked` *and* in `outOfDate`, which is the case US-06 exists for.
 */
export interface FlowWorkCounts {
  needsText: number
  needsReview: number
  outOfDate: number
  locked: number
}

/** What the arc level knows about a scene's outstanding work, as counts. */
export interface FlowSceneCounts extends FlowWorkCounts {
  beats: number
}

/**
 * One problem, already joined to the element responsible for it.
 *
 * `code` and the ids come from the backend unchanged; `severity` and `category`
 * are this side's reading of the code, and live in `badges.ts` beside the table
 * that assigns them.
 */
export interface FlowElementDiagnostic {
  /** Stable across renders, so a badge list can be keyed and compared. */
  id: string
  code: string
  message: string
  severity: 'error' | 'warning'
  category: FlowDiagnosticCategory
  /** The port this belongs to, when a destination is what is wrong. */
  portId?: string | null
  /** The dialogue slot or variant named, for a badge that has to say which. */
  slotId?: string | null
  variantId?: string | null
}

/**
 * How badges and their filter chips are grouped.
 *
 * `other` is not a spare slot: it is where a code this build has never heard of
 * lands. The backend's `DiagnosticCode` is deliberately open, so a newer Wobu
 * can report a problem this one cannot classify — and the honest answer is to
 * show it under a name that admits as much rather than to file it beside
 * something it may have nothing to do with.
 */
export type FlowDiagnosticCategory =
  'destination' | 'reference' | 'text' | 'identity' | 'type' | 'other'

/**
 * One whole scene, seen from outside — the arc level's only authored node.
 *
 * The same rule that makes a beat one node makes a scene one node: its `out`
 * ports are its **authored** exits and nothing else, and it carries counts
 * rather than content. Twelve lines and seven variants were one box a level
 * down; forty beats are one box here.
 *
 * `questId` is the seam to #155. It is a plain id with no quest model behind it
 * yet, so a caller that cannot say which quest a scene belongs to leaves it
 * null rather than guessing — see `flow/arc/model.ts`.
 */
export interface FlowSceneNode extends FlowElementBase {
  kind: 'scene'
  participants: string[]
  participantLabels?: Record<string, string>
  counts: FlowSceneCounts
  questId?: string | null
  questIds?: string[]
}

/**
 * A destination that does not resolve, drawn as a box of its own.
 *
 * Never authored: nothing creates one, and it is not offered on any toolbar.
 * `buildGraph({ dangling: true })` materialises one per unresolved port so the
 * wire ends somewhere visible and named, instead of simply not being drawn.
 * Inside a scene the unset port is enough — the box is right there with a hole
 * in it. Across an arc it is not: a quest whose last scene has no way in looks
 * exactly like a quest that ends there, and the difference is the whole point
 * of the view.
 */
export interface FlowMissing extends FlowElementBase {
  kind: 'missing'
  /** The id that was authored and could not be found. Null when none was. */
  targetId: string | null
}

export interface FlowQuestStage extends FlowElementBase {
  kind: 'questStage'
  questId: string
  stage: string
  /**
   * The player-facing objective for this stage (#207), or null when none is
   * authored. Shown on the node so a writer can see what the quest log will say
   * while the player is here, and see at a glance which stages say nothing.
   */
  objective: string | null
}

export type FlowElement =
  | FlowQuestStage
  | FlowBeat
  | FlowChoice
  | FlowCondition
  | FlowOutcome
  | FlowEnd
  | FlowSceneLink
  | FlowSceneNode
  | FlowMissing

/** A quest or sub-sequence container. Collapsible; that is most of its job. */
export interface FlowGroup {
  id: string
  name: string
}

/** One scene, as the canvas sees it. */
export interface FlowScene {
  id: string
  name: string
  groups: FlowGroup[]
  /** Authored order. Layout is told to respect it, so this is not cosmetic. */
  elements: FlowElement[]
  /** Where the scene starts, drawn with an entry cap. */
  entryId: string | null
}

/**
 * One level of the nested canvas.
 *
 * Structurally a `FlowScene`, and deliberately the same type rather than a
 * near-copy: an arc is a set of nodes with authored ways out, grouped into
 * containers, with one place it starts — which is the whole of what the graph
 * builder, the layout and the outline ever ask of a scene. The alias exists so
 * arc code can say what it means without either level having to pretend to be
 * the other.
 */
export type FlowLevel = FlowScene

/**
 * Node coordinates — the seam to #185, and the reason nothing above has an `x`.
 *
 * The canvas reads this and writes it back through a callback. It does not know
 * where it came from and never persists it: a layout-only change must produce
 * no source, revision, freshness or compiled-output difference, and the way to
 * guarantee that is for positions to be physically unable to reach `FlowScene`.
 */
export type FlowPositions = Readonly<Record<string, { x: number; y: number }>>

/** A problem with the scene, addressed at a field so the UI can jump to it. */
export interface FlowDiagnostic {
  /** Stable, so a list of these can be keyed and compared across edits. */
  id: string
  elementId: string
  /** `element.port` — what a "go to the field" link would open. */
  field: string
  message: string
  severity: 'error' | 'warning'
}

/** How many outs each kind is allowed. `null` means "as many as authored". */
const OUT_ARITY: Record<FlowKind, number | null> = {
  // As many as authored, not one. The fixture-era assumption was that a beat
  // has a single `then`; a real `Beat` states one way out per choice and one
  // per outcome, and warning about every branching beat in a project would
  // train a writer to ignore the list.
  beat: null,
  choice: null,
  condition: 2,
  outcome: 1,
  end: 0,
  sceneLink: 0,
  // A scene has as many exits as somebody authored, and none is also fine: a
  // quest's last scene is a real thing, not a scene missing an exit.
  scene: null,
  questStage: null,
  missing: 0,
}

/**
 * Everything wrong with the scene, recomputed rather than stored.
 *
 * Derived, because a stored diagnostic list is a second copy of the truth that
 * goes stale exactly when it matters — the moment somebody disconnects a port.
 * The scene is small (the canvas refuses to draw a big one; see `graph.ts`), so
 * recomputing costs nothing.
 */
export function sceneDiagnostics(scene: FlowScene): FlowDiagnostic[] {
  const ids = new Set(scene.elements.map((element) => element.id))
  const found: FlowDiagnostic[] = []

  for (const element of scene.elements) {
    for (const port of element.out) {
      if (port.to === null) {
        found.push({
          id: `${element.id}:${port.id}:unresolved`,
          elementId: element.id,
          field: `${element.id}.${port.id}`,
          message: `${element.title} — “${port.label}” has no destination.`,
          severity: 'error',
        })
      } else if (!ids.has(port.to)) {
        found.push({
          id: `${element.id}:${port.id}:missing`,
          elementId: element.id,
          field: `${element.id}.${port.id}`,
          message: `${element.title} — “${port.label}” leads to ${port.to}, which is not in this scene.`,
          severity: 'error',
        })
      }
    }

    const arity = OUT_ARITY[element.kind]
    if (arity !== null && element.out.length !== arity) {
      found.push({
        id: `${element.id}:arity`,
        elementId: element.id,
        field: element.id,
        message: `${FLOW_KIND_LABEL[element.kind]} “${element.title}” must have ${arity} way${
          arity === 1 ? '' : 's'
        } out, and has ${element.out.length}.`,
        severity: 'warning',
      })
    }
  }

  return found
}

/** The ports of `element` that lead nowhere. Used by the keyboard connector. */
export function unresolvedPorts(element: FlowElement): FlowPort[] {
  return element.out.filter((port) => port.to === null)
}

/**
 * The next free id for a kind, derived from the scene rather than from a
 * counter or a random source.
 *
 * Deterministic on purpose: a test that adds a beat can name the id it expects,
 * and two writers who add a beat to the same scene collide visibly rather than
 * both producing a uuid nobody can talk about. Real ids are the Rust side's to
 * mint; this is the placeholder that keeps the canvas honest until then.
 */
export function nextElementId(scene: FlowScene, kind: FlowKind): string {
  const prefix = kind.toLocaleLowerCase()
  let highest = 0
  for (const element of scene.elements) {
    const match = new RegExp(`^${prefix}\\.(\\d+)$`).exec(element.id)
    if (match) highest = Math.max(highest, Number(match[1]))
  }
  return `${prefix}.${highest + 1}`
}

/** The ports a freshly created element of this kind starts with. */
function initialPorts(kind: FlowKind): FlowPort[] {
  switch (kind) {
    case 'beat':
    case 'outcome':
      return [{ id: 'then', label: 'Then', to: null }]
    case 'choice':
      return [
        { id: 'option.1', label: 'First option', to: null },
        { id: 'option.2', label: 'Second option', to: null },
      ]
    case 'condition':
      return [
        { id: 'true', label: 'True', to: null },
        { id: 'false', label: 'False', to: null },
      ]
    case 'end':
    case 'sceneLink':
    case 'scene':
    case 'questStage':
    case 'missing':
      return []
  }
}

/** A new element of `kind`, with the ports its kind requires. */
export function newElement(scene: FlowScene, kind: FlowKind, groupId?: string | null): FlowElement {
  const id = nextElementId(scene, kind)
  const base = {
    id,
    title: `New ${FLOW_KIND_LABEL[kind].toLocaleLowerCase()}`,
    groupId: groupId ?? null,
    out: initialPorts(kind),
  }
  switch (kind) {
    case 'beat':
      return {
        ...base,
        kind,
        beatId: id,
        participants: [],
        lines: 0,
        variants: 0,
        status: 'needsText',
      }
    case 'choice':
      return { ...base, kind, effects: [] }
    case 'condition':
      return { ...base, kind, test: 'always' }
    case 'outcome':
      return { ...base, kind, effects: [] }
    case 'end':
      return { ...base, kind, title: 'End' }
    case 'sceneLink':
      return { ...base, kind, targetSceneId: null }
    case 'scene':
      return {
        ...base,
        kind,
        participants: [],
        counts: { beats: 0, needsText: 0, needsReview: 0, outOfDate: 0, locked: 0 },
        questId: null,
        status: 'needsText',
      }
    case 'questStage':
      return { ...base, kind, questId: '', stage: '', objective: null, derived: true }
    case 'missing':
      return { ...base, kind, targetId: null }
  }
}

/**
 * Add an element, wiring it in after `afterId` when that element has a spare
 * port.
 *
 * "After" means *through the first unresolved port*, never by displacing an
 * existing destination: adding a beat must not silently reroute a branch a
 * writer already drew. If the anchor is full, the new element lands
 * unconnected, which the diagnostics will say out loud.
 */
export function addElement(
  scene: FlowScene,
  kind: FlowKind,
  afterId: string | null,
): { scene: FlowScene; element: FlowElement } {
  const anchor = afterId === null ? null : (scene.elements.find((e) => e.id === afterId) ?? null)
  const element = newElement(scene, kind, anchor?.groupId ?? null)
  const spare = anchor === null ? null : (unresolvedPorts(anchor)[0] ?? null)

  const elements = scene.elements.map((existing) =>
    anchor !== null && spare !== null && existing.id === anchor.id
      ? {
          ...existing,
          out: existing.out.map((p) => (p.id === spare.id ? { ...p, to: element.id } : p)),
        }
      : existing,
  )

  return {
    scene: {
      ...scene,
      elements: [...elements, element],
      entryId: scene.entryId ?? element.id,
    },
    element,
  }
}

/**
 * Point a port at an element. `to === null` disconnects it.
 *
 * A port id the element does not have yet is **authored**, not ignored. That is
 * how a node born with no ports gains one: the arc level offers a scene a spare
 * exit handle, and connecting from it is the writer saying "this scene leads
 * there". Silently doing nothing instead would make dragging between two scenes
 * a gesture that appears to work and changes nothing.
 */
export function connectPort(
  scene: FlowLevel,
  from: { elementId: string; portId: string; label?: string },
  to: string | null,
): FlowLevel {
  return {
    ...scene,
    elements: scene.elements.map((element) => {
      if (element.id !== from.elementId) return element
      if (element.out.some((p) => p.id === from.portId)) {
        return {
          ...element,
          out: element.out.map((p) => (p.id === from.portId ? { ...p, to } : p)),
        }
      }
      return {
        ...element,
        out: [...element.out, { id: from.portId, label: from.label ?? from.portId, to }],
      }
    }),
  }
}

/**
 * Remove an element, and leave every port that pointed at it visibly empty.
 *
 * Not "and delete the edges too": an edge into a deleted beat was a decision
 * somebody made, and quietly dropping it loses the fact that a route now goes
 * nowhere. Setting `to` back to `null` turns each into an unresolved-destination
 * diagnostic, which is the thing the writer actually has to answer.
 */
export function removeElement(scene: FlowScene, id: string): FlowScene {
  const elements = scene.elements
    .filter((element) => element.id !== id)
    .map((element) =>
      element.out.some((p) => p.to === id)
        ? { ...element, out: element.out.map((p) => (p.to === id ? { ...p, to: null } : p)) }
        : element,
    )
  return {
    ...scene,
    elements,
    entryId: scene.entryId === id ? (elements[0]?.id ?? null) : scene.entryId,
  }
}
