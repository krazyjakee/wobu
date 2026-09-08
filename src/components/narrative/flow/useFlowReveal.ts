import { useEffect, useMemo, useRef, type RefObject } from 'react'
import { useUI, type NarrativeReveal } from '../../../store/ui'
import { flowNodeForTarget } from './canonicalFlow'
import { useFlowLevel, useFlowLevelApi } from './flowStore'
import type { FlowLevel } from './model'
import type { FlowGraph } from './graph'

/** Hidden tabs and collapsed groups do not consume pending reveal requests. */
export function useFlowReveal({
  scene,
  container,
  projectKey,
  graph,
  center,
  enabled = true,
}: {
  scene: FlowLevel
  container: RefObject<HTMLElement | null>
  projectKey?: string
  graph?: FlowGraph
  center?: (id: string) => void
  enabled?: boolean
}) {
  const globalReveal = useUI((state) => state.narrativeReveal)
  const local = useFlowLevel((state) => state.nodeReveal)
  const reveal = useMemo<NarrativeReveal | null>(
    () =>
      local
        ? {
            sceneId: scene.id,
            beatId: null,
            lineId: null,
            variantId: null,
            choiceId: null,
            outcomeId: null,
            field: null,
            seq: local.seq,
            origin: 'flow',
            projectKey: projectKey ?? null,
            focus: true,
          }
        : globalReveal,
    [local, scene.id, projectKey, globalReveal],
  )
  const store = useFlowLevelApi()
  const honoured = useRef(0)
  const sequence = reveal ? (local ? -reveal.seq : reveal.seq) : 0
  useEffect(() => {
    if (
      !enabled ||
      !reveal ||
      sequence === honoured.current ||
      reveal.sceneId !== scene.id ||
      (projectKey !== undefined && reveal.projectKey !== null && reveal.projectKey !== projectKey)
    )
      return
    if (!local && reveal.origin === 'flow' && (!reveal.focus || projectKey === undefined)) {
      honoured.current = sequence
      return
    }
    const root = container.current
    if (!root) return
    let frame = 0
    let centred = false
    const requested = local?.id ?? flowNodeForTarget(reveal)
    const exact = scene.elements.find((element) => element.id === requested)
    if (local && !exact) return
    const fallback =
      exact ??
      scene.elements.find(
        (element) => element.kind === 'beat' && element.beatId === reveal.beatId,
      ) ??
      scene.elements.find((element) => element.id === scene.entryId) ??
      scene.elements[0]
    const target =
      requested === null &&
      fallback?.groupId &&
      store.getState().closedGroups.includes(fallback.groupId)
        ? { id: fallback.groupId, groupId: null }
        : fallback
    const observer = new MutationObserver(() => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(attempt)
    })
    function attempt() {
      if (!visible(root!)) return
      if (!target) {
        const empty = root!.querySelector<HTMLButtonElement>('[data-flow-empty]')
        if (!empty || !visible(empty) || (reveal!.focus && empty.matches(':disabled'))) return
        if (reveal!.focus) {
          empty.focus({ preventScroll: true })
          if (document.activeElement !== empty) return
        }
        honoured.current = sequence
        observer.disconnect()
        return
      }
      if (requested === null && !reveal!.focus) {
        honoured.current = sequence
        observer.disconnect()
        return
      }
      store.getState().select(target!.id)
      if (!graph && target!.groupId && store.getState().closedGroups.includes(target!.groupId)) {
        store
          .getState()
          .setClosedGroups(store.getState().closedGroups.filter((id) => id !== target!.groupId))
        return
      }
      if (graph && !graph.nodes.some((node) => node.id === target!.id)) {
        if (target!.groupId)
          store
            .getState()
            .setClosedGroups(store.getState().closedGroups.filter((id) => id !== target!.groupId))
        return
      }
      if (!centred && center) {
        centered()
        return
      }
      const element = Array.from(
        root!.querySelectorAll<HTMLElement>('.react-flow__node[data-id], [data-flow-id]'),
      ).find((element) => (element.dataset.flowId ?? element.dataset.id) === target!.id)
      if (!element || !visible(element)) return
      if (!graph) element.scrollIntoView?.({ block: 'nearest' })
      if (reveal!.focus && !reveal!.field) {
        if (element.matches(':disabled')) return
        // At high UI scaling the surrounding panel may scroll independently
        // of the graph. Centre the node inside Flow, then expose the canvas
        // itself before restoring focus without moving its graph viewport.
        if (graph) root!.scrollIntoView?.({ block: 'nearest' })
        element.focus({ preventScroll: true })
        if (document.activeElement !== element) return
      }
      if (!exact && requested)
        store
          .getState()
          .announce('The requested element is gone. Showing its surviving beat or scene instead.')
      honoured.current = sequence
      if (local) store.getState().requestNodeReveal(null)
      observer.disconnect()
    }
    function centered() {
      centred = true
      center!(target!.id)
      frame = requestAnimationFrame(attempt)
    }
    observer.observe(root, { attributes: true, childList: true, subtree: true })
    for (let parent = root.parentElement; parent; parent = parent.parentElement)
      observer.observe(parent, {
        attributes: true,
        attributeFilter: ['hidden', 'aria-hidden', 'class', 'style', 'disabled', 'inert'],
      })
    attempt()
    return () => {
      observer.disconnect()
      cancelAnimationFrame(frame)
    }
  }, [scene, container, projectKey, graph, center, reveal, local, sequence, store, enabled])
}

function visible(target: HTMLElement) {
  for (let element: HTMLElement | null = target; element; element = element.parentElement) {
    const style = getComputedStyle(element)
    if (
      element.hidden ||
      element.hasAttribute('inert') ||
      element.getAttribute('aria-hidden') === 'true' ||
      style.display === 'none' ||
      style.visibility === 'hidden' ||
      style.contentVisibility === 'hidden'
    )
      return false
  }
  return true
}
