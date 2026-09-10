import { describe, expect, it } from 'vitest'
import type { Beat, Scene } from '../../../lib/api'
import type { Layout, NodeLayout } from '../../../lib/api'
import {
  beatCounts,
  conditionText,
  effectRows,
  layoutKeyOf,
  layoutWithPositions,
  mintId,
  nodeId,
  nodeOfLayoutKey,
  patchScene,
  positionsFromLayout,
  sceneToFlow,
  NEW_OUTCOME_PORT,
} from './source'

/** A layout sidecar holding exactly these boxes. */
function sidecar(nodes: Record<string, NodeLayout>): Layout {
  return {
    schemaVersion: 1,
    graph: { kind: 'scene', scene: 'scene' },
    mode: 'manual',
    modeUpdatedAt: 'then',
    nodes,
    groups: {},
    annotations: {},
  }
}
import { addElement, connectPort, removeElement, type FlowBeat } from './model'

/*
 * The tests that stand between a writer and losing every line of dialogue in a
 * project.
 *
 * `patchScene` is the only thing in the frontend that produces a scene document
 * to be written over somebody's file. The canvas it is fed cannot represent
 * dialogue, revisions, provenance, intents, `must_not_reveal`, typed conditions
 * or typed effects, so any implementation that *rebuilds* a scene from one
 * destroys all of it — silently, on the first drag, with no error and no
 * conflict. The first two tests below are the ones that catch that.
 */

/** Twenty-six Crockford characters, exactly as `wobu_core::Id` parses one. */
function id(): string {
  return mintId()
}

const KAEL = id()
const MIRA = id()

/**
 * A scene with one of everything the canvas cannot draw.
 *
 * Written out rather than generated: the point is that a reader can see, in one
 * screen, the fields a lossy save would take with it.
 */
function ashfall(): Scene {
  const arrival = id()
  const verdict = id()
  const slot = id()
  const variant = id()
  return {
    id: id(),
    name: 'Council hearing',
    summary: 'The council hears the beacon evidence.',
    participants: [
      { entity: KAEL, role: 'accuser' },
      { entity: MIRA, role: 'witness' },
    ],
    entry: { compare: { var: 'beacon_quest', op: 'eq', value: { literal: 'investigating' } } },
    beats: [
      {
        id: arrival,
        title: 'Arrival at the hearing',
        intents: [{ subject: { entity: KAEL }, intent: 'accuse the captain' }],
        must_convey: ['the beacon fell before the storm'],
        must_not_reveal: ['Mira was there'],
        dialogue: [
          {
            id: slot,
            speaker: { entity: MIRA },
            policy: 'locked',
            variants: [
              {
                id: variant,
                when: { compare: { var: 'trust', op: 'ge', value: { literal: 40 } } },
                text: {
                  revision: 'rev-one',
                  body: 'You brought the logbook after all.',
                  provenance: { generated: { fingerprint: 'fp-one' } },
                  lifecycle: { policy: 'edited', review: 'approved', freshness: 'out_of_date' },
                },
              },
            ],
          },
          { id: id(), speaker: 'narrator' },
        ],
        choices: [
          {
            id: id(),
            label: 'Show the logbook',
            requires: { compare: { var: 'has_logbook', op: 'eq', value: { literal: true } } },
            effects: [{ add: { var: 'support', by: 10 } }],
            to: { beat: verdict },
          },
          {
            id: id(),
            label: 'Walk out',
            effects: [{ command: { name: 'start_combat', args: [{ literal: 'guards' }] } }],
            to: { end: { label: '' } },
          },
        ],
      },
      {
        id: verdict,
        title: 'The verdict',
        outcomes: [
          {
            id: id(),
            when: { any: ['always', { not: 'never' }] },
            effects: [{ set: { var: 'beacon_quest', value: { literal: 'sanctioned' } } }],
            to: { scene: id() },
          },
          {
            id: id(),
            effects: [{ add: { var: 'trust', by: -8 } }],
            to: { end: { label: 'Done' } },
          },
        ],
      },
    ],
    tombstones: [{ target: { beat: id() }, label: 'Recess', deleted_at: '2026-01-01T00:00:00Z' }],
  }
}

