import { describe, expect, it } from 'vitest'
import type { Quest } from '../../../../lib/api/narrativeWorld'
import { buildGraph } from '../graph'
import { addElement, connectPort, removeElement, type FlowElement, type FlowScene } from '../model'
import { beaconArc, beaconScenes } from './fixture'
import {
  arcDiagnostics,
  arcGrouping,
  arcSpare,
  arcTarget,
  sceneNode,
  scenesOf,
  worldQuests,
  type FlowArc,
} from './model'

/** A World quest, with only the fields the arc reads made interesting. */
function quest(id: string, name: string, initial: string, scenes: string[]): Quest {
  return {
    id,
    name,
    summary: '',
    stages: [initial],
    initial,
    transitions: [],
    scene_ids: scenes,
  }
}

/**
 * A scene that talks about other scenes constantly and links to none of them.
 *
 * Every string a generator or a writer could put in this scene names another
 * scene: the beat titles, the choice, the option label, the end. If anything
 * anywhere in the arc view read text to find a transition, this scene would
 * sprout edges — which is the failure the test below exists to catch, and the
 * reason it is worth a fixture of its own rather than an assertion on the demo.
 */
function talksAboutScenes(): FlowScene {
  const elements: FlowElement[] = [
    {
      kind: 'beat',
      id: 'beat.1',
      title: 'Kael argues they should go to scene.harbour instead',
      beatId: 'beat.1',
      participants: ['Kael'],
      lines: 9,
      variants: 4,
      status: 'needsReview',
      out: [{ id: 'then', label: 'Then', to: 'choice.1' }],
    },
    {
      kind: 'choice',
      id: 'choice.1',
      title: 'scene.beacon or scene.harbour?',
      beatId: 'beat.1',
      effects: [],
      out: [
        { id: 'option.1', label: 'Say scene.beacon', to: 'end.1' },
        { id: 'option.2', label: 'Say scene.harbour', to: 'end.1' },
      ],
    },
    { kind: 'end', id: 'end.1', title: 'scene.council', out: [] },
  ]
  return { id: 'scene.talk', name: 'All talk', groups: [], elements, entryId: 'beat.1' }
}

describe('what makes an edge, and what does not', () => {
  it('gives a scene no exits at all when nothing authored one', () => {
    // Four strings naming three scenes, and zero transitions. If this ever
    // returns a port, something has started reading prose.
    expect(sceneNode(talksAboutScenes()).out).toEqual([])
  })

  it('gives it exactly one the moment a scene link is authored', () => {
    const scene = talksAboutScenes()
    const authored: FlowScene = {
      ...scene,
      elements: [
        ...scene.elements,
        {
          kind: 'sceneLink',
          id: 'link.1',
          title: 'Down to the harbour',
          beatId: 'beat.1',
          targetSceneId: 'scene.harbour',
          out: [],
        },
      ],
    }
    expect(sceneNode(authored).out).toEqual([
      { id: 'link.1', label: 'Down to the harbour', to: 'scene.harbour', via: 'beat.1' },
    ])
  })

  it('draws no wire between two scenes that only mention each other', () => {
    // The same claim one level up, against what actually reaches React Flow:
    // two scenes full of each other's names, and an empty edge array.
    const gossip: FlowArc = {
      level: {
        id: 'arc.gossip',
        name: 'Gossip',
        groups: [],
        elements: [
          sceneNode({ ...talksAboutScenes(), id: 'scene.beacon', name: 'scene.harbour' }),
          sceneNode({ ...talksAboutScenes(), id: 'scene.harbour', name: 'scene.beacon' }),
        ],
        entryId: 'scene.beacon',
      },
      quests: null,
    }
    const graph = buildGraph(gossip.level, { dangling: true })
    expect(graph.nodes).toHaveLength(2)
    expect(graph.edges).toEqual([])
  })

  it('keeps an authored destination the writer has not chosen yet, as a hole with a name', () => {
    const road = sceneNode(beaconScenes()['scene.road']!)
    // Two authored exits: one resolved, one deliberately unfinished. The second
    // is a port with `to: null` rather than an absent port, which is what lets
    // the diagnostic name the field instead of the scene.
    expect(road.out.map((port) => [port.id, port.to])).toEqual([
      ['link.beacon', 'scene.beacon'],
      ['link.back', null],
    ])
  })
})

describe('what a scene node summarises', () => {
  it('counts beats, work and people from the scene rather than being told', () => {
    const beacon = sceneNode(beaconScenes()['scene.beacon']!)
    expect(beacon.counts).toEqual({
      beats: 2,
      needsText: 1,
      needsReview: 0,
      outOfDate: 1,
      locked: 0,
    })
    expect(beacon.participants).toEqual(['Kael', 'Orren'])
  })

  it('carries no titles and no text from inside the scene', () => {
    const council = sceneNode(beaconScenes()['scene.council']!)
    const json = JSON.stringify(council)
    // The council hearing's most distinctive beat, and its most distinctive
    // line of prose. Neither may cross the level boundary — a scene node that
    // grew with its contents would make one Build reflow the whole arc.
    expect(json).not.toMatch(/Present evidence/)
    expect(json).not.toMatch(/verdict/i)
    expect(council.counts.beats).toBe(7)
  })
})

