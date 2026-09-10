import { groupIdentity } from './projectArcModel'
import type { Quest } from '../../../../lib/api/narrativeWorld'
import type { NarrativeTarget } from '../../../../store/ui'
import type {
  FlowDiagnostic,
  FlowElement,
  FlowGroup,
  FlowLevel,
  FlowPort,
  FlowScene,
  FlowSceneCounts,
  FlowSceneNode,
} from '../model'

/** Arc scene exits and quest stage transitions are projected from authored
 * source. Fixtures below can also project scene-link display nodes. Neither
 * path infers runtime edges or an entry scene from prose or memberships. */

/**
 * One quest, as the arc groups by it.
 *
 * `state` is a plain string rather than an enum because the stages a project's
 * quests move through are the project's own to declare: World quests carry a
 * `stages` list of names, and inventing a closed set here would be inventing a
 * vocabulary on the author's behalf.
 */
export interface FlowQuest {
  id: string
  name: string
  /** Null when the quest has no recorded state, which is different from having one. */
  state: string | null
  stages?: string[]
  transitions?: Quest['transitions']
}

export interface FlowArc {
  /** The scenes, their authored exits, and where the arc starts. */
  level: FlowLevel
  /** Null when the quest model is not available. See the note above. */
  quests: FlowQuest[] | null
}

/** How the arc is carved up. Presentation: none of it is written to the model. */
export type ArcGrouping = 'none' | 'arrangement' | 'quest' | 'questState'

export const ARC_GROUPINGS: { id: ArcGrouping; label: string }[] = [
  { id: 'none', label: 'No grouping' },
  { id: 'arrangement', label: 'By arrangement group' },
  { id: 'quest', label: 'By quest' },
  { id: 'questState', label: 'By quest state' },
]

/** What a quest with no recorded state is filed under, said rather than blank. */
const NO_STATE = 'State not recorded'

/**
 * One scene as the arc sees it: its exits, its people, and its outstanding work.
 *
 * Everything is *counted* from the scene's own elements. A scene node carries
 * no titles and no text, for the reason a beat node carries no lines: one
 * generation run must not be able to change what the arc looks like.
 */
export function sceneNode(scene: FlowScene, questId: string | null = null): FlowSceneNode {
  const counts: FlowSceneCounts = {
    beats: 0,
    needsText: 0,
    needsReview: 0,
    outOfDate: 0,
    locked: 0,
  }
  const participants = new Set<string>()
  const out: FlowPort[] = []

  for (const element of scene.elements) {
    if (element.kind === 'beat') {
      counts.beats += 1
      for (const name of element.participants) participants.add(name)
    }
    /*
     * Real lifecycle counts when the element has them, and the single-valued
     * status only when it does not.
     *
     * The order matters and it is not a preference. `counts` is four
     * independent numbers read off `Text.lifecycle`; `status` is the lossy
     * summary the filter chips use. Rolling the arc up from `status` would make
     * a scene with four out-of-date locked lines report one piece of work, so
     * an element that knows the real answer is asked for it.
     */
    if (element.counts) {
      counts.needsText += element.counts.needsText
      counts.needsReview += element.counts.needsReview
      counts.outOfDate += element.counts.outOfDate
      counts.locked += element.counts.locked
    } else {
      if (element.status === 'needsText') counts.needsText += 1
      if (element.status === 'needsReview') counts.needsReview += 1
      if (element.status === 'outOfDate') counts.outOfDate += 1
      if (element.status === 'locked') counts.locked += 1
    }

    /*
     * The only line in this file that makes an edge.
     *
     * An authored scene link, and its authored destination — which may be null,
     * and a null one is drawn as a dangling edge rather than quietly skipped.
     * `via` is the beat a writer has to open to change it, so the diagnostic
     * can offer to take them there instead of merely naming the scene.
     */
    if (element.kind === 'sceneLink') {
      out.push({
        id: element.id,
        label: element.title,
        to: element.targetSceneId,
        via: element.beatId ?? null,
      })
    }
  }

  return {
    kind: 'scene',
    id: scene.id,
    title: scene.name,
    questId,
    participants: [...participants].sort(),
    counts,
    out,
  }
}

/** The scene elements of an arc, typed. Everything in an arc level is one. */
export function scenesOf(arc: FlowArc): FlowSceneNode[] {
  return arc.level.elements.filter((element): element is FlowSceneNode => element.kind === 'scene')
}

/**
 * How the canvas should carve the arc up, or `undefined` for "not at all".
 *
 * Returns containers and a membership function rather than writing `groupId`
 * into the model, because switching a grouping control is not an edit to
 * anybody's story and must not produce a change to save.
 */
export function arcGrouping(
  arc: FlowArc,
  mode: ArcGrouping,
): { groups: FlowGroup[]; of: (element: FlowElement) => string | null } {
  /*
   * The level's own containers, which on a project arc are the arrangement
   * groups somebody drew in #185. They are the one grouping already written
   * down, so this hands them straight back rather than rebuilding them:
   * switching away and back has to land on the same boxes, closed the way the
   * sidecar says they were.
   */
  if (mode === 'arrangement')
    return { groups: arc.level.groups, of: (element) => element.groupId ?? null }

  const quests = arc.quests
  if (mode === 'none' || quests === null) return { groups: [], of: () => null }

  const byQuest = new Map(quests.map((quest) => [quest.id, quest]))
  const membership = (element: FlowElement) =>
    element.kind === 'scene'
      ? (element.questIds ?? (element.questId ? [element.questId] : []))
      : element.kind === 'questStage'
        ? [element.questId]
        : []
  const all = new Map<string, FlowGroup>()
  const owners = new Map<string, string>()
  for (const element of arc.level.elements) {
    const ids = membership(element)
      .filter((id) => byQuest.has(id))
      .sort()
    if (!ids.length) continue
    const labels =
      mode === 'questState'
        ? [...new Set(ids.map((id) => byQuest.get(id)!.state ?? NO_STATE))].sort()
        : ids.map((id) => byQuest.get(id)!.name)
    const key = JSON.stringify(mode === 'questState' ? labels : ids)
    const id = groupIdentity(`${mode}:${key}`)
    all.set(id, { id, name: `${ids.length > 1 ? 'Shared: ' : ''}${labels.join(' + ')}` })
    owners.set(element.id, id)
  }
  return { groups: [...all.values()], of: (element) => owners.get(element.id) ?? null }
}