describe('the round trip that has to change nothing', () => {
  it('gives back a byte-identical document when the canvas made no edit', () => {
    /*
     * The whole safety property, in one assertion.
     *
     * `sceneToFlow` throws away the dialogue, the revisions, the provenance,
     * the lifecycle, the intents, the typed conditions and the typed effects —
     * that is what makes the canvas cheap enough to draw 300 boxes. So if
     * `patchScene` ever *reconstructs* a scene rather than patching the one it
     * was given, this deep equality is the first thing to fail, and it fails
     * before anybody's file is touched.
     */
    const before = ashfall()
    const after = patchScene(before, sceneToFlow(before))
    expect(after.scene).toEqual(before)
    expect(after.unchanged).toBe(true)
  })

  it('still changes nothing after a round trip through the graph the canvas draws', () => {
    // Not the same test: this one goes through the view model's own operations
    // — the ones every editor calls — rather than through the adapter alone.
    const before = ashfall()
    const level = sceneToFlow(before)
    const untouched = connectPort(
      level,
      { elementId: level.elements[0]!.id, portId: 'then' },
      level.elements[0]!.out[0]?.to ?? null,
    )
    expect(patchScene(before, untouched).scene).toEqual(before)
  })

  it('preserves an ending whose label is empty rather than writing “End” into it', () => {
    // The box says "End" because an unlabelled ending has to be readable. If
    // the patch ever read a destination back off a title, this document would
    // gain a label nobody typed — and the export would ship it.
    const before = ashfall()
    const walkOut = before.beats![0]!.choices![1]!
    expect(walkOut.to).toEqual({ end: { label: '' } })
    expect(patchScene(before, sceneToFlow(before)).scene.beats![0]!.choices![1]!.to).toEqual({
      end: { label: '' },
    })
  })
})

describe('one structural edit changes one field', () => {
  it('moves a destination and leaves every revision, lifecycle and intent alone', () => {
    const before = ashfall()
    const level = sceneToFlow(before)
    const choice = before.beats![0]!.choices![0]!
    const arrival = before.beats![0]!
    const verdict = before.beats![1]!

    // Re-point "Show the logbook" from the verdict back at the arrival beat.
    const edited = connectPort(
      level,
      { elementId: nodeId.choice(choice.id), portId: 'then' },
      nodeId.beat(arrival.id),
    )
    const { scene, unchanged } = patchScene(before, edited)
    expect(unchanged).toBe(false)

    // The intended change, and only it.
    expect(scene.beats![0]!.choices![0]!.to).toEqual({ beat: arrival.id })

    // Everything else, asserted by rebuilding the expected document from the
    // original: any field the patch touched anywhere else fails this.
    const expected = structuredClone(before)
    expected.beats![0]!.choices![0]!.to = { beat: arrival.id }
    expect(scene).toEqual(expected)

    // Said again on the two things that would be catastrophic and are easy to
    // miss inside a deep equality: the words, and what they are keyed to.
    const variant = scene.beats![0]!.dialogue![0]!.variants![0]!
    expect(variant.text.body).toBe('You brought the logbook after all.')
    expect(variant.text.revision).toBe('rev-one')
    expect(variant.text.provenance).toEqual({ generated: { fingerprint: 'fp-one' } })
    expect(variant.text.lifecycle).toEqual({
      policy: 'edited',
      review: 'approved',
      freshness: 'out_of_date',
    })
    expect(scene.beats![0]!.must_not_reveal).toEqual(['Mira was there'])
    expect(scene.beats![1]!.outcomes![0]!.effects).toEqual(verdict.outcomes![0]!.effects)
  })

  it('refuses to clear a destination, because the source model has no “nowhere”', () => {
    // The canvas can say `to: null`; `Destination` cannot. A patch that took it
    // literally would have to delete the choice — and with it the label, the
    // condition and the effects the writer wrote.
    const before = ashfall()
    const choice = before.beats![0]!.choices![0]!
    const cleared = connectPort(
      sceneToFlow(before),
      { elementId: nodeId.choice(choice.id), portId: 'then' },
      null,
    )
    expect(patchScene(before, cleared).scene).toEqual(before)
  })
})

