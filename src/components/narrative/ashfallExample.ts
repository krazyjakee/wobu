import source from '../../../examples/narrative/ashfall-council/source.json'
import type { Scene, StateDocument, VariableDecl } from '../../lib/api'
import { mintId } from './sceneIdentity'

/** One original, handwritten fixture also used by the real-file command tests. */
export function ashfallScene(sceneId: string, id = mintId): Scene {
  const scene = structuredClone(source.scene) as Scene
  const ids = new Map<string, string>([[scene.id, sceneId]])
  const remap = (value: string) => {
    if (!ids.has(value)) ids.set(value, id())
    return ids.get(value)!
  }
  scene.id = sceneId
  for (const beat of scene.beats ?? []) {
    beat.id = remap(beat.id)
    for (const slot of beat.dialogue ?? []) {
      slot.id = remap(slot.id)
      for (const variant of slot.variants ?? []) variant.id = remap(variant.id)
    }
    for (const route of [...(beat.choices ?? []), ...(beat.outcomes ?? [])]) {
      route.id = remap(route.id)
      if ('beat' in route.to) route.to = { beat: remap(route.to.beat) }
    }
  }
  return scene
}

/** Reuse compatible declarations; never replace another author's variable. */
export function ashfallState(document: StateDocument): StateDocument {
  const variables = structuredClone(source.state.variables) as VariableDecl[]
  for (const expected of variables) {
    const existing = document.variables.find((variable) => variable.name === expected.name)
    if (
      existing &&
      (!sameType(existing.type, expected.type) ||
        (existing.owner ?? 'narrative') !== expected.owner)
    )
      throw new Error(`Variable ${expected.name} already exists with a different type or owner.`)
  }
  return {
    ...document,
    variables: [
      ...document.variables,
      ...variables.filter(
        (variable) => !document.variables.some((existing) => existing.name === variable.name),
      ),
    ],
  }
}

function sameType(left: VariableDecl['type'], right: VariableDecl['type']): boolean {
  if (left === 'bool' || right === 'bool') return left === right
  return (
    'int' in left &&
    'int' in right &&
    left.int.min === right.int.min &&
    left.int.max === right.int.max
  )
}
