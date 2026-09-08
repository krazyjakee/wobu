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

/**
 * The arc level: whole scenes, and the transitions somebody wrote between them.
 *
 * ── what an arc is made of, and what it is not ───────────────────────────────
 *
 * **Every edge here comes from an authored `sceneLink`.** `sceneNode` reads a
 * scene's elements, keeps the ones whose kind is `sceneLink`, and takes their
 * `targetSceneId`. That is the entire source of arc edges. Nothing reads a
 * beat's title, a line of dialogue, a participant list or a summary, and there
 * is no place in this file where a string is inspected for the name of another
 * scene. This is not a style preference: an arc drawn partly from prose would
 * show a designer a transition the runtime will never take, and the whole
 * purpose of the view is to be trustworthy about where the story can go.
 * `model.test.ts` holds it to that with a scene whose prose is nothing but
 * other scenes' names.
 *
 * ── where the quests come from ───────────────────────────────────────────────
 *
 * #187 asks for grouping by quest and by quest state, and both read the
 * project's own World document: `worldQuests` below turns its quests into
 * `FlowQuest`s and files each scene under the quest whose `scene_ids` lists it
 * — the same membership the Scene library's quest facet is built from. Nothing
 * is derived from a scene's name, its folder or the shape of its id, all of
 * which would look like a quest model and be a guess.
 *
 * `FlowArc.quests` is still `FlowQuest[] | null`, and **null is still not "no
 * quests"** — it is "this build cannot say", which is what a World read that
 * has not answered yet, or that failed, leaves behind. When it is null the
 * grouping control is refused with that reason rather than grouping by
 * something else and calling it a quest.
 *
 * ── why an arc is a `FlowLevel` ──────────────────────────────────────────────
 *
 * Because it is one. A set of nodes, each with authored ways out, grouped into
 * containers, with one place the story starts — which is exactly what the graph
 * builder, the layout, the keyboard layer and the outline ask of a scene. So
 * the arc is edited by `addElement`, `connectPort` and `removeElement`, the
 * same three functions the scene canvas and the scene *form* both call. That is
 * what makes #187's "creating a scene on the canvas produces the same model
 * change as the form path" true by construction instead of by two
 * implementations being kept in step.
 */

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
}

/**
 * The World document's quests, and which quest each scene is filed under.
 *
 * ── what `state` is, and what it is not ──────────────────────────────────────
 *
 * The stage the quest **starts** in. That is the only stage a project records:
 * a running quest state lives in a player's save file, which an authoring tool
 * never sees, so "group by quest state" can only ever mean "group by the stage
 * these quests begin in". The view says that in as many words, because a
 * grouping that quietly means something else is worse than no grouping.
 *
 * ── a scene two quests both claim ────────────────────────────────────────────
 *
 * World lets a scene appear in more than one quest's `scene_ids`, and a canvas
 * cannot draw one box inside two frames. So the first quest in document order
 * wins, deterministically, and every scene that was claimed twice is returned
 * in `shared` so the pane can say which ones and how many. Silently picking one
 * would leave a designer wondering why a scene is not in the quest they opened.
 *
 * `undefined` in means `quests: null` out — "this build cannot say" — which is
 * what an unanswered or failed World read has to produce.
 */
export interface ArcQuests {
  /** Null when the World document could not be read. Never an empty list for that. */
  quests: FlowQuest[] | null
  questOf: (sceneId: string) => string | null
  /** Scenes more than one quest lists, filed under the first. */
  shared: string[]
}

export function worldQuests(quests: readonly Quest[] | undefined | null): ArcQuests {
  if (!quests) return { quests: null, questOf: () => null, shared: [] }
  const owner = new Map<string, string>()
  const shared: string[] = []
  for (const quest of quests) {
    for (const sceneId of quest.scene_ids) {
      const held = owner.get(sceneId)
      if (held === undefined) owner.set(sceneId, quest.id)
      else if (held !== quest.id && !shared.includes(sceneId)) shared.push(sceneId)
    }
  }
  return {
    quests: quests.map((quest) => ({
      id: quest.id,
      name: quest.name,
      // Empty rather than absent is what a half-written quest leaves; "no
      // recorded state" is the honest reading of it, not a stage called "".
      state: quest.initial || null,
    })),
    questOf: (sceneId) => owner.get(sceneId) ?? null,
    shared,
  }
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

  const questOf = (element: FlowElement) =>
    element.kind === 'scene' ? (element.questId ?? null) : null

  if (mode === 'quest') {
    return {
      // Only the quests that actually hold a scene in this arc. An empty
      // container is a frame around nothing, and at 50 quests it is 50 of them.
      groups: quests
        .filter((quest) => arc.level.elements.some((e) => questOf(e) === quest.id))
        .map((quest) => ({ id: quest.id, name: quest.name })),
      of: questOf,
    }
  }

  const stateOf = new Map(quests.map((quest) => [quest.id, quest.state ?? NO_STATE]))
  const of = (element: FlowElement) => {
    const quest = questOf(element)
    const state = quest === null ? undefined : stateOf.get(quest)
    return state === undefined ? null : `state:${state}`
  }
  const states = new Set<string>()
  for (const element of arc.level.elements) {
    const group = of(element)
    if (group !== null) states.add(group)
  }
  return {
    groups: [...states].sort().map((id) => ({ id, name: id.slice('state:'.length) })),
    of,
  }
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
  at: { sceneId: string; beatId: string | null } | null
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
    for (const exit of scene.out) {
      const at = { sceneId: scene.id, beatId: exit.via ?? null }
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
      message: `Nothing leads to ${scene.title}, and it is not where the arc starts.`,
      severity: 'warning',
      at: { sceneId: scene.id, beatId: null },
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
  return { sceneId: element?.id ?? null, beatId: null }
}
