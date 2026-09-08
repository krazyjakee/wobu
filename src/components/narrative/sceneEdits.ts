import type { Beat, Condition, Destination, Effect, Scene, Speaker } from '../../lib/api'
import type { NarrativeTarget } from '../../store/ui'
import { beatHasLockedText, duplicateBeat } from './sceneEditPrimitives'
import { mintId } from './sceneIdentity'

export interface RouteRef {
  kind: 'choice' | 'outcome'
  beatId: string
  id: string
}
interface VariantRef {
  kind: 'variant'
  beatId: string
  slotId: string
  id: string
}
export type SceneEditOperation =
  | { kind: 'addBeat'; title: string; afterId?: string }
  | { kind: 'removeBeat' | 'duplicateBeat'; beatId: string }
  | { kind: 'moveBeat'; beatId: string; index: number }
  | {
      kind: 'patchBeat'
      beatId: string
      changes: Partial<Pick<Beat, 'title' | 'intents' | 'must_convey' | 'must_not_reveal'>>
    }
  | {
      kind: 'patchScene'
      changes: Partial<
        Pick<Scene, 'name' | 'summary' | 'participants' | 'act_id' | 'arc_id' | 'tag_ids'>
      >
    }
  | {
      kind: 'addChoice'
      beatId: string
      label: string
      to: Destination
      condition?: Condition
      effects?: Effect[]
    }
  | {
      kind: 'addOutcome'
      beatId: string
      to: Destination
      condition?: Condition
      effects?: Effect[]
    }
  | { kind: 'removeRoute'; route: RouteRef }
  | { kind: 'moveRoute'; route: RouteRef; index: number }
  | { kind: 'setDestination'; route: RouteRef; value: Destination }
  | {
      kind: 'setCondition'
      owner: RouteRef | VariantRef | { kind: 'scene' }
      value: Condition | undefined
    }
  | { kind: 'setEffects'; route: RouteRef; value: Effect[] | undefined }
  | { kind: 'setChoiceLabel'; beatId: string; choiceId: string; value: string }
  | { kind: 'removeSlot'; beatId: string; slotId: string }
  | { kind: 'removeVariant'; beatId: string; slotId: string; variantId: string }
  | { kind: 'setSpeaker'; beatId: string; slotId: string; value: Speaker }
  | { kind: 'addSlot'; beatId: string; speaker: Speaker }
  | { kind: 'addVariant'; beatId: string; slotId: string; body?: string }
  | { kind: 'moveSlot'; beatId: string; slotId: string; index: number }
  | { kind: 'moveVariant'; owner: VariantRef; index: number }
  | { kind: 'setVariantBody'; owner: VariantRef; value: string }

export type SceneEditTarget = NarrativeTarget & { sceneId: string }
export type SceneEditResult =
  | {
      scene: Scene
      changed: boolean
      created: SceneEditTarget[]
      removed: SceneEditTarget[]
      target: SceneEditTarget
    }
  | { refused: string }
export interface SceneEditEnvironment {
  mintId: () => string
  now: () => string
}
const DEFAULT_ENV: SceneEditEnvironment = { mintId, now: () => new Date().toISOString() }
class Refusal extends Error {}

/**
 * One canonical source operation for Script, Flow and outline. No layout or
 * display strings enter this reducer. Typed fields are copied in full; existing
 * prose, provenance, revisions, metadata and unknown future fields survive.
 *
 * This is authoring preflight, not permission authority. Save still goes through
 * the guarded backend. Review, lock and freshness changes use their own commands.
 */