describe('what a canvas edit is allowed to do', () => {
  it('adds a beat as a beat, with a real identity and nothing invented in it', () => {
    const before = ashfall()
    const added = addElement(sceneToFlow(before), 'beat', null)
    const { scene, created } = patchScene(before, added.scene)
    expect(scene.beats).toHaveLength(3)
    const fresh = scene.beats![2]!
    expect(fresh.id).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/)
    expect(fresh.title).toBe('New beat')
    // No dialogue, no choices, no outcomes: a beat with no way out is what the
    // backend's `no_destination` diagnostic is for, and inventing an ending to
    // silence it would be writing a story decision nobody made.
    expect(fresh.dialogue).toBeUndefined()
    expect(fresh.outcomes).toBeUndefined()
    expect(created).toEqual([nodeId.beat(fresh.id)])
    // The two beats that were already there are untouched.
    expect(scene.beats!.slice(0, 2)).toEqual(before.beats)
  })

  it('deletes a beat with tombstones, and leaves the routes into it dangling', () => {
    const before = ashfall()
    const verdict = before.beats![1]!
    const level = sceneToFlow(before)
    const { scene } = patchScene(before, removeElement(level, nodeId.beat(verdict.id)))

    expect(scene.beats!.map((beat) => beat.id)).toEqual([before.beats![0]!.id])
    // Not repaired. The choice still names the beat that used to be there,
    // which is what turns the deletion into a diagnostic somebody can answer
    // rather than into a silent rewrite of where the story went.
    expect(scene.beats![0]!.choices![0]!.to).toEqual({ beat: verdict.id })
    // And the tombstone is what lets that diagnostic say "The verdict" instead
    // of a ULID.
    expect(scene.tombstones).toContainEqual(
      expect.objectContaining({ target: { beat: verdict.id }, label: 'The verdict' }),
    )
    // The one that was already recorded is still recorded.
    expect(scene.tombstones![0]).toEqual(before.tombstones![0])
  })

  it('leaves a tombstone per line, so a recording script can still be explained', () => {
    const before = ashfall()
    const arrival = before.beats![0]!
    const slot = arrival.dialogue![0]!
    const { scene } = patchScene(
      before,
      removeElement(sceneToFlow(before), nodeId.beat(arrival.id)),
    )
    expect(scene.tombstones).toContainEqual(
      expect.objectContaining({ target: { dialogue_slot: slot.id } }),
    )
    expect(scene.tombstones).toContainEqual(
      expect.objectContaining({
        target: { variant: slot.variants![0]!.id },
        label: 'You brought the logbook after all.',
      }),
    )
  })

  it('removes a choice from its beat and nothing else in the document', () => {
    const before = ashfall()
    const walkOut = before.beats![0]!.choices![1]!
    const { scene } = patchScene(
      before,
      removeElement(sceneToFlow(before), nodeId.choice(walkOut.id)),
    )
    const expected = structuredClone(before)
    expected.beats![0]!.choices!.splice(1, 1)
    expect(scene).toEqual(expected)
  })

  it('authors an outcome when a beat’s spare handle is connected, and only then', () => {
    const before = ashfall()
    const arrival = before.beats![0]!
    const verdict = before.beats![1]!
    const level = sceneToFlow(before)

    // Dragged nowhere: nothing to author, because the destination is the whole
    // of what the gesture supplies.
    const nowhere = connectPort(
      level,
      { elementId: nodeId.beat(arrival.id), portId: NEW_OUTCOME_PORT, label: 'A way out' },
      null,
    )
    expect(patchScene(before, nowhere).scene).toEqual(before)

    const wired = connectPort(
      level,
      { elementId: nodeId.beat(arrival.id), portId: NEW_OUTCOME_PORT, label: 'A way out' },
      nodeId.beat(verdict.id),
    )
    const { scene } = patchScene(before, wired)
    expect(scene.beats![0]!.outcomes).toHaveLength(1)
    expect(scene.beats![0]!.outcomes![0]!.to).toEqual({ beat: verdict.id })
    expect(scene.beats![0]!.outcomes![0]!.id).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/)
    // No condition and no effects: a wire says where, and says nothing about
    // when or what it does.
    expect(scene.beats![0]!.outcomes![0]!.when).toBeUndefined()
    expect(scene.beats![0]!.outcomes![0]!.effects).toBeUndefined()
  })

  it('copies an ending rather than rebuilding one when a wire lands on its box', () => {
    // Pointing one choice at another's End box means "end the same way". The
    // label is copied out of the document, never read off the box's title.
    const before = ashfall()
    const showLogbook = before.beats![0]!.choices![0]!
    const done = before.beats![1]!.outcomes![1]!
    const wired = connectPort(
      sceneToFlow(before),
      { elementId: nodeId.choice(showLogbook.id), portId: 'then' },
      nodeId.end(nodeId.outcome(done.id)),
    )
    expect(patchScene(before, wired).scene.beats![0]!.choices![0]!.to).toEqual({
      end: { label: 'Done' },
    })
  })
})

