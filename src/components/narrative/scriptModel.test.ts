import { describe, expect, it } from 'vitest'
import type { Beat, Scene } from '../../lib/api'
import { duplicateScriptBeat, removeScriptBeat } from './scriptModel'
import { mintId, nodeId, patchScene, sceneToFlow } from './flow/source'
import { removeElement } from './flow/model'

const beat: Beat = {
  id: mintId(),
  title: 'Evidence',
  dialogue: [
    {
      id: mintId(),
      speaker: 'player',
      variants: [{ id: mintId(), text: { body: 'I saw it.', revision: 'preserved' } }],
    },
  ],
  choices: [],
}
beat.choices = [{ id: mintId(), label: 'Repeat', to: { beat: beat.id } }]

describe('Script structural edits', () => {
  it('duplicates identities while preserving wording and authored destinations', () => {
    const copy = duplicateScriptBeat(beat)
    expect(copy.id).not.toBe(beat.id)
    expect(copy.dialogue?.[0]?.id).not.toBe(beat.dialogue?.[0]?.id)
    expect(copy.dialogue?.[0]?.variants?.[0]?.id).not.toBe(beat.dialogue?.[0]?.variants?.[0]?.id)
    expect(copy.dialogue?.[0]?.variants?.[0]?.text).toEqual({
      ...beat.dialogue?.[0]?.variants?.[0]?.text,
      lifecycle: { policy: 'edited', review: 'draft' },
    })
    expect(copy.choices?.[0]?.to).toEqual({ beat: beat.id })
    expect(beat.choices?.[0]?.to).toEqual({ beat: beat.id })
  })
  it('deletes with Flow semantics including tombstones and dangling references', () => {
    const scene: Scene = {
      id: mintId(),
      name: 'Council',
      beats: [
        beat,
        { id: mintId(), title: 'After', outcomes: [{ id: mintId(), to: { beat: beat.id } }] },
      ],
    }
    const deleted = removeScriptBeat(scene, beat.id)
    const level = sceneToFlow(scene)
    const flowDeleted = patchScene(scene, removeElement(level, nodeId.beat(beat.id))).scene
    expect(deleted.beats).toEqual(flowDeleted.beats)
    expect(deleted.tombstones?.map((one) => one.target)).toEqual(
      flowDeleted.tombstones?.map((one) => one.target),
    )
    expect(deleted.beats?.[0]?.outcomes?.[0]?.to).toEqual({ beat: beat.id })
  })
})
