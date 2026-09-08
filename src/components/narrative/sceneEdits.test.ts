import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Scene } from '../../lib/api'
import { applySceneEdit, type SceneEditOperation, type SceneEditResult } from './sceneEdits'
import {
  duplicateScriptBeat,
  removeScriptBeat,
  removeScriptSlot,
  removeScriptVariant,
} from './scriptModel'
import { connectPort, removeElement } from './flow/model'
import { nodeId, patchScene, sceneToFlow } from './flow/source'

const id = (value: number) => String(value).padStart(26, '0')
const at = '2026-09-08T12:00:00.000Z'
const route = { kind: 'choice' as const, beatId: id(2), id: id(5) }
const condition = {
  all: [
    'always' as const,
    { not: { compare: { var: 'trust', op: 'lt' as const, value: { literal: 40 } } } },
  ],
}
const effects = [
  { add: { var: 'trust', by: 10 } },
  { command: { name: 'journal_add', args: [{ literal: 'heard' }] } },
]
function fixture(): Scene {
  return {
    id: id(1),
    name: 'Ashfall hearing',
    editorial_head: 'immutable-receipt',
    summary: 'Three approaches reconverge at the verdict.',
    participants: [{ entity: id(8), role: 'witness' }],
    entry: { compare: { var: 'quest', op: 'eq', value: { literal: 'open' } } },
    beats: [
      {
        id: id(2),
        title: 'Present evidence',
        intents: [{ subject: 'player', intent: 'Challenge the account' }],
        must_convey: ['The logbook survived.'],
        must_not_reveal: ['The captain returned.'],
        dialogue: [
          {
            id: id(3),
            speaker: { entity: id(8) },
            policy: 'edited',
            variants: [
              {
                id: id(4),
                when: condition,
                text: {
                  revision: 'exact-wording-hash',
                  body: 'The ink is still legible.',
                  provenance: { generated: { fingerprint: 'original-request' } },
                  lifecycle: { policy: 'edited', review: 'approved', freshness: 'out_of_date' },
                },
              },
            ],
          },
        ],
        choices: [
          {
            id: id(5),
            label: 'Show the logbook',
            to: { beat: id(6) },
            requires: condition,
            effects,
          },
        ],
        outcomes: [{ id: id(7), to: { beat: id(2) }, when: 'never' }],
      },
      { id: id(6), title: 'Verdict' },
    ],
    tombstones: [{ target: { variant: id(9) }, label: 'Earlier wording', deleted_at: at }],
  }
}
function accepted(result: SceneEditResult) {
  if ('refused' in result) throw new Error(result.refused)
  return result
}
function run(scene: Scene, op: SceneEditOperation, firstId = 100) {
  let next = firstId
  return accepted(applySceneEdit(scene, op, { mintId: () => id(next++), now: () => at }))
}
afterEach(() => vi.useRealTimers())

