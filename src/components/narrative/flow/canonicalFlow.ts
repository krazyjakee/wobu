import type { Destination, Scene } from '../../../lib/api'
import type { NarrativeTarget } from '../../../store/ui'
import type { RouteRef, SceneEditOperation } from '../sceneEdits'
import type { FlowConnection } from './flowStore'
import type { FlowElement, FlowKind, FlowLevel } from './model'
import { narrativeIdOf, nodeId } from './source'

export interface CanonicalFlowActions {
  projectKey?: string
  disabled?: boolean
  connectionsDisabled?: boolean
  add: (kind: FlowKind, anchor: string | null) => boolean
  connect: (from: FlowConnection, to: string | null) => boolean
  remove: (id: string) => boolean
}
export function routeForNode(scene: Scene, node: string): RouteRef | null {
  const holder = node.startsWith('end:')
    ? node.slice(4)
    : node.startsWith('link:')
      ? node.slice(5)
      : node
  const parsed = narrativeIdOf(holder)
  if (!parsed || (parsed.kind !== 'choice' && parsed.kind !== 'outcome')) return null
  for (const beat of scene.beats ?? []) {
    if (
      (parsed.kind === 'choice' ? beat.choices : beat.outcomes)?.some(
        (route) => route.id === parsed.id,
      )
    )
      return { kind: parsed.kind, beatId: beat.id, id: parsed.id }
  }
  return null
}
export function flowTarget(level: FlowLevel, element: FlowElement | null): NarrativeTarget {
  const target: NarrativeTarget = { sceneId: level.id, beatId: element?.beatId ?? null }
  if (!element) return target
  const holder = element.id.replace(/^(end:|link:)/, '')
  const parsed = narrativeIdOf(holder)
  if (parsed?.kind === 'choice') target.choiceId = parsed.id
  if (parsed?.kind === 'outcome') target.outcomeId = parsed.id
  return target
}
export function flowNodeForTarget(target: NarrativeTarget): string | null {
  if (target.choiceId) return nodeId.choice(target.choiceId)
  if (target.outcomeId) return nodeId.outcome(target.outcomeId)
  return target.beatId ? nodeId.beat(target.beatId) : null
}
function destination(scene: Scene, node: string | null): Destination | null {
  if (node === null) return { unresolved: {} }
  const parsed = narrativeIdOf(node)
  if (parsed?.kind === 'beat' && scene.beats?.some((beat) => beat.id === parsed.id))
    return { beat: parsed.id }
  if (!node.startsWith('end:') && !node.startsWith('link:')) return null
  const ref = routeForNode(scene, node)
  const beat = scene.beats?.find((beat) => beat.id === ref?.beatId)
  return (
    (ref?.kind === 'choice' ? beat?.choices : beat?.outcomes)?.find((route) => route.id === ref?.id)
      ?.to ?? null
  )
}
/** Gestures resolve stable source IDs and emit the same typed operations as forms. */
export function canonicalFlowActions(
  scene: Scene,
  dispatch: (operation: SceneEditOperation) => boolean,
  refuse: (message: string) => void,
  options: { projectKey?: string; disabled?: boolean } = {},
): CanonicalFlowActions {
  const denied = (message: string) => {
    refuse(message)
    return false
  }
  return {
    ...options,
    add: (kind, anchor) => {
      const beatId = anchor
        ? (routeForNode(scene, anchor)?.beatId ?? narrativeIdOf(anchor)?.id)
        : undefined
      const beat = scene.beats?.find((one) => one.id === beatId) ?? scene.beats?.[0]
      if (kind === 'beat')
        return dispatch({ kind: 'addBeat', title: 'New beat', afterId: beat?.id })
      if (!beat) return denied('Add a beat before adding one of its routes.')
      if (kind === 'choice')
        return dispatch({
          kind: 'addChoice',
          beatId: beat.id,
          label: 'New choice',
          to: { unresolved: {} },
        })
      if (kind === 'outcome' || kind === 'condition' || kind === 'end')
        return dispatch({
          kind: 'addOutcome',
          beatId: beat.id,
          to: kind === 'end' ? { end: {} } : { unresolved: {} },
          ...(kind === 'condition' ? { condition: 'always' as const } : {}),
        })
      return denied('This element is authored outside the scene canvas.')
    },
    connect: (from, to) => {
      const next = destination(scene, to)
      if (!next) return denied('Connect to a beat, an explicit ending, or a scene link.')
      const ref = routeForNode(scene, from.elementId)
      if (ref) return dispatch({ kind: 'setDestination', route: ref, value: next })
      const parsed = narrativeIdOf(from.elementId)
      if (parsed?.kind === 'beat' && from.portId === 'outcome:new')
        return dispatch({ kind: 'addOutcome', beatId: parsed.id, to: next })
      return denied(
        'A beat owns its choices and outcomes. Change the wire leaving the route instead.',
      )
    },
    remove: (node) => {
      const ref = routeForNode(scene, node)
      if (ref)
        return dispatch(
          node.startsWith('end:') || node.startsWith('link:')
            ? { kind: 'setDestination', route: ref, value: { unresolved: {} } }
            : { kind: 'removeRoute', route: ref },
        )
      const parsed = narrativeIdOf(node)
      if (parsed?.kind === 'beat') return dispatch({ kind: 'removeBeat', beatId: parsed.id })
      return denied('Use the arrangement controls to remove a presentation group or note.')
    },
  }
}