describe('the boxes a document becomes', () => {
  it('draws one box per beat, choice and outcome, keyed by the layout’s own names', () => {
    const scene = ashfall()
    const level = sceneToFlow(scene)
    const ids = level.elements.map((element) => element.id)
    expect(ids).toContain(nodeId.beat(scene.beats![0]!.id))
    expect(ids).toContain(nodeId.choice(scene.beats![0]!.choices![0]!.id))
    expect(ids).toContain(nodeId.outcome(scene.beats![1]!.outcomes![0]!.id))
    // Ids *are* layout keys, so #185's stored coordinates need no translation.
    for (const kind of ['beat', 'choice', 'outcome']) {
      const node = ids.find((one) => one.startsWith(`${kind}:`))!
      expect(layoutKeyOf(node, 'scene')).toBe(node)
      expect(nodeOfLayoutKey(node, 'scene')).toBe(node)
    }
    // A derived box has no identity to file a coordinate under, and sending one
    // would be rejected by `NodeKey`'s parser at the bridge.
    const ending = ids.find((one) => one.startsWith('end:'))!
    expect(layoutKeyOf(ending, 'scene')).toBeNull()
  })

  it('keeps a beat to counts, never to its text', () => {
    const scene = ashfall()
    const beat = sceneToFlow(scene).elements.find(
      (element): element is FlowBeat => element.id === nodeId.beat(scene.beats![0]!.id),
    )!
    expect(beat.lines).toBe(2)
    expect(beat.variants).toBe(1)
    expect(JSON.stringify(beat)).not.toContain('You brought the logbook')
  })

  it('counts the three lifecycle dimensions separately, and never collapses them', () => {
    // One slot: locked, approved, and out of date. Every one of those is true at
    // once, and a summary that picked one would hide the other two.
    const beat: Beat = {
      id: id(),
      title: 'Present evidence',
      dialogue: [
        {
          id: id(),
          speaker: 'narrator',
          policy: 'locked',
          variants: [
            {
              id: id(),
              text: {
                revision: 'r',
                body: 'x',
                lifecycle: { policy: 'locked', review: 'approved', freshness: 'out_of_date' },
              },
            },
          ],
        },
        { id: id(), speaker: 'narrator' },
      ],
    }
    expect(beatCounts(beat)).toEqual({
      needsText: 1,
      needsReview: 0,
      outOfDate: 1,
      locked: 1,
    })
  })

  it('reads a beat’s participants off who speaks and who intends, not off a guess', () => {
    const scene = ashfall()
    const beat = sceneToFlow(scene, {
      nameOf: (entity) => (entity === MIRA ? 'Mira' : undefined),
    }).elements.find((element): element is FlowBeat => element.kind === 'beat')!
    expect(beat.participants).toEqual([KAEL, 'Mira', 'Narrator'].sort())
  })

  it('puts a scene’s boxes in the containers the layout sidecar drew', () => {
    const scene = ashfall()
    const beat = nodeId.beat(scene.beats![0]!.id)
    const level = sceneToFlow(scene, {
      layout: {
        schemaVersion: 1,
        graph: { kind: 'scene', scene: scene.id },
        mode: 'manual',
        modeUpdatedAt: '2026-01-01T00:00:00Z',
        nodes: {},
        groups: {
          'group.evidence': {
            id: 'group.evidence',
            label: 'Evidence review',
            members: [beat],
            updatedAt: '2026-01-01T00:00:00Z',
          },
        },
        annotations: {},
      },
    })
    expect(level.groups).toEqual([{ id: 'group.evidence', name: 'Evidence review' }])
    expect(level.elements.find((element) => element.id === beat)?.groupId).toBe('group.evidence')
  })
})

describe('no arc edge is ever inferred from prose', () => {
  it('makes a scene link from a destination, and from nothing a writer typed', () => {
    /*
     * #187's property, held against a real document rather than a fixture.
     *
     * Every string in this scene names another scene. If anything on the path
     * from a document to the canvas read a title, a summary, an intent or a
     * line of dialogue looking for a reference, this scene would sprout edges —
     * and a designer would be shown a transition the runtime will never take,
     * which is the one thing the arc view must never do.
     */
    const other = id()
    const real = id()
    const doc: Scene = {
      id: id(),
      name: `A scene that leads to ${other}`,
      summary: `Kael goes to ${other} and then to ${real}.`,
      beats: [
        {
          id: id(),
          title: `Talk about ${other}`,
          intents: [{ subject: 'narrator', intent: `mention ${other}` }],
          must_convey: [other],
          dialogue: [
            {
              id: id(),
              speaker: 'narrator',
              variants: [{ id: id(), text: { revision: 'r', body: `We should go to ${other}.` } }],
            },
          ],
          outcomes: [{ id: id(), to: { scene: real } }],
        },
      ],
    }
    const links = sceneToFlow(doc).elements.filter((element) => element.kind === 'sceneLink')
    expect(links).toHaveLength(1)
    expect(links[0]).toMatchObject({ targetSceneId: real })
  })
})

