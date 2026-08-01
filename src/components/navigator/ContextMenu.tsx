import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode, RefObject } from 'react'

export function ContextMenu({
  x,
  y,
  onClose,
  children,
  className,
  restoreFocus,
  restoreFocusRef,
  label,
}: {
  x?: number
  y?: number
  onClose: () => void
  children: ReactNode
  className?: string
  restoreFocus?: HTMLElement | null
  restoreFocusRef?: RefObject<HTMLElement | null>
  label?: string
}) {
  const ref = useRef<HTMLDivElement>(null)
  const opener = useRef<HTMLElement | null>(
    restoreFocus ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null),
  )
  const [pos, setPos] = useState(() =>
    x === undefined || y === undefined ? undefined : { left: x, top: y },
  )

  useLayoutEffect(() => {
    const el = ref.current
    if (!el || x === undefined || y === undefined) return
    const r = el.getBoundingClientRect()
    setPos({
      left: Math.max(6, Math.min(x, window.innerWidth - r.width - 6)),
      top: Math.max(6, Math.min(y, window.innerHeight - r.height - 6)),
    })
  }, [x, y])

  useLayoutEffect(() => {
    const menu = ref.current
    const items = menuItems(menu)
    for (const item of items) item.tabIndex = -1
    const first = items[0]
    const restoreTarget = restoreFocusRef?.current ?? opener.current
    ;(first ?? menu)?.focus()

    return () => {
      if (restoreTarget?.isConnected) restoreTarget.focus()
    }
  }, [restoreFocusRef])

  useEffect(() => {
    const down = (e: MouseEvent) => {
      const target = e.target as Node
      const trigger = restoreFocusRef?.current ?? opener.current
      if (!ref.current?.contains(target) && !trigger?.contains(target)) onClose()
    }
    window.addEventListener('mousedown', down)
    window.addEventListener('resize', onClose)
    return () => {
      window.removeEventListener('mousedown', down)
      window.removeEventListener('resize', onClose)
    }
  }, [onClose, restoreFocusRef])

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const items = menuItems(ref.current)
    const active = document.activeElement
    const index = items.findIndex((item) => item === active)
    let next: HTMLElement | undefined

    switch (event.key) {
      case 'ArrowDown':
        next = items[(index + 1 + items.length) % items.length]
        break
      case 'ArrowUp':
        next = items[(index - 1 + items.length) % items.length]
        break
      case 'Home':
        next = items[0]
        break
      case 'End':
        next = items.at(-1)
        break
      case 'Escape':
        event.preventDefault()
        event.stopPropagation()
        onClose()
        return
      default:
        return
    }

    if (next) {
      event.preventDefault()
      event.stopPropagation()
      next.focus()
    }
  }

  return (
    <div
      ref={ref}
      className={className ? `ctx ${className}` : 'ctx'}
      style={pos}
      role="menu"
      aria-label={label}
      tabIndex={-1}
      onKeyDown={onKeyDown}
    >
      {children}
    </div>
  )
}

function menuItems(menu: HTMLElement | null): HTMLElement[] {
  if (!menu) return []
  return Array.from(
    menu.querySelectorAll<HTMLElement>(
      '[role="menuitem"]:not([disabled]):not([aria-disabled="true"])',
    ),
  )
}
