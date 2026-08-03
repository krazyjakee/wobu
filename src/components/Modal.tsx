import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { setModalDepth } from '../lib/modalStack'

const FOCUSABLE = [
  'a[href]',
  'button:not([disabled])',
  'input:not([type="hidden"]):not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[contenteditable="true"]',
  '[tabindex]:not([tabindex="-1"])',
].join(',')

interface ModalEntry {
  host: HTMLDivElement
  surface: RefObject<HTMLElement | null>
  restoreTarget: HTMLElement | null
}

interface InertState {
  inert: boolean
  ariaHidden: string | null
}

const stack: ModalEntry[] = []
const inertBeforeModal = new Map<HTMLElement, InertState>()
let bodyObserver: MutationObserver | null = null

function restoreElement(element: HTMLElement) {
  const previous = inertBeforeModal.get(element)
  if (!previous) return
  if (previous.inert) element.setAttribute('inert', '')
  else element.removeAttribute('inert')
  if (previous.ariaHidden === null) element.removeAttribute('aria-hidden')
  else element.setAttribute('aria-hidden', previous.ariaHidden)
  inertBeforeModal.delete(element)
}

function syncBackground() {
  // Published for the keyboard dispatcher, which has to know whether the
  // workspace is reachable before it runs a shortcut against it. Set here
  // rather than at the two call sites so it cannot fall out of step with the
  // stack: everything that changes the stack ends by calling this.
  setModalDepth(stack.length)

  // A background child can unmount while a nested modal is still open. Do not
  // retain detached nodes in the process-long restoration map.
  for (const element of inertBeforeModal.keys()) {
    if (!element.isConnected) inertBeforeModal.delete(element)
  }

  const top = stack.at(-1)?.host ?? null
  for (const child of Array.from(document.body.children)) {
    if (!(child instanceof HTMLElement)) continue
    if (child === top) {
      restoreElement(child)
      continue
    }
    if (!top) {
      restoreElement(child)
      continue
    }
    if (!inertBeforeModal.has(child)) {
      inertBeforeModal.set(child, {
        inert: child.hasAttribute('inert'),
        ariaHidden: child.getAttribute('aria-hidden'),
      })
    }
    child.setAttribute('inert', '')
    child.setAttribute('aria-hidden', 'true')
  }

  if (top && !bodyObserver) {
    bodyObserver = new MutationObserver(syncBackground)
    bodyObserver.observe(document.body, { childList: true })
  } else if (!top && bodyObserver) {
    bodyObserver.disconnect()
    bodyObserver = null
  }
}

function focusableWithin(surface: HTMLElement): HTMLElement[] {
  return Array.from(surface.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (element) => !element.closest('[inert]') && element.getAttribute('aria-hidden') !== 'true',
  )
}

function focusInitial(entry: ModalEntry, requested?: RefObject<HTMLElement | null>) {
  const surface = entry.surface.current
  if (!surface) return
  const requestedElement = requested?.current
  const target =
    (requestedElement && surface.contains(requestedElement) ? requestedElement : null) ??
    surface.querySelector<HTMLElement>('[data-modal-initial-focus]') ??
    focusableWithin(surface)[0] ??
    surface
  target.focus()
}

export function Modal({
  children,
  onClose,
  busy = false,
  busyMessage,
  titleId,
  descriptionId,
  role = 'dialog',
  className = 'sheet',
  scrimClassName = '',
  initialFocus,
  closeOnBackdrop = true,
}: {
  children: ReactNode
  onClose: () => void
  busy?: boolean
  /** Visible reason Escape/backdrop dismissal is temporarily unavailable. */
  busyMessage?: string
  titleId: string
  descriptionId: string
  role?: 'dialog' | 'alertdialog'
  className?: string
  scrimClassName?: string
  initialFocus?: RefObject<HTMLElement | null>
  closeOnBackdrop?: boolean
}) {
  const [host] = useState(() => {
    const element = document.createElement('div')
    element.dataset.modalHost = ''
    return element
  })
  const surface = useRef<HTMLDivElement>(null)
  const closeRef = useRef(onClose)
  const busyRef = useRef(busy)

  useEffect(() => {
    closeRef.current = onClose
    busyRef.current = busy
  }, [busy, onClose])

  useLayoutEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const entry: ModalEntry = { host, surface, restoreTarget: opener }
    document.body.append(host)
    stack.push(entry)
    syncBackground()
    focusInitial(entry, initialFocus)

    const onKeyDown = (event: KeyboardEvent) => {
      if (stack.at(-1) !== entry) return
      if (event.key === 'Escape') {
        event.preventDefault()
        event.stopPropagation()
        if (!busyRef.current) closeRef.current()
        return
      }
      if (event.key !== 'Tab') return
      const currentSurface = surface.current
      if (!currentSurface) return
      const focusable = focusableWithin(currentSurface)
      if (focusable.length === 0) {
        event.preventDefault()
        currentSurface.focus()
        return
      }
      const active = document.activeElement
      const current = active instanceof HTMLElement ? focusable.indexOf(active) : -1
      const next = event.shiftKey
        ? current <= 0
          ? focusable[focusable.length - 1]
          : focusable[current - 1]
        : current < 0 || current === focusable.length - 1
          ? focusable[0]
          : focusable[current + 1]
      event.preventDefault()
      next?.focus()
    }
    document.addEventListener('keydown', onKeyDown, true)

    return () => {
      document.removeEventListener('keydown', onKeyDown, true)
      const index = stack.indexOf(entry)
      const wasTop = index === stack.length - 1
      if (index >= 0) {
        // If a parent closes around a nested async dialog, carry its outside
        // restoration target forward instead of focusing a detached opener.
        for (const nested of stack.slice(index + 1)) {
          if (nested.restoreTarget && host.contains(nested.restoreTarget)) {
            nested.restoreTarget = entry.restoreTarget
          }
        }
        stack.splice(index, 1)
      }
      host.remove()
      syncBackground()

      if (!wasTop) return
      const target = entry.restoreTarget
      if (target?.isConnected && !target.closest('[inert]')) target.focus()
      else {
        const revealed = stack.at(-1)
        if (revealed) focusInitial(revealed)
      }
    }
  }, [host, initialFocus])

  return createPortal(
    <div
      className={`scrim${scrimClassName ? ` ${scrimClassName}` : ''}`}
      onMouseDown={(event) => {
        if (
          closeOnBackdrop &&
          !busy &&
          event.target === event.currentTarget &&
          stack.at(-1)?.host === host
        ) {
          onClose()
        }
      }}
    >
      <div
        ref={surface}
        className={className}
        role={role}
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
        aria-busy={busy || undefined}
        tabIndex={-1}
      >
        {children}
        {busy && busyMessage && (
          <p className="sheet-busy" aria-live="polite">
            {busyMessage}
          </p>
        )}
      </div>
    </div>,
    host,
  )
}