describe('shared canonical scene operations', () => {
  it('makes identical destination edits from a Flow connection and typed form operation', () => {
    const source = fixture()
    const level = sceneToFlow(source)
    const choiceNode = level.elements.find((one) => one.id === nodeId.choice(route.id))!
    const connected = connectPort(
      level,
      { elementId: choiceNode.id, portId: choiceNode.out[0]!.id },
      nodeId.beat(id(2)),
    )
    const flow = patchScene(source, connected).scene
    const form = run(source, { kind: 'setDestination', route, value: { beat: id(2) } })
    expect(form.scene).toEqual(flow)
    expect(form.target).toEqual({
      sceneId: source.id,
      beatId: id(2),
      choiceId: id(5),
      field: 'destination',
    })
    expect(source).toEqual(fixture())
  })

  it('disconnects explicitly without deleting the route or fabricating an ending', () => {
    const source = fixture()
    const disconnected = run(source, { kind: 'setDestination', route, value: { unresolved: {} } })
    expect(disconnected.scene.beats![0]!.choices![0]).toEqual({
      ...source.beats![0]!.choices![0],
      to: { unresolved: {} },
    })
    const level = sceneToFlow(disconnected.scene)
    expect(level.elements.find((one) => one.id === nodeId.choice(route.id))?.out[0]?.to).toBeNull()
    expect(level.elements.some((one) => one.id === nodeId.end(nodeId.choice(route.id)))).toBe(false)
    expect(patchScene(disconnected.scene, level)).toMatchObject({
      scene: disconnected.scene,
      unchanged: true,
    })
    expect(
      run(disconnected.scene, {
        kind: 'setDestination',
        route,
        value: { end: { label: 'hearing_complete' } },
      }).scene.beats![0]!.choices![0]!.to,
    ).toEqual({ end: { label: 'hearing_complete' } })
  })

  it('inserts and reorders beats without rewiring entry or named links, then selects a surviving ancestor', () => {
    const source = fixture()
    const added = run(source, { kind: 'addBeat', title: 'A final question', afterId: id(2) })
    expect(added.scene.beats!.map((beat) => beat.id)).toEqual([id(2), id(100), id(6)])
    expect(added.created).toEqual([{ sceneId: id(1), beatId: id(100) }])
    expect(added.scene.entry).toEqual(source.entry)
    expect(added.scene.beats![0]).toEqual(source.beats![0])
    const only: Scene = {
      id: id(1),
      name: 'Empty afterward',
      beats: [{ id: id(2), title: 'Last beat' }],
    }
    const removed = run(only, { kind: 'removeBeat', beatId: id(2) })
    expect(removed.target).toEqual({ sceneId: id(1), beatId: null })
    expect(removed.scene.beats).toEqual([])
  })

  it('edits narrow scene, beat and speaker fields while preserving editorial authority', () => {
    const source = fixture()
    const classified = run(source, {
      kind: 'patchScene',
      changes: { act_id: id(20), arc_id: id(21), tag_ids: [id(22)] },
    }).scene
    const titled = run(classified, {
      kind: 'patchBeat',
      beatId: id(2),
      changes: { title: 'Read the logbook', must_convey: ['Ink proves the timing.'] },
    }).scene
    const named = run(titled, {
      kind: 'setChoiceLabel',
      beatId: id(2),
      choiceId: id(5),
      value: 'Read it aloud',
    }).scene
    const spoken = run(named, {
      kind: 'setSpeaker',
      beatId: id(2),
      slotId: id(3),
      value: 'player',
    }).scene
    const expected = structuredClone(source)
    Object.assign(expected, { act_id: id(20), arc_id: id(21), tag_ids: [id(22)] })
    Object.assign(expected.beats![0]!, {
      title: 'Read the logbook',
      must_convey: ['Ink proves the timing.'],
    })
    expected.beats![0]!.choices![0]!.label = 'Read it aloud'
    expected.beats![0]!.dialogue![0]!.speaker = 'player'
    expect(spoken).toEqual(expected)
    expect(
      run(spoken, { kind: 'patchScene', changes: { act_id: undefined } }).scene,
    ).not.toHaveProperty('act_id')
  })

  it('preserves full source and opaque fields while changing typed route fields', () => {
    const source = Object.assign(fixture(), {
      future_content: { language: 'original', enabled: true },
    })
    const guarded = run(source, {
      kind: 'setCondition',
      owner: route,
      value: { any: [condition, 'never'] },
    }).scene
    const changed = run(guarded, { kind: 'setEffects', route, value: [...effects].reverse() }).scene
    const expected = structuredClone(source)
    expected.beats![0]!.choices![0]!.requires = { any: [condition, 'never'] }
    expected.beats![0]!.choices![0]!.effects = [...effects].reverse()
    expect(changed).toEqual(expected)
    expect(source).toEqual(
      Object.assign(fixture(), { future_content: { language: 'original', enabled: true } }),
    )
  })

  it('creates three routes reconverging on one beat without multiplying its dialogue', () => {
    let source = fixture()
    for (const [label, firstId] of [
      ['Diplomatic', 100],
      ['Direct', 200],
    ] as const) {
      const added = run(
        source,
        {
          kind: 'addChoice',
          beatId: id(2),
          label,
          to: { beat: id(6) },
          condition,
          effects,
        },
        firstId,
      )
      source = added.scene
    }
    const level = sceneToFlow(source)
    expect(level.elements.filter((one) => one.kind === 'beat')).toHaveLength(2)
    const choices = level.elements.filter((one) => one.kind === 'choice')
    expect(choices).toHaveLength(3)
    expect(choices.every((one) => one.out[0]?.to === nodeId.beat(id(6)))).toBe(true)
    expect(source.beats![0]!.dialogue).toEqual(fixture().beats![0]!.dialogue)
  })

  it('shares beat deletion with Script and Flow, including tombstones and dangling incoming IDs', () => {
    vi.useFakeTimers().setSystemTime(new Date(at))
    const source = fixture()
    const gone = run(source, { kind: 'removeBeat', beatId: id(6) })
    expect(gone.scene).toEqual(removeScriptBeat(source, id(6)))
    expect(gone.scene).toEqual(
      patchScene(source, removeElement(sceneToFlow(source), nodeId.beat(id(6)))).scene,
    )
    expect(gone.scene.beats![0]!.choices![0]!.to).toEqual({ beat: id(6) })
    expect(gone.scene.tombstones).toContainEqual({
      target: { beat: id(6) },
      label: 'Verdict',
      deleted_at: at,
    })
    expect(gone.target).toEqual({ sceneId: source.id, beatId: id(2) })
    expect(gone.removed).toEqual([{ sceneId: source.id, beatId: id(6) }])
  })

  it('retires removed dialogue identities once and reports a surviving focus target', () => {
    vi.useFakeTimers().setSystemTime(new Date(at))
    const source = fixture()
    const gone = run(source, { kind: 'removeBeat', beatId: id(2) })
    expect(gone.removed).toHaveLength(5)
    expect(gone.scene.tombstones!.map((one) => one.target)).toEqual([
      { variant: id(9) },
      { beat: id(2) },
      { dialogue_slot: id(3) },
      { variant: id(4) },
    ])
    expect(gone.target.beatId).toBe(id(6))
    const slot = run(source, { kind: 'removeSlot', beatId: id(2), slotId: id(3) })
    expect(slot.scene).toEqual(removeScriptSlot(source, id(2), id(3)))
    const variant = run(source, {
      kind: 'removeVariant',
      beatId: id(2),
      slotId: id(3),
      variantId: id(4),
    })
    expect(variant.scene).toEqual(removeScriptVariant(source, id(2), id(3), id(4)))
    expect(variant.target).toEqual({ sceneId: id(1), beatId: id(2), lineId: id(3) })
    expect(
      applySceneEdit(variant.scene, {
        kind: 'removeVariant',
        beatId: id(2),
        slotId: id(3),
        variantId: id(4),
      }),
    ).toHaveProperty('refused')
  })

  it('duplicates fresh identities while preserving authored links, wording, provenance and revisions', () => {
    const source = fixture()
    const result = run(source, { kind: 'duplicateBeat', beatId: id(2) })
    const copy = result.scene.beats![1]!
    expect(result.created).toHaveLength(5)
    expect(
      new Set(
        result.created.flatMap((target) => [
          target.choiceId ?? target.outcomeId ?? target.variantId ?? target.lineId ?? target.beatId,
        ]),
      ).size,
    ).toBe(5)
    expect(copy.outcomes![0]!.to).toEqual({ beat: id(2) })
    const words = copy.dialogue![0]!.variants![0]!.text
    expect(words).toEqual({
      ...source.beats![0]!.dialogue![0]!.variants![0]!.text,
      lifecycle: { policy: 'edited', review: 'draft', freshness: 'out_of_date' },
    })
    const script = duplicateScriptBeat(source.beats![0]!)
    expect(script.dialogue![0]!.variants![0]!.text).toEqual(words)
    expect(result.scene.tombstones).toEqual(source.tombstones)
  })

  it('refuses protected deletion and speaker/condition edits without blocking unrelated routes', () => {
    const source = fixture()
    source.beats![0]!.dialogue![0]!.variants![0]!.text.lifecycle!.policy = 'locked'
    const before = structuredClone(source)
    const protectedEdits: SceneEditOperation[] = [
      { kind: 'removeBeat', beatId: id(2) },
      { kind: 'removeSlot', beatId: id(2), slotId: id(3) },
      { kind: 'removeVariant', beatId: id(2), slotId: id(3), variantId: id(4) },
      { kind: 'setSpeaker', beatId: id(2), slotId: id(3), value: 'player' },
      {
        kind: 'setCondition',
        owner: { kind: 'variant', beatId: id(2), slotId: id(3), id: id(4) },
        value: 'never',
      },
    ]
    for (const operation of protectedEdits)
      expect(applySceneEdit(source, operation)).toHaveProperty('refused')
    expect(run(source, { kind: 'setEffects', route, value: [] }).changed).toBe(true)
    expect(
      run(source, { kind: 'duplicateBeat', beatId: id(2) }).scene.beats![1]!.dialogue![0]!
        .variants![0]!.text.lifecycle,
    ).toEqual({ policy: 'locked', review: 'draft', freshness: 'out_of_date' })
    expect(source).toEqual(before)
  })

  it('keeps route order, labels, guards and ordered effects when moving or deleting routes', () => {
    const source = run(fixture(), {
      kind: 'addOutcome',
      beatId: id(2),
      to: { end: { label: 'hearing_complete' } },
      condition,
      effects,
    }).scene
    const ref = { kind: 'outcome' as const, beatId: id(2), id: id(100) }
    const moved = run(source, { kind: 'moveRoute', route: ref, index: 0 }).scene
    expect(moved.beats![0]!.outcomes![0]).toEqual(source.beats![0]!.outcomes![1])
    const removed = run(moved, { kind: 'removeRoute', route: ref })
    expect(removed.scene.beats![0]!.outcomes).toEqual(fixture().beats![0]!.outcomes)
    expect(removed.target).toEqual({ sceneId: id(1), beatId: id(2) })
    const reordered = run(removed.scene, { kind: 'moveBeat', beatId: id(6), index: 0 })
    expect(reordered.scene.beats![1]).toEqual(removed.scene.beats![0])
    expect(reordered.scene.beats![1]!.choices![0]!.to).toEqual({ beat: id(6) })
  })

  it('adds and orders dialogue identities without rewriting existing wording or editorial proof', () => {
    const source = fixture()
    const slot = run(source, { kind: 'addSlot', beatId: id(2), speaker: 'player' })
    expect(slot.target).toEqual({ sceneId: id(1), beatId: id(2), lineId: id(100) })
    const added = run(
      slot.scene,
      { kind: 'addVariant', beatId: id(2), slotId: id(100), body: 'Read it aloud.' },
      101,
    )
    expect(added.created).toEqual([
      { sceneId: id(1), beatId: id(2), lineId: id(100), variantId: id(101) },
    ])
    const second = run(added.scene, { kind: 'addVariant', beatId: id(2), slotId: id(100) }, 102)
    const owner = { kind: 'variant' as const, beatId: id(2), slotId: id(100), id: id(102) }
    const reordered = run(second.scene, { kind: 'moveVariant', owner, index: 0 }).scene
    const moved = run(reordered, {
      kind: 'moveSlot',
      beatId: id(2),
      slotId: id(100),
      index: 0,
    }).scene
    expect(moved.beats![0]!.dialogue![0]!.variants!.map((one) => one.id)).toEqual([
      id(102),
      id(101),
    ])
    expect(moved.beats![0]!.dialogue![1]).toEqual(source.beats![0]!.dialogue![0])
    expect(moved.editorial_head).toBe(source.editorial_head)
    const originalOwner = { kind: 'variant' as const, beatId: id(2), slotId: id(3), id: id(4) }
    const edited = run(moved, {
      kind: 'setVariantBody',
      owner: originalOwner,
      value: 'The date is still legible.',
    }).scene
    expect(edited.beats![0]!.dialogue![1]!.variants![0]!.text).toEqual({
      ...source.beats![0]!.dialogue![0]!.variants![0]!.text,
      body: 'The date is still legible.',
    })
    source.beats![0]!.dialogue![0]!.variants![0]!.text.lifecycle!.policy = 'locked'
    expect(
      applySceneEdit(source, {
        kind: 'setVariantBody',
        owner: originalOwner,
        value: 'Forbidden change',
      }),
    ).toHaveProperty('refused')
    expect(source.beats![0]!.dialogue![0]!.variants![0]!.text.body).toBe(
      'The ink is still legible.',
    )
  })

  it('preserves unchanged identity and rejects missing targets, collisions and forbidden patch fields atomically', () => {
    const source = fixture()
    expect(
      run(source, { kind: 'patchBeat', beatId: id(2), changes: { title: 'Present evidence' } }),
    ).toMatchObject({ scene: source, changed: false })
    expect(run(source, { kind: 'patchScene', changes: {} }).scene).toBe(source)
    expect(
      applySceneEdit(
        source,
        { kind: 'addBeat', title: 'Collision' },
        { mintId: () => id(4), now: () => at },
      ),
    ).toHaveProperty('refused')
    expect(applySceneEdit(source, { kind: 'moveBeat', beatId: id(2), index: 10 })).toHaveProperty(
      'refused',
    )
    expect(
      applySceneEdit(source, { kind: 'removeRoute', route: { ...route, id: id(90) } }),
    ).toHaveProperty('refused')
    expect(
      applySceneEdit(source, {
        kind: 'patchBeat',
        beatId: id(2),
        changes: { title: 'Changed', dialogue: [] },
      } as unknown as SceneEditOperation),
    ).toHaveProperty('refused')
    expect(source).toEqual(fixture())
  })
})
