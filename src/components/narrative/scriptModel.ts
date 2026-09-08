import type { Beat, Scene, Speaker } from '../../lib/api'
import { applySceneEdit, type SceneEditOperation } from './sceneEdits'
import { duplicateBeat } from './sceneEditPrimitives'
import { mintId } from './sceneIdentity'

export { beatHasLockedText } from './sceneEditPrimitives'

/** Legacy form adapters; every deletion uses the same canonical operation. */
function edit(scene: Scene, operation: SceneEditOperation): Scene {
  const result = applySceneEdit(scene, operation)
  return 'refused' in result ? scene : result.scene
}
export function removeScriptBeat(scene: Scene, beatId: string): Scene {
  return edit(scene, { kind: 'removeBeat', beatId })
}
export function duplicateScriptBeat(beat: Beat): Beat {
  return duplicateBeat(beat, mintId)
}
export function speakerKey(speaker: Speaker): string {
  return typeof speaker === 'string' ? speaker : speaker.entity
}
export function speakerFromKey(key: string): Speaker {
  return key === 'player' || key === 'narrator' ? key : { entity: key }
}
export function removeScriptVariant(
  scene: Scene,
  beatId: string,
  slotId: string,
  variantId: string,
): Scene {
  return edit(scene, { kind: 'removeVariant', beatId, slotId, variantId })
}
export function removeScriptSlot(scene: Scene, beatId: string, slotId: string): Scene {
  return edit(scene, { kind: 'removeSlot', beatId, slotId })
}