/**
 * A diagnostic that knows where to send somebody, one level down.
 *
 * The extra field is the whole difference from a scene diagnostic. Inside a
 * scene, "go to the field" means selecting a node that is already on screen;
 * across an arc it means opening a different scene and landing on the beat that
 * authored the destination, which is a navigation the arc has to describe
 * because nothing else can work it out from the message.
 */
export interface ArcDiagnostic extends FlowDiagnostic {
  at: { sceneId: string; beatId: string | null; choiceId?: string; outcomeId?: string } | null
  questId?: string
}

/**
 * Everything wrong with the arc, recomputed rather than stored.
 *
 * Three findings, and each one is a thing #187 or US-02 names:
 *
 * - an exit with no destination chosen;
 * - an exit naming a scene this arc does not contain;
 * - a scene nothing leads to — "the scene with no way in", which is the one a
 *   canvas is worst at showing and the reason the outline exists.
 */
export function arcDiagnostics(arc: FlowArc): ArcDiagnostic[] {
  const scenes = scenesOf(arc)
  const known = new Set(scenes.map((scene) => scene.id))
  const found: ArcDiagnostic[] = []
  const reached = new Set<string>()

  for (const scene of scenes) {
    if (scene.counts.beats === 0)
      found.push({
        id: `${scene.id}:empty`,
        elementId: scene.id,
        field: scene.id,
        message: `${scene.title} has no beats. Open it to add its first beat.`,
        severity: 'error',
        at: { sceneId: scene.id, beatId: null },
      })
    for (const exit of scene.out) {
      const at = {
        sceneId: scene.id,
        beatId: exit.via ?? null,
        ...(exit.routeId
          ? exit.choice
            ? { choiceId: exit.routeId }
            : { outcomeId: exit.routeId }
          : {}),
      }
      if (exit.to === null) {
        found.push({
          id: `${scene.id}:${exit.id}:unresolved`,
          elementId: scene.id,
          field: `${scene.id}.${exit.id}`,
          message: `${scene.title} — “${exit.label}” has no destination.`,
          severity: 'error',
          at,
        })
      } else if (!known.has(exit.to)) {
        found.push({
          id: `${scene.id}:${exit.id}:missing`,
          elementId: scene.id,
          field: `${scene.id}.${exit.id}`,
          message: `${scene.title} — “${exit.label}” leads to ${exit.to}, which is not in this arc.`,
          severity: 'error',
          at,
        })
      } else {
        reached.add(exit.to)
      }
    }
  }

  for (const scene of scenes) {
    if (scene.id === arc.level.entryId || reached.has(scene.id)) continue
    found.push({
      id: `${scene.id}:orphan`,
      elementId: scene.id,
      field: scene.id,
      message: arc.level.entryId
        ? `Nothing leads to ${scene.title}, and it is not where the arc starts.`
        : `No authored scene exit leads to ${scene.title}. Story entry is not declared.`,
      severity: 'warning',
      at: { sceneId: scene.id, beatId: null },
    })
  }

  for (const element of arc.level.elements) {
    if (element.kind !== 'questStage') continue
    const quest = arc.quests?.find((q) => q.id === element.questId)
    if (!quest?.stages?.includes(element.stage))
      found.push({
        id: `${element.id}:missing`,
        elementId: element.id,
        field: element.id,
        message: `${element.title}: transition source stage is not declared.`,
        severity: 'error',
        at: null,
        questId: element.questId,
      })
    for (const port of element.out)
      if (!arc.level.elements.some((node) => node.id === port.to))
        found.push({
          id: `${element.id}:${port.id}:missing`,
          elementId: element.id,
          field: port.id,
          message: `${element.title}: ${port.label} targets an undeclared quest stage.`,
          severity: 'error',
          at: null,
          questId: element.questId,
        })
  }

  return found
}

/**
 * The one more way out a scene always offers.
 *
 * A scene with no exits has no source handle, so there is nothing to drag from
 * and nothing for the keyboard connector to pick — "set a scene exit by
 * connecting two nodes" would be impossible on the first scene of a new arc.
 * The port is not created until something connects to it (`connectPort` appends
 * it), so a scene that is simply finished never grows a hole it has to answer
 * for.
 */
export function arcSpare(element: FlowElement): FlowPort | null {
  if (element.kind !== 'scene') return null
  let highest = 0
  for (const port of element.out) {
    const match = /^exit\.(\d+)$/.exec(port.id)
    if (match) highest = Math.max(highest, Number(match[1]))
  }
  const next = Math.max(highest + 1, element.out.length + 1)
  return { id: `exit.${next}`, label: `Exit ${next}`, to: null }
}

/**
 * What selecting an arc node means on the shared narrative path.
 *
 * The element *is* a scene here, so it names itself. Passing the level's own id
 * — which is the arc's — would put an arc id where Script expects a scene's and
 * open nothing.
 */
export function arcTarget(_level: FlowLevel, element: FlowElement | null): NarrativeTarget {
  return { sceneId: element?.kind === 'scene' ? element.id : null, beatId: null }
}
