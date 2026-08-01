import { useEffect, useRef, useState } from 'react'
import type { UIEvent } from 'react'

/** Shared geometry for virtualized card grids; consumers retain their semantics. */
export function useVirtualCardWindow({
  count,
  tileMin,
  tileHeight,
  gap,
  overscan,
  initialWidth,
  initialHeight,
}: {
  count: number
  tileMin: number
  tileHeight: number
  gap: number
  overscan: number
  initialWidth: number
  initialHeight: number
}) {
  const viewportRef = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ width: initialWidth, height: initialHeight })
  const [scrollTop, setScrollTop] = useState(0)
  const hasItems = count > 0

  useEffect(() => {
    const element = viewportRef.current
    if (!element) return
    const measure = () =>
      setSize({ width: Math.max(1, element.clientWidth - 24), height: element.clientHeight })
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [hasItems])

  const columns = Math.max(1, Math.floor((size.width + gap) / (tileMin + gap)))
  const tileWidth = (size.width - gap * (columns - 1)) / columns
  const rows = Math.ceil(count / columns)
  const startRow = Math.max(0, Math.floor(scrollTop / tileHeight) - overscan)
  const endRow = Math.min(rows, Math.ceil((scrollTop + size.height) / tileHeight) + overscan)
  const start = startRow * columns
  const end = Math.min(count, endRow * columns)

  return {
    viewportRef,
    start,
    end,
    tileWidth,
    totalHeight: rows * tileHeight,
    onScroll: (event: UIEvent<HTMLDivElement>) => setScrollTop(event.currentTarget.scrollTop),
    position(index: number) {
      const row = Math.floor(index / columns)
      const column = index % columns
      return { top: row * tileHeight, left: column * (tileWidth + gap) }
    },
  }
}