describe('arc diagnostics', () => {
  it('names the field and the beat behind an exit with no destination', () => {
    const found = arcDiagnostics(beaconArc())
    const unresolved = found.find((d) => d.id === 'scene.road:link.back:unresolved')
    expect(unresolved?.severity).toBe('error')
    expect(unresolved?.field).toBe('scene.road.link.back')
    expect(unresolved?.message).toMatch(/“Turn back” has no destination/)
    // The link, not just the scene: this is what "links to the responsible
    // outcome" has to mean if following it is to land anywhere useful.
    expect(unresolved?.at).toEqual({ sceneId: 'scene.road', beatId: 'beat.2' })
  })

  it('reports a destination this arc does not contain', () => {
    const found = arcDiagnostics(beaconArc())
    const missing = found.find((d) => d.id === 'scene.beacon:link.harbour:missing')
    expect(missing?.severity).toBe('error')
    expect(missing?.message).toMatch(/leads to scene.harbour, which is not in this arc/)
  })

  it('spots the scene with no way in, which is what the view is opened for', () => {
    const found = arcDiagnostics(beaconArc())
    expect(found.filter((d) => d.severity === 'warning').map((d) => d.message)).toEqual([
      'Nothing leads to The drowned ruins, and it is not where the arc starts.',
    ])
  })

  it('says nothing about the scene the arc starts at', () => {
    // The entry has no way in either, and that is not a finding.
    const found = arcDiagnostics(beaconArc())
    expect(found.some((d) => d.elementId === 'scene.council')).toBe(false)
  })

  it('turns a removed scene into an unresolved destination rather than silence', () => {
    const arc = beaconArc()
    const without = { ...arc, level: removeElement(arc.level, 'scene.road') }
    const found = arcDiagnostics(without)
    expect(found.find((d) => d.field === 'scene.council.sceneLink.1')?.message).toMatch(
      /has no destination/,
    )
  })
})

describe('grouping, which is presentation and never an edit', () => {
  it('groups by quest', () => {
    const arc = beaconArc()
    const { groups, of } = arcGrouping(arc, 'quest')
    expect(groups.map((group) => group.name)).toEqual(['The beacon inquiry', 'Aftermath'])
    expect(of(scenesOf(arc)[0]!)).toBe('quest.beacon')
  })

  it('groups by quest state, folding the three beacon scenes into one box', () => {
    const arc = beaconArc()
    const { groups, of } = arcGrouping(arc, 'questState')
    expect(groups.map((group) => group.name)).toEqual(['Investigating', 'Not started'])
    expect(scenesOf(arc).map(of)).toEqual([
      'state:Investigating',
      'state:Investigating',
      'state:Investigating',
      'state:Not started',
    ])
  })

  it('refuses to group at all when the quest model is not available', () => {
    // Null is "this build cannot say", not "there are no quests", and the only
    // honest answer to it is no grouping — not one invented from scene ids.
    const arc: FlowArc = { ...beaconArc(), quests: null }
    expect(arcGrouping(arc, 'quest')).toMatchObject({ groups: [] })
    expect(arcGrouping(arc, 'quest').of(scenesOf(arc)[0]!)).toBeNull()
  })

  it('hands back the level’s own containers when asked for the arrangement', () => {
    // The one grouping that is already written down (#185). It is returned
    // untouched rather than rebuilt, so switching away and back lands on the
    // same boxes with the same ids — which is what the stored collapsed set
    // was recorded against.
    const arc = beaconArc()
    const groups = [{ id: 'group.1', name: 'Evidence' }]
    const level = {
      ...arc.level,
      groups,
      elements: arc.level.elements.map((element, index) =>
        index === 0 ? { ...element, groupId: 'group.1' } : element,
      ),
    }
    const grouping = arcGrouping({ ...arc, level }, 'arrangement')
    expect(grouping.groups).toBe(groups)
    expect(level.elements.map(grouping.of)).toEqual(['group.1', null, null, null])
  })

  it('leaves the model untouched whichever grouping is chosen', () => {
    const arc = beaconArc()
    const before = JSON.stringify(arc.level)
    for (const mode of ['none', 'arrangement', 'quest', 'questState'] as const)
      arcGrouping(arc, mode)
    expect(JSON.stringify(arc.level)).toBe(before)
  })
})

