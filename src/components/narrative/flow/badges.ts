import type { NarrativeDiagnostic } from '../../../lib/api'
import { nodeId } from './source'
import type {
  FlowDiagnostic,
  FlowDiagnosticCategory,
  FlowElementDiagnostic,
  FlowLevel,
} from './model'

/**
 * Diagnostics on the canvas: joined by id, shown as words, and never stored.
 *
 * ── joined by id, never by text or position ──────────────────────────────────
 *
 * `wobu-narrative` already answers with the element responsible —
 * `Site::Destination(DestinationSite::Choice { beat, choice })` and its
 * siblings — and `commands/narrative.rs` flattens that onto plain
 * `beatId`/`choiceId`/`outcomeId`/`slotId`/`variantId` fields for exactly this
 * join. So the whole of the attachment below is "build the node id these ids
 * name". Nothing here reads a message, matches a title, or looks at where a box
 * happens to sit: a badge that landed by string matching would move the day
 * somebody reworded a diagnostic, and a badge that landed by position would be
 * wrong the moment two boxes overlapped.
 *
 * ── derived, and inert ───────────────────────────────────────────────────────
 *
 * Everything in this file is recomputed from the diagnostics the backend
 * returned and the level currently drawn. Nothing is written anywhere: toggling
 * a filter changes a value in the canvas's own zustand store, which is memory
 * for the life of the pane, and no path from here reaches a project file. That
 * is #189's "badges are derived views; they store nothing" made structural
 * rather than remembered.
 *
 * ── severity, and the one guarantee filters must keep ────────────────────────
 *
 * The backend does not rank problems, and it should not: severity is a
 * *product* judgement about what stops a release, and it belongs beside the
 * filter it governs. [`SEVERITY`] is that judgement, written out per code so a
 * reader can argue with it. An unrecognised code is an **error**, because this
 * build cannot prove it is safe to hide — and hiding it is the only mistake in
 * this file that a person would never notice.
 */

/**
 * Which problems block a release, and therefore which no filter may hide.
 *
 * Errors are things the story cannot do: a route to a beat that is not there, a
 * comparison that does not type, two elements claiming one id, a revision that
 * no longer describes the words a translation is keyed to. Warnings are work
 * outstanding: a slot with no wording yet is a *task*, and #151 is explicit
 * that missing prose is shown as one rather than treated as a broken scene. The
 * release gate that refuses to ship a missing required line is the CLI's
 * (#180), not this canvas's.
 */
const SEVERITY: Record<string, 'error' | 'warning'> = {
  dangling_beat: 'error',
  deleted_beat: 'error',
  unknown_scene: 'error',
  no_destination: 'error',
  no_beats: 'error',
  type_error: 'error',
  duplicate_id: 'error',
  revision_mismatch: 'error',
  not_a_participant: 'error',
  missing_text: 'warning',
}

const CATEGORY: Record<string, FlowDiagnosticCategory> = {
  dangling_beat: 'destination',
  deleted_beat: 'destination',
  unknown_scene: 'destination',
  no_destination: 'destination',
  no_beats: 'destination',
  type_error: 'type',
  duplicate_id: 'identity',
  revision_mismatch: 'identity',
  not_a_participant: 'reference',
  missing_text: 'text',
}

/**
 * The legend, and the filter chips, from one list.
 *
 * Each carries a word and a glyph. Shape and colour alone are never a status
 * here, for the same reason they are never one anywhere else in this workspace:
 * a reader who cannot tell the tints apart still has to be able to read the
 * canvas.
 */
export const FLOW_DIAGNOSTIC_CATEGORIES: {
  id: FlowDiagnosticCategory
  label: string
  /** The singular, for a badge that has to read "1 destination error". */
  noun: string
  icon: string
  /** What lands here, so the legend explains rather than merely names. */
  about: string
}[] = [
  {
    id: 'destination',
    label: 'Destinations',
    noun: 'destination',
    icon: 'x',
    about: 'A route that leads to a beat, a scene or an ending that is not there.',
  },
  {
    id: 'reference',
    label: 'References',
    noun: 'reference',
    icon: 'link',
    about: 'A speaker or a participant this scene does not contain.',
  },
  {
    id: 'type',
    label: 'Types',
    noun: 'type',
    icon: 'chev',
    about: 'A condition or an effect that does not type.',
  },
  {
    id: 'identity',
    label: 'Identities',
    noun: 'identity',
    icon: 'layers',
    about: 'A duplicated id, or a revision that no longer describes its words.',
  },
  {
    id: 'text',
    label: 'Text',
    noun: 'text',
    icon: 'spark',
    about: 'A slot that is still waiting for wording.',
  },
  {
    id: 'other',
    label: 'Unrecognised',
    noun: 'unrecognised',
    icon: 'clock',
    about:
      'A problem this build has no name for. Shown as an error, because it cannot be ruled out.',
  },
]

