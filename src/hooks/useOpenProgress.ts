import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { isTauri, type ScanProgress } from '../lib/api'

/**
 * How far through the first scan of a project we are.
 *
 * `null` when nothing is scanning. Only the *first* open of a project reports
 * anything: after that the index is warm and the backend only re-reads files
 * whose stamp moved, which is fast enough that a progress bar would flash and
 * vanish.
 *
 * This hook is mounted only by the in-flight scanning surface, so unmounting
 * that surface clears both its progress and its backend listener.
 */
export function useOpenProgress(): ScanProgress | null {
  const [progress, setProgress] = useState<ScanProgress | null>(null)

  useEffect(() => {
    if (!isTauri()) return

    let disposed = false
    let unlisten: (() => void) | undefined
    void listen<ScanProgress>('project:open-progress', (e) => {
      if (!disposed) setProgress(e.payload)
    })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* no backend to listen to; the spinner still runs */
      })

    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])

  return progress
}
