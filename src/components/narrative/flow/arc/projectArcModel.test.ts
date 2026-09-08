import { describe, expect, it } from 'vitest'
import type { Scene } from '../../../../lib/api'
import type { ArcScene } from '../../../../lib/api/narrativeArc'
import type { Quest } from '../../../../lib/api/narrativeWorld'
import { applySceneEdit } from '../../sceneEdits'
import { buildGraph, NODE_BUDGET } from '../graph'
import { mintId } from '../source'
import { arcDiagnostics, arcGrouping } from './model'
import { draftArcScene, groupIdentity, projectArc, questStageId } from './projectArcModel'

function projection(scene: Scene): ArcScene {
  return draftArcScene(scene, {
    summary: { id: scene.id, name: scene.name, slug: 'scene', rel: 'scene.yaml' },
    actId: null,
    arcId: null,
    tagIds: [],
    participants: [],
    slots: 0,
    filled: 0,
    beats: 0,
    counts: { generated: 0, edited: 0, locked: 0, needsReview: 0, outOfDate: 0 },
    arc: { exits: [], needsText: 0 },
  })
}
const quest = (name: string, scenes: string[]): Quest => ({
  id: mintId(),
  name,
  summary: '',
  stages: ['open', 'closed'],
  initial: 'open',
  transitions: [{ from: 'open', to: 'closed', when: 'always' }],
  scene_ids: scenes,
})

describe('the authored project arc contract', () => {
  it('retains every cross-scene route and exact dangling owner without deriving edges from text or stage membership', () => {
    const target = mintId(),
      beat = mintId(),
      choice = mintId(),
      outcome = mintId()
    const scene: Scene = {
      id: mintId(),
      name: 'Mentions the harbour',
      summary: 'Go to the other scene',
      beats: [
        {
          id: beat,
          title: 'Opening',
          choices: [{ id: choice, label: 'Sail', to: { scene: target } }],
          outcomes: [{ id: outcome, to: { unresolved: {} } }],
        },
      ],
    }
    const q = quest('Inquiry', [scene.id, target])
    const arc = projectArc(
      [projection(scene), projection({ id: target, name: 'Harbour', beats: [] })],
      [q],
      '',
      () => undefined,
    )
    expect(arc.level.elements.filter((n) => n.kind === 'scene')).toHaveLength(2)
    expect(arc.level.elements.flatMap((n) => n.out)).toHaveLength(3)
    expect(arc.level.elements.find((n) => n.id === questStageId(q.id, 'open'))!.out[0]!.to).toBe(
      questStageId(q.id, 'closed'),
    )
    expect(arcDiagnostics(arc).find((d) => d.severity === 'error')?.at).toEqual({
      sceneId: scene.id,
      beatId: beat,
      outcomeId: outcome,
    })
    expect(arc.level.entryId).toBeNull()
  })

  it('keeps one shared scene in a stable combined group and diagnoses both missing ends of a quest transition', () => {
    const scene = projection({ id: mintId(), name: 'Shared evidence', beats: [] })
    const a = quest('Witness', [scene.summary.id]),
      b = quest('Council', [scene.summary.id])
    b.transitions = [{ from: 'undeclared', to: 'missing', when: 'always' }]
    const arc = projectArc([scene], [a, b], '', () => undefined)
    const groups = arcGrouping(arc, 'quest')
    const shared = arc.level.elements.find((n) => n.kind === 'scene')!
    expect(arc.level.elements.filter((n) => n.id === shared.id)).toHaveLength(1)
    expect(groups.groups.find((g) => g.id === groups.of(shared))!.name).toMatch(
      /Shared:.*Witness|Shared:.*Council/,
    )
    const renamed = projectArc([scene], [{ ...b, name: 'Renamed council' }, a], '', () => undefined)
    expect(arcGrouping(renamed, 'quest').of(shared)).toBe(groups.of(shared))
    expect(arcDiagnostics(arc).filter((d) => d.questId === b.id)).toHaveLength(2)
    const collapsed = buildGraph(arc.level, {
      grouping: groups,
      closedGroups: groups.groups.map((g) => g.id),
      dangling: true,
    })
    expect(collapsed.nodes.length).toBeLessThan(arc.level.elements.length)
    expect(arcDiagnostics(arc).filter((d) => d.severity === 'error')).toHaveLength(3)
    expect(
      arcDiagnostics(arc).find((d) => d.elementId === shared.id && d.severity === 'error')?.message,
    ).toContain('has no beats')
  })

  it('projects canonical Script destination edits and undo without touching locked text', () => {
    const original: Scene = {
      id: mintId(),
      name: 'Council',
      beats: [
        {
          id: mintId(),
          title: 'Verdict',
          dialogue: [
            {
              id: mintId(),
              speaker: 'narrator',
              policy: 'locked',
              variants: [
                {
                  id: mintId(),
                  text: {
                    revision: 'same',
                    body: 'Keep this evidence.',

                    lifecycle: { policy: 'locked', review: 'approved', freshness: 'current' },
                  },
                },
              ],
            },
          ],
          outcomes: [{ id: mintId(), to: { unresolved: {} } }],
        },
      ],
    }
    const target = mintId(),
      beat = original.beats![0]!,
      route = beat.outcomes![0]!
    const changed = applySceneEdit(original, {
      kind: 'setDestination',
      route: { kind: 'outcome', beatId: beat.id, id: route.id },
      value: { scene: target },
    })
    if ('refused' in changed) throw Error(changed.refused)
    const base = projection(original)
    expect(draftArcScene(changed.scene, base).arc.exits[0]).toMatchObject({
      routeId: route.id,
      beatId: beat.id,
      to: target,
    })
    expect(changed.scene.beats![0]!.dialogue).toEqual(beat.dialogue)
    expect(draftArcScene(original, base).arc.exits[0]!.to).toBeNull()
  })

  it('projects 1,000 scenes into a bounded canvas including selected distant scene', () => {
    const scenes = Array.from({ length: 1000 }, (_, i) =>
      projection({ id: mintId(), name: `Scene ${i}`, beats: [] }),
    )
    const arc = projectArc(scenes, [], '', () => undefined)
    const selected = scenes[999]!.summary.id
    const graph = buildGraph(arc.level, { focusId: selected, dangling: true })
    expect(graph.nodes.length).toBeLessThanOrEqual(NODE_BUDGET)
    expect(graph.nodes.some((n) => n.id === selected)).toBe(true)
    expect(arc.level.elements).toHaveLength(1000)
  })

  it('keeps presentation group keys canonical and separates membership and grouping modes', () => {
    const identities = ['quest:["a"]', 'quest:["a","b"]', 'questState:["a"]'].map(groupIdentity)
    expect(new Set(identities).size).toBe(3)
    for (const id of identities) expect(id).toMatch(/^[0-7][0-9A-HJKMNP-TV-Z]{25}$/)
  })
})