/**
 * The badges one element wears: one per severity and category, with a count.
 *
 * Grouped rather than one badge per finding, because a node is a fixed-size box
 * and four full sentences do not fit in one — and a box that grew to hold them
 * would make layout a function of how broken the scene is. The messages
 * themselves are on the badge's title, in the node's accessible name, and in
 * full in the outline list, which is the alternative #151 requires anyway.
 */
export function badgeRows(
  diagnostics: readonly FlowElementDiagnostic[] | undefined,
  filter: BadgeFilter,
): { id: string; severity: 'error' | 'warning'; label: string; icon: string; detail: string }[] {
  const groups = new Map<string, FlowElementDiagnostic[]>()
  for (const found of diagnostics ?? []) {
    if (!badgeShown(found, filter)) continue
    const key = `${found.severity}:${found.category}`
    const list = groups.get(key)
    if (list) list.push(found)
    else groups.set(key, [found])
  }
  return [...groups].map(([key, found]) => {
    const first = found[0]!
    const meta = FLOW_DIAGNOSTIC_CATEGORIES.find((one) => one.id === first.category)
    const word = first.severity === 'error' ? 'error' : 'warning'
    return {
      id: key,
      severity: first.severity,
      // A word and a count, never a tint on its own: `03-ui-layout.md` makes
      // that the rule for every status in this workspace.
      label: `${found.length} ${meta?.noun ?? first.category} ${word}${found.length === 1 ? '' : 's'}`,
      icon: meta?.icon ?? 'x',
      detail: found.map((one) => one.message).join(' · '),
    }
  })
}

export function severityOf(code: string): 'error' | 'warning' {
  return SEVERITY[code] ?? 'error'
}

export function categoryOf(code: string): FlowDiagnosticCategory {
  return CATEGORY[code] ?? 'other'
}

/** What a canvas is currently choosing to show. Ephemeral; see the file header. */
export interface BadgeFilter {
  /** Whether warnings are drawn at all. Errors ignore this. */
  warnings: boolean
  /** Which categories' *warnings* are drawn. Errors ignore this too. */
  categories: Record<FlowDiagnosticCategory, boolean>
}

export const ALL_BADGES: BadgeFilter = {
  warnings: true,
  categories: {
    destination: true,
    reference: true,
    text: true,
    identity: true,
    type: true,
    other: true,
  },
}

/**
 * Whether a filter shows this one.
 *
 * The single line that #187 and #189 both hang a guarantee on: an error is
 * shown whatever the filters say. Written as an early return rather than as a
 * clause in a boolean expression, so it cannot be lost in a later edit to the
 * rest of the condition.
 */
export function badgeShown(diagnostic: FlowElementDiagnostic, filter: BadgeFilter): boolean {
  if (diagnostic.severity === 'error') return true
  if (!filter.warnings) return false
  return filter.categories[diagnostic.category]
}

/**
 * The node a diagnostic belongs to, from its ids alone.
 *
 * `null` means "not a box on this canvas" — a problem with the scene itself,
 * its entry condition or its participant list. Those are shown above the canvas
 * rather than dropped, because a scene with no beats has no box to put a badge
 * on and is exactly the case a writer needs told about.
 */
