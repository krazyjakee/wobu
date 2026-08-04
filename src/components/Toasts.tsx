import { useEffect, useRef, useState } from 'react'
import { useUI, type Toast } from '../store/ui'

export function Toasts() {
  const toasts = useUI((s) => s.toasts)
  const drop = useUI((s) => s.dropToast)

  if (!toasts.length) return null
  return (
    <div className="toast" aria-label="Messages">
      {toasts.map((t) => (
        <ToastRow key={t.id} toast={t} onDismiss={() => drop(t.id)} />
      ))}
    </div>
  )
}

/**
 * Each row owns its live region and timer. Stable keyed rows mean adding or
 * removing a sibling does not mutate an already-announced live region, while
 * an intentional update to one row is announced exactly once.
 */
function ToastRow({ toast, onDismiss }: { toast: Toast; onDismiss: () => void }) {
  const [hovered, setHovered] = useState(false)
  const [focusWithin, setFocusWithin] = useState(false)
  const [detailOpen, setDetailOpen] = useState(false)
  const remaining = useRef(toast.durationMs)
  const armedRevision = useRef(-1)
  const paused = hovered || focusWithin

  useEffect(() => {
    if (toast.persistent || paused) return

    if (armedRevision.current !== toast.revision) {
      remaining.current = toast.durationMs
      armedRevision.current = toast.revision
    }

    const startedAt = Date.now()
    const timer = window.setTimeout(onDismiss, remaining.current)
    return () => {
      window.clearTimeout(timer)
      remaining.current = Math.max(0, remaining.current - (Date.now() - startedAt))
    }
  }, [onDismiss, paused, toast.durationMs, toast.persistent, toast.revision])

  const leaveFocus = (event: React.FocusEvent<HTMLDivElement>) => {
    if (!event.currentTarget.contains(event.relatedTarget)) setFocusWithin(false)
  }

  return (
    <div
      className={toast.kind === 'error' ? 'toast-item err' : 'toast-item'}
      role={toast.kind === 'error' ? 'alert' : 'status'}
      aria-live={toast.kind === 'error' ? 'assertive' : 'polite'}
      aria-atomic="true"
      onPointerEnter={() => setHovered(true)}
      onPointerLeave={() => setHovered(false)}
      onFocusCapture={() => setFocusWithin(true)}
      onBlurCapture={leaveFocus}
      onKeyDown={(event) => {
        if (event.key === 'Escape') onDismiss()
      }}
    >
      <div className="toast-body">
        <span className="toast-text">{toast.text}</span>
        {toast.detail && (
          <>
            <button
              className="toast-more"
              type="button"
              aria-expanded={detailOpen}
              onClick={() => setDetailOpen((open) => !open)}
            >
              {detailOpen ? 'Hide details' : 'Details'}
            </button>
            {detailOpen && <code className="toast-detail">{toast.detail}</code>}
          </>
        )}
      </div>
      {toast.action && (
        <button
          className="toast-action"
          type="button"
          onClick={() => {
            toast.action?.run()
            onDismiss()
          }}
        >
          {toast.action.label}
        </button>
      )}
      <button
        className="toast-dismiss"
        type="button"
        aria-label={`Dismiss message: ${toast.text}`}
        onClick={onDismiss}
      >
        <span aria-hidden="true">×</span>
      </button>
    </div>
  )
}
