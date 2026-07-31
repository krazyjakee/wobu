import { useCallback, useEffect, useRef, useState } from 'react'
import type { WobuNode } from '../lib/api'
import { useUpsertNode } from '../lib/queries'
import { report } from '../store/ui'

export type SaveStatus = 'idle' | 'dirty' | 'saving' | 'saved' | 'error'

/**
 * Debounced writes through `node_upsert`. The queued patch is merged onto the
 * freshest node we hold, so a `world:changed` refetch mid-typing cannot resurrect
 * older text. Pending edits are flushed when the node changes or the pane unmounts.
 */
export function useAutosaveNode(node: WobuNode | undefined, delay = 500) {
  const upsert = useUpsertNode()
  const [status, setStatus] = useState<SaveStatus>('idle')

  const latest = useRef<WobuNode | undefined>(node)
  const patch = useRef<Partial<WobuNode> | null>(null)
  const timer = useRef<number | undefined>(undefined)
  const mutate = useRef(upsert.mutate)

  mutate.current = upsert.mutate
  // Only adopt server state we are not currently overwriting.
  if (!patch.current) latest.current = node

  const send = useCallback(() => {
    const base = latest.current
    const p = patch.current
    if (!base || !p) return
    patch.current = null
    const next = { ...base, ...p }
    latest.current = next
    setStatus('saving')
    mutate.current(next, {
      onSuccess: (saved) => {
        if (!patch.current) latest.current = saved
        setStatus('saved')
      },
      onError: (e) => {
        setStatus('error')
        report(e, 'Save failed')
      },
    })
  }, [])

  const flush = useCallback(() => {
    if (timer.current !== undefined) {
      window.clearTimeout(timer.current)
      timer.current = undefined
    }
    send()
  }, [send])

  const queue = useCallback(
    (p: Partial<WobuNode>) => {
      patch.current = { ...(patch.current ?? {}), ...p }
      setStatus('dirty')
      if (timer.current !== undefined) window.clearTimeout(timer.current)
      timer.current = window.setTimeout(() => {
        timer.current = undefined
        send()
      }, delay)
    },
    [delay, send],
  )

  // Flush on unmount and whenever the edited node is swapped out.
  const id = node?.id
  useEffect(() => {
    return () => {
      if (timer.current !== undefined) {
        window.clearTimeout(timer.current)
        timer.current = undefined
        send()
      }
    }
  }, [id, send])

  return { queue, flush, status }
}

export function saveLabel(status: SaveStatus): string {
  switch (status) {
    case 'dirty':
      return 'unsaved…'
    case 'saving':
      return 'saving…'
    case 'saved':
      return 'saved'
    case 'error':
      return 'save failed'
    default:
      return ''
  }
}
