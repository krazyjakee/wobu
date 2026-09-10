import type { FlowElement, FlowScene } from './model'

/**
 * The Ashfall council hearing, in memory.
 *
 * Two jobs, and honest about both: it is the fixture every canvas test builds
 * on, and it is what an **empty project** draws so somebody can drive the
 * canvas before there is a story in the folder. It is drawn nowhere else. A
 * project with scenes draws its own — see `flow/source.ts` — because a fixture
 * shown over a writer's real work would be the most misleading thing this view
 * could do, and the banner above it says as much in the one case it appears.
 *
 * Nothing here is a document the backend would accept: the ids are `beat.1`
 * and friends rather than ULIDs, deliberately, so a fixture can never be saved
 * into somebody's project by accident.
 *
 * The shape is the one #186 names: three approaches — evidence, challenge,
 * threaten — reconverging on one verdict, with a nested Evidence review group
 * and a back edge from the verdict choice all the way up to the arrival beat.
 * It is written from the issue's description, not read out of
 * `examples/Ashfall.wobu/`, and a real story's shape will differ.
 */

const ELEMENTS: FlowElement[] = [
  {
    kind: 'beat',
    id: 'beat.1',
    title: 'Arrival at the hearing',
    beatId: 'beat.1',
    participants: ['Kael', 'Mira', 'Orren'],
    lines: 6,
    variants: 2,
    status: 'ready',
    out: [{ id: 'then', label: 'Then', to: 'choice.1' }],
  },
  {
    kind: 'choice',
    id: 'choice.1',
    title: 'How do you open?',
    beatId: 'beat.1',
    effects: [],
    out: [
      { id: 'option.1', label: 'Show the logbook', requires: 'has_logbook', to: 'beat.2' },
      { id: 'option.2', label: 'Challenge the captain', to: 'beat.3' },
      { id: 'option.3', label: 'Threaten the council', to: 'beat.4' },
    ],
  },

  // ── the evidence approach, inside its own group ──
  {
    kind: 'beat',
    id: 'beat.2',
    title: 'Present evidence',
    groupId: 'group.evidence',
    beatId: 'beat.2',
    participants: ['Kael', 'Mira'],
    // The spike's own example, and the whole point of the one-beat-one-node
    // rule: twelve lines and seven variants, still one box.
    lines: 12,
    variants: 7,
    status: 'needsReview',
    out: [{ id: 'then', label: 'Then', to: 'outcome.1' }],
  },
  {
    kind: 'outcome',
    id: 'outcome.1',
    title: 'The council reads the logbook',
    groupId: 'group.evidence',
    beatId: 'beat.2',
    effects: [
      { variable: 'support', operator: '+', value: '10' },
      { variable: 'trust', operator: '+', value: '5' },
    ],
    out: [{ id: 'then', label: 'Then', to: 'condition.1' }],
  },
  {
    kind: 'condition',
    id: 'condition.1',
    title: 'Is the council persuaded?',
    groupId: 'group.evidence',
    beatId: 'beat.2',
    test: 'trust >= 40',
    out: [
      { id: 'true', label: 'True', to: 'beat.5' },
      { id: 'false', label: 'False', to: 'beat.6' },
    ],
  },
  {
    kind: 'beat',
    id: 'beat.5',
    title: 'Mira speaks for you',
    groupId: 'group.evidence',
    beatId: 'beat.5',
    participants: ['Mira'],
    lines: 4,
    variants: 3,
    status: 'locked',
    out: [{ id: 'then', label: 'Then', to: 'beat.7' }],
  },
  {
    kind: 'beat',
    id: 'beat.6',
    title: 'Orren stays sceptical',
    groupId: 'group.evidence',
    beatId: 'beat.6',
    participants: ['Orren'],
    lines: 3,
    variants: 1,
    status: 'outOfDate',
    out: [{ id: 'then', label: 'Then', to: 'beat.7' }],
  },

  // ── the challenge approach ──
  {
    kind: 'beat',
    id: 'beat.3',
    title: 'Challenge the captain',
    beatId: 'beat.3',
    participants: ['Kael', 'Orren'],
    lines: 8,
    variants: 4,
    status: 'ready',
    out: [{ id: 'then', label: 'Then', to: 'outcome.2' }],
  },
  {
    kind: 'outcome',
    id: 'outcome.2',
    title: 'The captain loses composure',
    beatId: 'beat.3',
    effects: [
      { variable: 'support', operator: '+', value: '4' },
      { variable: 'captain_hostile', operator: '=', value: 'true' },
    ],
    out: [{ id: 'then', label: 'Then', to: 'beat.7' }],
  },

  // ── the threat approach ──
  {
    kind: 'beat',
    id: 'beat.4',
    title: 'Threaten the council',
    beatId: 'beat.4',
    participants: ['Kael'],
    lines: 0,
    variants: 0,
    status: 'needsText',
    out: [{ id: 'then', label: 'Then', to: 'outcome.3' }],
  },
  {
    kind: 'outcome',
    id: 'outcome.3',
    title: 'The room turns cold',
    beatId: 'beat.4',
    effects: [
      { variable: 'support', operator: '−', value: '8' },
      { variable: 'trust', operator: '−', value: '10' },
      { variable: 'orren_alarmed', operator: '=', value: 'true' },
      { variable: 'guards_called', operator: '=', value: 'true' },
    ],
    out: [{ id: 'then', label: 'Then', to: 'beat.7' }],
  },

  // ── where all three come back together ──
  {
    kind: 'beat',
    id: 'beat.7',
    title: 'The verdict',
    beatId: 'beat.7',
    participants: ['Mira', 'Orren'],
    lines: 9,
    variants: 5,
    status: 'ready',
    out: [{ id: 'then', label: 'Then', to: 'choice.2' }],
  },
  {
    kind: 'choice',
    id: 'choice.2',
    title: 'Do you accept the ruling?',
    beatId: 'beat.7',
    effects: [],
    out: [
      { id: 'option.1', label: 'Accept the ruling', to: 'outcome.4' },
      // The back edge: the writer can send the player round again.
      { id: 'option.2', label: 'Demand a recess', requires: 'support >= 30', to: 'beat.1' },
      { id: 'option.3', label: 'Walk out', to: 'end.1' },
    ],
  },
  {
    kind: 'outcome',
    id: 'outcome.4',
    title: 'The beacon inquiry is sanctioned',
    beatId: 'beat.7',
    effects: [{ variable: 'beacon_quest', operator: '=', value: 'sanctioned' }],
    out: [{ id: 'then', label: 'Then', to: 'sceneLink.1' }],
  },
  {
    kind: 'sceneLink',
    id: 'sceneLink.1',
    title: 'The long road out',
    targetSceneId: 'scene.road',
    out: [],
  },
  {
    kind: 'end',
    id: 'end.1',
    title: 'Hearing adjourned',
    out: [],
  },
]