describe('the arrangement sidecar', () => {
  it('reads back only the coordinates for boxes this level draws', () => {
    const beat = nodeId.beat(id())
    const layout = sidecar({ [beat]: { x: 10, y: 20, updatedAt: 'then' } })
    expect(positionsFromLayout(layout, 'scene')).toEqual({ [beat]: { x: 10, y: 20 } })
    // A scene key in a scene's sidecar is the scene itself, which is not a box
    // on this canvas — the canvas *is* the scene.
    expect(
      positionsFromLayout(sidecar({ [`scene:${id()}`]: { x: 1, y: 2, updatedAt: 't' } }), 'scene'),
    ).toEqual({})
  })

  it('never sends a key the layout format cannot name, and stamps only what moved', () => {
    /*
     * The filter that keeps #185's "a layout save can never fail" true.
     *
     * An ending's box has no identity — it is a picture of a destination field —
     * so `NodeKey` has no spelling for it, and a layout carrying one would be
     * rejected by serde at the bridge before the command ever ran. Dropping it
     * here is what makes the promise structural rather than a `catch`.
     */
    const beat = nodeId.beat(id())
    const outcome = nodeId.outcome(id())
    const ending = nodeId.end(outcome)
    const before = sidecar({ [beat]: { x: 0, y: 0, updatedAt: 'yesterday' } })
    const after = layoutWithPositions(
      before,
      { [beat]: { x: 0, y: 0 }, [outcome]: { x: 5, y: 6 }, [ending]: { x: 9, y: 9 } },
      'scene',
      'today',
    )
    expect(Object.keys(after.nodes).sort()).toEqual([beat, outcome].sort())
    // The box nobody moved keeps its stamp: `updatedAt` is the whole of the
    // merge rule, so re-stamping it would win an argument we are not having.
    expect(after.nodes[beat]?.updatedAt).toBe('yesterday')
    expect(after.nodes[outcome]).toEqual({ x: 5, y: 6, updatedAt: 'today' })
    // And nothing about the source moved.
    expect(after.graph).toEqual(before.graph)
  })
})

describe('typed conditions and effects, said in words', () => {
  it('renders each shape the source model allows, one way only', () => {
    expect(conditionText(undefined)).toBeNull()
    expect(conditionText('always')).toBe('always')
    // Not an unsatisfiable comparison: a branch the author closed deliberately.
    expect(conditionText('never')).toBe('never')
    expect(conditionText({ not: 'always' })).toBe('not (always)')
    expect(
      conditionText({
        all: [
          { compare: { var: 'trust', op: 'ge', value: { literal: 40 } } },
          { compare: { var: 'support', op: 'lt', value: { var: 'threshold' } } },
        ],
      }),
    ).toBe('trust ≥ 40 and support < threshold')
    expect(conditionText({ any: ['always', 'never'] })).toBe('always or never')
  })

  it('renders an increment, an assignment and a host command as rows', () => {
    expect(
      effectRows([
        { add: { var: 'support', by: 10 } },
        { add: { var: 'trust', by: -8 } },
        { set: { var: 'beacon_quest', value: { literal: 'sanctioned' } } },
        { command: { name: 'start_combat', args: [{ literal: 'guards' }, { var: 'wave' }] } },
      ]),
    ).toEqual([
      { variable: 'support', operator: '+', value: '10' },
      { variable: 'trust', operator: '−', value: '8' },
      { variable: 'beacon_quest', operator: '=', value: 'sanctioned' },
      { variable: 'start_combat', operator: 'call', value: 'guards, wave' },
    ])
  })
})

describe('minted identities', () => {
  it('mints something `wobu_core::Id` will parse, and a different one each time', () => {
    const one = mintId()
    expect(one).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/)
    expect(mintId()).not.toBe(one)
  })
})

describe('the arc level’s keys', () => {
  it('files a scene box under the scene key, and a dangling box under none', () => {
    const scene = id()
    expect(layoutKeyOf(scene, 'arc')).toBe(`scene:${scene}`)
    expect(nodeOfLayoutKey(`scene:${scene}`, 'arc')).toBe(scene)
    expect(layoutKeyOf('beat.1:then:unresolved', 'arc')).toBeNull()
  })
})
