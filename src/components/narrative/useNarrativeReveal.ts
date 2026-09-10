import { useEffect, useRef, type RefObject } from 'react'
import { useUI } from '../../store/ui'
import { narrativeField } from './scriptDiagnosticField'

/** A reveal is acknowledged only after its actual mounted field becomes visible. */
export function useNarrativeReveal(
  root: RefObject<HTMLElement | null>,
  projectKey: string,
  sceneId: string,
  fieldsOnly = false,
) {
  const reveal = useUI((state) => state.narrativeReveal)
  const acknowledged = useRef<number | null>(null)
  useEffect(() => {
    if (
      !reveal ||
      (fieldsOnly && reveal.field === null) ||
      (reveal.origin === 'script' && reveal.field === null) ||
      reveal.seq === acknowledged.current ||
      reveal.sceneId !== sceneId ||
      (reveal.projectKey !== null && reveal.projectKey !== projectKey)
    )
      return
    const container = root.current
    if (!container) return
    let frame = 0
    const observer = new MutationObserver(() => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(attempt)
    })
    function attempt() {
      const marker = Array.from(
        container!.querySelectorAll<HTMLElement>('[data-narrative-field]'),
      ).find((element) => element.dataset.narrativeField === narrativeField(reveal!))
      const control = marker?.matches('input, select, textarea, button')
        ? marker
        : marker?.querySelector<HTMLElement>('input, select, textarea, button')
      if (!control) return
      for (let element: HTMLElement | null = control; element; element = element.parentElement) {
        const style = getComputedStyle(element)
        if (
          element.hidden ||
          element.getAttribute('aria-hidden') === 'true' ||
          style.display === 'none' ||
          style.visibility === 'hidden'
        )
          return
      }
      if (reveal!.focus && control.matches(':disabled')) return
      control.scrollIntoView?.({ block: 'center' })
      if (reveal!.focus) {
        control.focus({ preventScroll: true })
        if (document.activeElement !== control) return
      }
      acknowledged.current = reveal!.seq
      observer.disconnect()
    }
    observer.observe(container, { subtree: true, childList: true, attributes: true })
    for (let parent = container.parentElement; parent; parent = parent.parentElement)
      observer.observe(parent, {
        attributes: true,
        attributeFilter: ['hidden', 'aria-hidden', 'style', 'class', 'disabled'],
      })
    attempt()
    return () => {
      observer.disconnect()
      cancelAnimationFrame(frame)
    }
  }, [projectKey, sceneId, reveal, root, fieldsOnly])
}
