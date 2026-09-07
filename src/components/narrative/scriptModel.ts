import type { Beat, Scene, Speaker } from '../../lib/api'
import { removeElement } from './flow/model'
import { mintId, nodeId, patchScene, sceneToFlow } from './flow/source'

/** Form edits preserve the source document; deletion uses the same patch as Flow. */
export function removeScriptBeat(scene: Scene, beatId: string): Scene {
  const level = sceneToFlow(scene)
  return patchScene(scene, removeElement(level, nodeId.beat(beatId))).scene
}

export function duplicateScriptBeat(beat: Beat): Beat {
  const copy = structuredClone(beat)
  copy.id = mintId()
  copy.title = `${beat.title} (copy)`
  for (const slot of copy.dialogue ?? []) {
    slot.id = mintId()
    for (const variant of slot.variants ?? []) variant.id = mintId()
  }
  for (const exit of [...(copy.choices ?? []), ...(copy.outcomes ?? [])]) {
    exit.id = mintId()
  }
  return copy
}

export function speakerKey(speaker: Speaker): string {
  return typeof speaker === 'string' ? speaker : speaker.entity
}

export function speakerFromKey(key: string): Speaker {
  return key === 'player' || key === 'narrator' ? key : { entity: key }
}

export function beatHasLockedText(beat: Beat): boolean {
  return (beat.dialogue ?? []).some(
    (slot) =>
      slot.policy === 'locked' ||
      slot.variants?.some((variant) => variant.text.lifecycle?.policy === 'locked'),
  )
}
