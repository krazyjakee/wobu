import { useEffect, useRef, type RefObject } from 'react'
import { useUI } from '../../../store/ui'
import { flowNodeForTarget } from './canonicalFlow'
import { useFlowLevelApi } from './flowStore'
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
  const reveal = useUI((state) => state.narrativeReveal)
  const store = useFlowLevelApi()
  const honoured = useRef(0)
  useEffect(() => {
    if (
      !enabled ||
      !reveal ||
      reveal.seq === honoured.current ||
      reveal.sceneId !== scene.id ||
      (projectKey !== undefined && reveal.projectKey !== null && reveal.projectKey !== projectKey)
    )
      return
    if (reveal.origin === 'flow' && (!reveal.focus || projectKey === undefined)) {
      honoured.current = reveal.seq
      return
    }
    const root = container.current
    if (!root) return
    let frame = 0
    let centred = false
    const requested = flowNodeForTarget(reveal)
    const exact = scene.elements.find((element) => element.id === requested)
    const target =
      exact ??
      scene.elements.find(
        (element) => element.kind === 'beat' && element.beatId === reveal.beatId,
      ) ??
      scene.elements.find((element) => element.id === scene.entryId) ??
      scene.elements[0]
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
        honoured.current = reveal!.seq
        observer.disconnect()
        return
      }
      if (requested === null) {
        honoured.current = reveal!.seq
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
        element.focus({ preventScroll: true })
        if (document.activeElement !== element) return
      }
      if (!exact && requested)
        store
          .getState()
          .announce('The requested element is gone. Showing its surviving beat or scene instead.')
      honoured.current = reveal!.seq
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
  }, [scene, container, projectKey, graph, center, reveal, store, enabled])
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
