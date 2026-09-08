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

/** Deletions retain identities for diagnostics and external references. */
export function removeScriptVariant(
  scene: Scene,
  beatId: string,
  slotId: string,
  variantId: string,
): Scene {
  const beat = scene.beats?.find((one) => one.id === beatId)
  const slot = beat?.dialogue?.find((one) => one.id === slotId)
  const variant = slot?.variants?.find((one) => one.id === variantId)
  if (!variant || slot?.policy === 'locked' || variant.text.lifecycle?.policy === 'locked')
    return scene
  return {
    ...scene,
    beats: scene.beats?.map((one) =>
      one.id === beatId
        ? {
            ...one,
            dialogue: one.dialogue?.map((line) =>
              line.id === slotId
                ? { ...line, variants: line.variants?.filter((item) => item.id !== variantId) }
                : line,
            ),
          }
        : one,
    ),
    tombstones: [
      ...(scene.tombstones ?? []),
      {
        target: { variant: variantId },
        label: variant.text.body || 'Empty variant',
        deleted_at: new Date().toISOString(),
      },
    ],
  }
}

export function removeScriptSlot(scene: Scene, beatId: string, slotId: string): Scene {
  const beat = scene.beats?.find((one) => one.id === beatId)
  const slot = beat?.dialogue?.find((one) => one.id === slotId)
  if (
    !slot ||
    slot.policy === 'locked' ||
    slot.variants?.some((variant) => variant.text.lifecycle?.policy === 'locked')
  )
    return scene
  const deleted_at = new Date().toISOString()
  return {
    ...scene,
    beats: scene.beats?.map((one) =>
      one.id === beatId
        ? { ...one, dialogue: one.dialogue?.filter((line) => line.id !== slotId) }
        : one,
    ),
    tombstones: [
      ...(scene.tombstones ?? []),
      {
        target: { dialogue_slot: slotId },
        label: `Dialogue in ${beat?.title ?? beatId}`,
        deleted_at,
      },
      ...(slot.variants ?? []).map((variant) => ({
        target: { variant: variant.id },
        label: variant.text.body || 'Empty variant',
        deleted_at,
      })),
    ],
  }
}
