import { useEffect } from 'react'
import { useUI } from '../store/ui'

export function Toasts() {
  const toasts = useUI((s) => s.toasts)
  const drop = useUI((s) => s.dropToast)

  useEffect(() => {
    if (!toasts.length) return
    const timers = toasts.map((t) => window.setTimeout(() => drop(t.id), 4200))
    return () => timers.forEach(window.clearTimeout)
  }, [toasts, drop])

  if (!toasts.length) return null
  return (
    <div className="toast">
      {toasts.map((t) => (
        <div key={t.id} className={t.kind === 'error' ? 'toast-item err' : 'toast-item'}>
          {t.text}
        </div>
      ))}
    </div>
  )
}
