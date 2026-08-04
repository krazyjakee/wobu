import { useCallback, useEffect, useState } from 'react'
import { isOverflowing } from '../lib/tooltip'

type ObserverCtor = new (callback: () => void) => { observe(el: Element): void; disconnect(): void }

/**
 * Whether an element is currently clipping its text.
 *
 * Measured rather than assumed. A label is truncated at one window width and
 * not at another, so this re-measures when the element resizes — a `title` that
 * is always there is the thing this replaces, not the thing it copies.
 *
 * `ResizeObserver` when the runtime has one (every browser Wobu ships on), a
 * window `resize` listener when it does not (jsdom, so the tests exercise the
 * fallback path rather than a mock of the fast one).
 */
export function useTruncated<T extends HTMLElement>(
  text: string,
): [(node: T | null) => void, boolean] {
  const [node, setNode] = useState<T | null>(null)
  const [truncated, setTruncated] = useState(false)
  const ref = useCallback((next: T | null) => setNode(next), [])

  useEffect(() => {
    if (!node) return
    const measure = () => setTruncated(isOverflowing(node))
    measure()
    const Observer = (globalThis as { ResizeObserver?: ObserverCtor }).ResizeObserver
    if (!Observer) {
      window.addEventListener('resize', measure)
      return () => window.removeEventListener('resize', measure)
    }
    const observer = new Observer(measure)
    observer.observe(node)
    return () => observer.disconnect()
  }, [node, text])

  // Read through the element rather than reset when it goes: an unmounted
  // element is not truncated, and clearing the state in the effect would be one
  // more render on every mount.
  return [ref, node !== null && truncated]
}