/** A fresh copy every call: the canvas edits its scene, and tests share none. */
export function councilHearing(): FlowScene {
  return {
    id: 'scene.council',
    name: 'Council hearing',
    groups: [{ id: 'group.evidence', name: 'Evidence review' }],
    elements: ELEMENTS.map((element) => ({ ...element, out: element.out.map((p) => ({ ...p })) })),
    entryId: 'beat.1',
  }
}

/**
 * A scene of `count` beats in one chain, in `groups` groups.
 *
 * Only the budget tests use this. It is deliberately not a story: what is being
 * measured is how many boxes reach the `nodes` prop, and a realistic shape
 * would make that harder to read rather than easier.
 */
export function chainScene(count: number, groups = 1): FlowScene {
  const elements: FlowElement[] = []
  const perGroup = Math.ceil(count / groups)
  for (let index = 0; index < count; index++) {
    const next = index + 1 < count ? `beat.${index + 2}` : null
    elements.push({
      kind: 'beat',
      id: `beat.${index + 1}`,
      title: `Beat ${index + 1}`,
      groupId: `group.${Math.floor(index / perGroup) + 1}`,
      beatId: `beat.${index + 1}`,
      participants: [],
      lines: 1,
      variants: 0,
      out: [{ id: 'then', label: 'Then', to: next }],
    })
  }
  return {
    id: 'scene.chain',
    name: `${count} beats`,
    groups: Array.from({ length: groups }, (_, index) => ({
      id: `group.${index + 1}`,
      name: `Sequence ${index + 1}`,
    })),
    elements,
    entryId: 'beat.1',
  }
}
