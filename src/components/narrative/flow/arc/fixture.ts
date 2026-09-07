import { councilHearing } from '../fixture'
import type { FlowElement, FlowLevel, FlowScene } from '../model'
import { sceneNode, type FlowArc, type FlowQuest } from './model'

/**
 * The beacon inquiry, in memory.
 *
 * What the arc tests build on, and what an **empty project** draws so somebody
 * can drive the view before there is a story in the folder. A project with
 * scenes draws its own arc from its own authored scene links.
 *
 * The quests, though, are a fixture for a second and separate reason: there is
 * no quest model at all (#155), so they would still be invented on the day the
 * scenes are real. The pane keeps the two claims apart and says both — a
 * fabricated quest model presented as if it had been read from the project
 * would be the single most misleading thing this view could do.
 *
 * The shape is chosen so each thing #187 asks for appears exactly once:
 *
 * - a resolved transition: the council hearing leads to the long road;
 * - an exit with **no destination chosen**: "Turn back" on the long road;
 * - an exit naming a scene **this arc does not contain**: the beacon chamber
 *   leads to `scene.harbour`;
 * - a scene with **no way in**: the drowned ruins, which is the finding US-02
 *   opens the view for.
 *
 * Every arc edge below is produced by `sceneNode` reading a `sceneLink`
 * element. None of these scenes' prose mentions another scene, and the arc
 * would look the same if it did — see `model.test.ts`.
 */

/** One beat with a status, so a scene's work counts come from somewhere real. */
function beat(
  id: string,
  title: string,
  people: string[],
  status: FlowElement['status'],
  to: string | null,
): FlowElement {
  return {
    kind: 'beat',
    id,
    title,
    beatId: id,
    participants: people,
    lines: status === 'needsText' ? 0 : 4,
    variants: status === 'needsText' ? 0 : 2,
    status,
    out: [{ id: 'then', label: 'Then', to }],
  }
}

/** A way out of the scene, and the beat a writer would open to change it. */
function link(id: string, title: string, via: string, to: string | null): FlowElement {
  return { kind: 'sceneLink', id, title, beatId: via, targetSceneId: to, out: [] }
}

function scene(id: string, name: string, elements: FlowElement[]): FlowScene {
  return { id, name, groups: [], elements, entryId: elements[0]?.id ?? null }
}

const QUESTS: FlowQuest[] = [
  { id: 'quest.beacon', name: 'The beacon inquiry', state: 'Investigating' },
  { id: 'quest.aftermath', name: 'Aftermath', state: 'Not started' },
]

/** The four scenes of the demonstration arc, each one a real scene to drill into. */
export function beaconScenes(): Record<string, FlowScene> {
  const road = scene('scene.road', 'The long road out', [
    beat('beat.1', 'Leaving the chamber', ['Kael', 'Mira'], 'ready', 'beat.2'),
    beat('beat.2', 'The road north', ['Kael'], 'needsReview', 'link.beacon'),
    link('link.beacon', 'On to the beacon', 'beat.2', 'scene.beacon'),
    // Authored, and deliberately unfinished: an exit somebody drew and has not
    // decided about. It is a dangling edge on the arc, not a missing one.
    link('link.back', 'Turn back', 'beat.2', null),
  ])

  const beacon = scene('scene.beacon', 'The beacon chamber', [
    beat('beat.1', 'The chamber wakes', ['Kael', 'Orren'], 'needsText', 'beat.2'),
    beat('beat.2', 'Orren reads the array', ['Orren'], 'outOfDate', 'link.harbour'),
    // Names a scene this arc does not hold. The runtime would fail here, so the
    // canvas has to show it rather than draw nothing.
    link('link.harbour', 'Down to the harbour', 'beat.2', 'scene.harbour'),
  ])

  const ruins = scene('scene.ruins', 'The drowned ruins', [
    beat('beat.1', 'What the tide left', ['Mira'], 'needsText', null),
  ])

  return {
    'scene.council': councilHearing(),
    'scene.road': road,
    'scene.beacon': beacon,
    'scene.ruins': ruins,
  }
}

/** The arc over those four scenes, with its demonstration quests attached. */
export function beaconArc(scenes: Record<string, FlowScene> = beaconScenes()): FlowArc {
  const quest: Record<string, string> = {
    'scene.council': 'quest.beacon',
    'scene.road': 'quest.beacon',
    'scene.beacon': 'quest.beacon',
    'scene.ruins': 'quest.aftermath',
  }
  const level: FlowLevel = {
    id: 'arc.beacon',
    name: 'The beacon inquiry',
    groups: [],
    elements: Object.values(scenes).map((one) => sceneNode(one, quest[one.id] ?? null)),
    entryId: 'scene.council',
  }
  return { level, quests: QUESTS }
}

/**
 * An arc of `count` scenes in `quests` quests, chained.
 *
 * Only the budget and layout measurements use this. It is deliberately not a
 * story: what is being measured is how many boxes reach the `nodes` prop and
 * how long elk takes on them, and a realistic shape would make both harder to
 * read rather than easier. The proportions are the spike's — 1,000 scenes in 50
 * quests is the fixture the 240-node collapsed figure came from.
 */
export function chainArc(count: number, quests = 50): FlowArc {
  const perQuest = Math.ceil(count / quests)
  const elements: FlowElement[] = []
  for (let index = 0; index < count; index++) {
    const next = index + 1 < count ? `scene.${index + 2}` : null
    elements.push({
      kind: 'scene',
      id: `scene.${index + 1}`,
      title: `Scene ${index + 1}`,
      questId: `quest.${Math.floor(index / perQuest) + 1}`,
      participants: [],
      counts: { beats: 3, needsText: 0, needsReview: 0, outOfDate: 0, locked: 0 },
      out: next === null ? [] : [{ id: 'link.1', label: 'Then', to: next }],
    })
  }
  return {
    level: {
      id: 'arc.chain',
      name: `${count} scenes`,
      groups: [],
      elements,
      entryId: 'scene.1',
    },
    quests: Array.from({ length: quests }, (_, index) => ({
      id: `quest.${index + 1}`,
      name: `Quest ${index + 1}`,
      state: index % 2 === 0 ? 'Investigating' : 'Not started',
    })),
  }
}