export function nodeOf(diagnostic: NarrativeDiagnostic): string | null {
  switch (diagnostic.kind) {
    case 'choice':
      return diagnostic.choiceId ? nodeId.choice(diagnostic.choiceId) : null
    case 'outcome':
      return diagnostic.outcomeId ? nodeId.outcome(diagnostic.outcomeId) : null
    // A slot and a variant are inside a beat and have no box of their own — one
    // beat is one node however many lines are under it — so the badge lands on
    // the beat and names the slot. That is the same rule that keeps a beat one
    // node after a Build multiplies its variants.
    case 'beat':
    case 'intent':
    case 'dialogueSlot':
    case 'variant':
      return diagnostic.beatId ? nodeId.beat(diagnostic.beatId) : null
    case 'scene':
    case 'entry':
    case 'participant':
      return null
    default:
      // An unrecognised kind still has ids on it; a beat id is the most useful
      // thing to land on, and no box at all is better than a wrong box.
      return diagnostic.beatId ? nodeId.beat(diagnostic.beatId) : null
  }
}

/** A stable key for one finding, so a badge list can be keyed and compared. */
function keyOf(diagnostic: NarrativeDiagnostic, index: number): string {
  return [
    diagnostic.kind,
    diagnostic.code,
    diagnostic.beatId ?? '',
    diagnostic.choiceId ?? '',
    diagnostic.outcomeId ?? '',
    diagnostic.slotId ?? '',
    diagnostic.variantId ?? '',
    diagnostic.intentIndex ?? '',
    // Two identical findings on one element are possible — a beat with two
    // effects that fail the same way — so position within the answer breaks the
    // tie. It is only ever a tiebreak: everything before it is an identity.
    index,
  ].join('|')
}

export interface AttachedDiagnostics {
  /** The same level, with each element carrying the findings that name it. */
  level: FlowLevel
  /** Findings about the scene as a whole, which no box can carry. */
  sceneWide: FlowElementDiagnostic[]
  /** Findings whose element is not drawn — a beat inside a closed group, say. */
  unplaced: FlowElementDiagnostic[]
}

/**
 * Join the backend's answer onto the boxes on screen.
 *
 * Returns a new level rather than a side table, because the node components
 * already receive their element and re-render when it changes — and diagnostics
 * change exactly when the scene does. A side table would have to be threaded
 * through `buildGraph`, the node array and every memo on the way.
 */
export function attachDiagnostics(
  level: FlowLevel,
  diagnostics: readonly NarrativeDiagnostic[],
): AttachedDiagnostics {
  const byNode = new Map<string, FlowElementDiagnostic[]>()
  const sceneWide: FlowElementDiagnostic[] = []
  const unplaced: FlowElementDiagnostic[] = []
  const drawn = new Set(level.elements.map((element) => element.id))

  diagnostics.forEach((diagnostic, index) => {
    const found: FlowElementDiagnostic = {
      id: keyOf(diagnostic, index),
      code: diagnostic.code,
      message: diagnostic.message,
      severity: severityOf(diagnostic.code),
      category: categoryOf(diagnostic.code),
      // A destination problem is about the way *out*, so it marks the wire as
      // well as the box. Every element that states a destination states it
      // through one port called `then`.
      portId: diagnostic.destination ? 'then' : null,
      slotId: diagnostic.slotId ?? null,
      variantId: diagnostic.variantId ?? null,
    }
    const node = nodeOf(diagnostic)
    if (node === null) {
      sceneWide.push(found)
      return
    }
    if (!drawn.has(node)) {
      unplaced.push(found)
      return
    }
    const list = byNode.get(node)
    if (list) list.push(found)
    else byNode.set(node, [found])
  })

  return {
    level: {
      ...level,
      elements: level.elements.map((element) => {
        const found = byNode.get(element.id)
        return found ? { ...element, diagnostics: found } : element
      }),
    },
    sceneWide,
    unplaced,
  }
}

/**
 * The canvas's own diagnostic list, from the attached findings.
 *
 * `FlowCanvas` uses this for two things and no others: the count in its status
 * line, and `blockingIds`, which is what forbids the *node* filters from muting
 * a box a release is blocked on. Both want severity and an element id, which is
 * all this carries.
 */
export function flowDiagnostics(level: FlowLevel): FlowDiagnostic[] {
  const out: FlowDiagnostic[] = []
  for (const element of level.elements) {
    for (const found of element.diagnostics ?? []) {
      out.push({
        id: `${element.id}:${found.id}`,
        elementId: element.id,
        field: found.portId ? `${element.id}.${found.portId}` : element.id,
        message: found.message,
        severity: found.severity,
      })
    }
  }
  return out
}