export function applySceneEdit(
  original: Scene,
  operation: SceneEditOperation,
  environment: SceneEditEnvironment = DEFAULT_ENV,
): SceneEditResult {
  const scene = structuredClone(original)
  const created: SceneEditTarget[] = []
  const removed: SceneEditTarget[] = []
  let target: SceneEditTarget = { sceneId: scene.id }
  const allIds = new Set<string>([scene.id])
  for (const beat of scene.beats ?? []) {
    allIds.add(beat.id)
    for (const child of [
      ...(beat.choices ?? []),
      ...(beat.outcomes ?? []),
      ...(beat.dialogue ?? []),
    ])
      allIds.add(child.id)
    for (const slot of beat.dialogue ?? [])
      for (const variant of slot.variants ?? []) allIds.add(variant.id)
  }
  for (const tombstone of scene.tombstones ?? [])
    for (const id of Object.values(tombstone.target)) allIds.add(id)
  const freshId = () => {
    const id = environment.mintId()
    if (allIds.has(id))
      throw new Refusal('A new narrative identity already exists. Try the operation again.')
    allIds.add(id)
    return id
  }
  const beatAt = (id: string) => {
    const beat = scene.beats?.find((one) => one.id === id)
    if (!beat) throw new Refusal('This beat no longer exists. Select a surviving beat.')
    target = { sceneId: scene.id, beatId: beat.id }
    return beat
  }
  const routeAt = (ref: RouteRef) => {
    const beat = beatAt(ref.beatId)
    const route = (ref.kind === 'choice' ? beat.choices : beat.outcomes)?.find(
      (one) => one.id === ref.id,
    )
    if (!route) throw new Refusal('This route no longer exists. Select a surviving route.')
    target = {
      ...target,
      ...(ref.kind === 'choice' ? { choiceId: ref.id } : { outcomeId: ref.id }),
    }
    return route
  }
  const slotAt = (beatId: string, slotId: string) => {
    const beat = beatAt(beatId)
    const slot = beat.dialogue?.find((one) => one.id === slotId)
    if (!slot) throw new Refusal('This dialogue slot no longer exists.')
    target = { ...target, lineId: slotId }
    return { beat, slot }
  }
  const guardSlot = (slot: NonNullable<Beat['dialogue']>[number], allVariants: boolean) => {
    if (
      slot.policy === 'locked' ||
      (allVariants && slot.variants?.some((one) => one.text.lifecycle?.policy === 'locked'))
    ) {
      throw new Refusal('Unlock protected dialogue through Review before changing or deleting it.')
    }
  }
  const retireSlot = (
    beatId: string,
    slot: NonNullable<Beat['dialogue']>[number],
    label: string,
    at: string,
  ) => {
    scene.tombstones = [
      ...(scene.tombstones ?? []),
      { target: { dialogue_slot: slot.id }, label, deleted_at: at },
    ]
    removed.push({ sceneId: scene.id, beatId, lineId: slot.id })
    for (const variant of slot.variants ?? []) {
      scene.tombstones.push({
        target: { variant: variant.id },
        label: variant.text.body || 'Empty variant',
        deleted_at: at,
      })
      removed.push({ sceneId: scene.id, beatId, lineId: slot.id, variantId: variant.id })
    }
  }
  try {
    switch (operation.kind) {
      case 'addBeat': {
        const beats = scene.beats ?? []
        const index =
          operation.afterId === undefined
            ? beats.length
            : beats.findIndex((beat) => beat.id === operation.afterId) + 1
        if (operation.afterId !== undefined && index === 0)
          throw new Refusal('The insertion beat no longer exists.')
        const beat = { id: freshId(), title: operation.title }
        beats.splice(index, 0, beat)
        scene.beats = beats
        target = { sceneId: scene.id, beatId: beat.id }
        created.push(target)
        break
      }
      case 'removeBeat': {
        const beat = beatAt(operation.beatId)
        if (beatHasLockedText(beat))
          throw new Refusal('Unlock protected dialogue through Review before deleting this beat.')
        const index = scene.beats!.indexOf(beat)
        scene.beats!.splice(index, 1)
        removed.push(target)
        const at = environment.now()
        scene.tombstones = [
          ...(scene.tombstones ?? []),
          { target: { beat: beat.id }, label: beat.title, deleted_at: at },
        ]
        for (const slot of beat.dialogue ?? []) retireSlot(beat.id, slot, beat.title, at)
        for (const choice of beat.choices ?? [])
          removed.push({ sceneId: scene.id, beatId: beat.id, choiceId: choice.id })
        for (const outcome of beat.outcomes ?? [])
          removed.push({ sceneId: scene.id, beatId: beat.id, outcomeId: outcome.id })
        target = {
          sceneId: scene.id,
          beatId: scene.beats![Math.min(index, scene.beats!.length - 1)]?.id ?? null,
        }
        break
      }
      case 'duplicateBeat': {
        const beat = beatAt(operation.beatId)
        const copy = duplicateBeat(beat, freshId)
        scene.beats!.splice(scene.beats!.indexOf(beat) + 1, 0, copy)
        target = { sceneId: scene.id, beatId: copy.id }
        created.push(target)
        for (const slot of copy.dialogue ?? []) {
          created.push({ ...target, lineId: slot.id })
          for (const variant of slot.variants ?? [])
            created.push({ ...target, lineId: slot.id, variantId: variant.id })
        }
        for (const choice of copy.choices ?? []) created.push({ ...target, choiceId: choice.id })
        for (const outcome of copy.outcomes ?? [])
          created.push({ ...target, outcomeId: outcome.id })
        break
      }
      case 'moveBeat': {
        const beat = beatAt(operation.beatId)
        move(scene.beats!, beat, operation.index)
        break
      }
      case 'patchBeat':
        patch(beatAt(operation.beatId), operation.changes, [
          'title',
          'intents',
          'must_convey',
          'must_not_reveal',
        ])
        break
      case 'patchScene':
        patch(scene, operation.changes, [
          'name',
          'summary',
          'participants',
          'act_id',
          'arc_id',
          'tag_ids',
        ])
        break
      case 'addChoice':
      case 'addOutcome': {
        const beat = beatAt(operation.beatId)
        const id = freshId()
        const route = {
          id,
          to: structuredClone(operation.to),
          ...(operation.effects === undefined
            ? {}
            : { effects: structuredClone(operation.effects) }),
        }
        if (operation.kind === 'addChoice') {
          beat.choices = [
            ...(beat.choices ?? []),
            {
              ...route,
              label: operation.label,
              ...(operation.condition === undefined
                ? {}
                : { requires: structuredClone(operation.condition) }),
            },
          ]
          target = { ...target, choiceId: id }
        } else {
          beat.outcomes = [
            ...(beat.outcomes ?? []),
            {
              ...route,
              ...(operation.condition === undefined
                ? {}
                : { when: structuredClone(operation.condition) }),
            },
          ]
          target = { ...target, outcomeId: id }
        }
        created.push(target)
        break
      }
      case 'removeRoute': {
        const route = routeAt(operation.route)
        const beat = scene.beats!.find((one) => one.id === operation.route.beatId)!
        removed.push(target)
        if (operation.route.kind === 'choice')
          beat.choices = beat.choices!.filter((one) => one.id !== route.id)
        else beat.outcomes = beat.outcomes!.filter((one) => one.id !== route.id)
        target = { sceneId: scene.id, beatId: beat.id }
        break
      }
      case 'moveRoute': {
        const route = routeAt(operation.route)
        const beat = scene.beats!.find((one) => one.id === operation.route.beatId)!
        move(
          operation.route.kind === 'choice' ? beat.choices! : beat.outcomes!,
          route,
          operation.index,
        )
        break
      }
      case 'setDestination':
        routeAt(operation.route).to = structuredClone(operation.value)
        target.field = 'destination'
        break
      case 'setEffects':
        optional(routeAt(operation.route), 'effects', operation.value)
        target.field = 'effects'
        break
      case 'setChoiceLabel': {
        const beat = beatAt(operation.beatId)
        const choice = beat.choices?.find((one) => one.id === operation.choiceId)
        if (!choice) throw new Refusal('This choice no longer exists.')
        choice.label = operation.value
        target = { ...target, choiceId: choice.id, field: 'label' }
        break
      }
      case 'setCondition': {
        const owner = operation.owner
        if (owner.kind === 'scene') optional(scene, 'entry', operation.value)
        else if (owner.kind === 'variant') {
          const { slot } = slotAt(owner.beatId, owner.slotId)
          guardSlot(slot, false)
          const variant = slot.variants?.find((one) => one.id === owner.id)
          if (!variant) throw new Refusal('This variant no longer exists.')
          if (variant.text.lifecycle?.policy === 'locked')
            throw new Refusal('Unlock this variant through Review before changing its condition.')
          optional(variant, 'when', operation.value)
          target = { ...target, variantId: variant.id }
        } else {
          const route = routeAt(owner)
          if (owner.kind === 'choice') optional(route, 'requires', operation.value)
          else optional(route, 'when', operation.value)
        }
        target.field = owner.kind === 'scene' ? 'entry' : 'condition'
        break
      }
      case 'removeSlot': {
        const { beat, slot } = slotAt(operation.beatId, operation.slotId)
        guardSlot(slot, true)
        beat.dialogue = beat.dialogue!.filter((one) => one.id !== slot.id)
        retireSlot(beat.id, slot, `Dialogue in ${beat.title}`, environment.now())
        target = { sceneId: scene.id, beatId: beat.id }
        break
      }
      case 'removeVariant': {
        const { slot } = slotAt(operation.beatId, operation.slotId)
        guardSlot(slot, false)
        const variant = slot.variants?.find((one) => one.id === operation.variantId)
        if (!variant) throw new Refusal('This variant no longer exists.')
        if (variant.text.lifecycle?.policy === 'locked')
          throw new Refusal('Unlock this variant through Review before deleting it.')
        slot.variants = slot.variants!.filter((one) => one.id !== variant.id)
        scene.tombstones = [
          ...(scene.tombstones ?? []),
          {
            target: { variant: variant.id },
            label: variant.text.body || 'Empty variant',
            deleted_at: environment.now(),
          },
        ]
        removed.push({ ...target, variantId: variant.id })
        break
      }
      case 'addSlot': {
        const beat = beatAt(operation.beatId)
        const slot = {
          id: freshId(),
          speaker: structuredClone(operation.speaker),
          policy: 'edited' as const,
        }
        beat.dialogue = [...(beat.dialogue ?? []), slot]
        target = { ...target, lineId: slot.id }
        created.push(target)
        break
      }
      case 'addVariant': {
        const { slot } = slotAt(operation.beatId, operation.slotId)
        guardSlot(slot, true)
        const variant = {
          id: freshId(),
          text: {
            revision: '',
            body: operation.body ?? '',
            provenance: 'human' as const,
            lifecycle: { review: 'draft' as const, freshness: 'current' as const },
          },
        }
        slot.variants = [...(slot.variants ?? []), variant]
        target = { ...target, variantId: variant.id }
        created.push(target)
        break
      }
      case 'moveSlot': {
        const { beat, slot } = slotAt(operation.beatId, operation.slotId)
        move(beat.dialogue!, slot, operation.index)
        break
      }
      case 'moveVariant':
      case 'setVariantBody': {
        const { owner } = operation
        const { slot } = slotAt(owner.beatId, owner.slotId)
        guardSlot(slot, false)
        const variant = slot.variants?.find((one) => one.id === owner.id)
        if (!variant) throw new Refusal('This variant no longer exists.')
        if (variant.text.lifecycle?.policy === 'locked')
          throw new Refusal('Unlock this variant through Review before editing it.')
        if (operation.kind === 'moveVariant') move(slot.variants!, variant, operation.index)
        else variant.text.body = operation.value
        target = { ...target, variantId: variant.id, field: 'text' }
        break
      }
      case 'setSpeaker': {
        const { slot } = slotAt(operation.beatId, operation.slotId)
        guardSlot(slot, true)
        slot.speaker = structuredClone(operation.value)
        target.field = 'speaker'
        break
      }
    }
  } catch (error) {
    if (error instanceof Refusal) return { refused: error.message }
    throw error
  }
  const changed = JSON.stringify(original) !== JSON.stringify(scene)
  return { scene: changed ? scene : original, changed, created, removed, target }
}

function move<T>(items: T[], item: T, index: number) {
  if (!Number.isInteger(index) || index < 0 || index >= items.length)
    throw new Refusal('Choose a position within the current list.')
  items.splice(items.indexOf(item), 1)
  items.splice(index, 0, item)
}
function optional<T extends object>(object: T, key: string, value: unknown) {
  if (value === undefined) Reflect.deleteProperty(object, key)
  else Reflect.set(object, key, structuredClone(value))
}
function patch<T extends object>(object: T, changes: Partial<T>, allowed: string[]) {
  for (const key of Object.keys(changes)) {
    if (!allowed.includes(key))
      throw new Refusal('This field needs its dedicated authoring or Review operation.')
    optional(object, key, Reflect.get(changes, key))
  }
}