describe('the quests the arc groups by, read from World', () => {
  it('files each scene under the quest whose scene_ids name it', () => {
    const { quests, questOf, shared } = worldQuests([
      quest('quest.beacon', 'The beacon inquiry', 'investigating', ['scene.council', 'scene.road']),
      quest('quest.aftermath', 'Aftermath', 'not_started', ['scene.ruins']),
    ])
    expect(questOf('scene.road')).toBe('quest.beacon')
    expect(questOf('scene.ruins')).toBe('quest.aftermath')
    // A scene no quest lists belongs to none, which is not the same as
    // belonging to the first one.
    expect(questOf('scene.beacon')).toBeNull()
    expect(quests).toEqual([
      { id: 'quest.beacon', name: 'The beacon inquiry', state: 'investigating' },
      { id: 'quest.aftermath', name: 'Aftermath', state: 'not_started' },
    ])
    expect(shared).toEqual([])
  })

  it('takes a quest’s state from the stage it starts in, and says nothing when there is none', () => {
    // A running quest state lives in a save file this toolchain never sees, so
    // the starting stage is the only state a project records. An empty one is
    // "not recorded" rather than a stage whose name is the empty string.
    const [started, blank] = worldQuests([
      quest('quest.1', 'Started somewhere', 'open', []),
      quest('quest.2', 'Half written', '', []),
    ]).quests!
    expect(started!.state).toBe('open')
    expect(blank!.state).toBeNull()
  })

  it('files a scene two quests both claim under the first, and reports it', () => {
    // A canvas cannot draw one box inside two frames, so the tie is broken in
    // document order — and the scene is named, because a designer who cannot
    // find it in the quest they opened is owed the reason.
    const { questOf, shared } = worldQuests([
      quest('quest.1', 'First', 'open', ['scene.shared', 'scene.shared']),
      quest('quest.2', 'Second', 'open', ['scene.shared']),
    ])
    expect(questOf('scene.shared')).toBe('quest.1')
    expect(shared).toEqual(['scene.shared'])
  })

  it('says “cannot tell” for a World that has not answered, rather than “no quests”', () => {
    const unread = worldQuests(undefined)
    expect(unread.quests).toBeNull()
    expect(unread.questOf('scene.council')).toBeNull()
    // And an empty document is the other claim: quests were read, there are none.
    expect(worldQuests([]).quests).toEqual([])
  })

  it('carves a real arc up by quest and by the stage those quests start in', () => {
    const scenes = beaconScenes()
    const { quests, questOf } = worldQuests([
      quest('quest.beacon', 'The beacon inquiry', 'Investigating', [
        'scene.council',
        'scene.road',
        'scene.beacon',
      ]),
      quest('quest.aftermath', 'Aftermath', 'Investigating', ['scene.ruins']),
    ])
    const arc: FlowArc = {
      level: {
        id: 'arc.world',
        name: 'Every scene',
        groups: [],
        elements: Object.values(scenes).map((one) => sceneNode(one, questOf(one.id))),
        entryId: 'scene.council',
      },
      quests,
    }
    expect(arcGrouping(arc, 'quest').groups.map((group) => group.id)).toEqual([
      'quest.beacon',
      'quest.aftermath',
    ])
    // Two quests, one starting stage: four scenes in a single box.
    const byState = arcGrouping(arc, 'questState')
    expect(byState.groups).toEqual([{ id: 'state:Investigating', name: 'Investigating' }])
    expect(
      arc.level.elements.every((element) => byState.of(element) === 'state:Investigating'),
    ).toBe(true)
  })
})

describe('authoring an arc with the same three functions the scene canvas uses', () => {
  it('offers a spare exit only on a scene, and numbers it after the ones there are', () => {
    const arc = beaconArc()
    const road = scenesOf(arc).find((scene) => scene.id === 'scene.road')!
    expect(arcSpare(road)).toEqual({ id: 'exit.3', label: 'Exit 3', to: null })
    expect(arcSpare({ kind: 'end', id: 'end.1', title: 'End', out: [] })).toBeNull()
  })

  it('authors a new exit when something connects to the spare port', () => {
    const arc = beaconArc()
    const ruins = scenesOf(arc).find((scene) => scene.id === 'scene.ruins')!
    const spare = arcSpare(ruins)!
    // A scene with no ports at all gains one. Without this there would be
    // nothing to drag from and the gesture would silently do nothing.
    const opened = connectPort(
      arc.level,
      { elementId: 'scene.ruins', portId: spare.id, label: spare.label },
      'scene.council',
    )
    expect(opened.elements.find((e) => e.id === 'scene.ruins')?.out).toEqual([
      { id: 'exit.1', label: 'Exit 1', to: 'scene.council' },
    ])

    // And giving the orphan a way *in* clears the finding about it.
    const beacon = scenesOf(arc).find((scene) => scene.id === 'scene.beacon')!
    const wired = connectPort(
      opened,
      { elementId: 'scene.beacon', portId: arcSpare(beacon)!.id, label: 'Exit 2' },
      'scene.ruins',
    )
    expect(arcDiagnostics({ ...arc, level: wired }).some((d) => d.id.endsWith(':orphan'))).toBe(
      false,
    )
  })

  it('creates a scene with the same call the form path would make', () => {
    const arc = beaconArc()
    const created = addElement(arc.level, 'scene', 'scene.ruins')
    expect(created.element.kind).toBe('scene')
    expect(created.element.id).toBe('scene.1')
    expect(created.scene.elements).toHaveLength(5)
  })

  it('selects the scene itself, not the arc it sits in', () => {
    const arc = beaconArc()
    const scene = scenesOf(arc)[1]!
    expect(arcTarget(arc.level, scene)).toEqual({ sceneId: 'scene.road', beatId: null })
    expect(arcTarget(arc.level, null)).toEqual({ sceneId: null, beatId: null })
  })
})
