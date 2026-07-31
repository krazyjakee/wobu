import { useCallback, useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import type { WobuNode } from '../lib/api'
import { isRetryable, isTauri } from '../lib/api'
import { useUpsertNode } from '../lib/queries'
import { report } from '../store/ui'

export type SaveStatus = 'idle' | 'dirty' | 'saving' | 'saved' | 'error' | 'held'

/**
 * Debounced writes through `node_upsert`. The queued patch is merged onto the
 * freshest node we hold, so a `world:changed` refetch mid-typing cannot resurrect
 * older text. Pending edits are flushed when the node changes or the pane unmounts.
 *
 * A failure that could still succeed later — the share went away — puts the
 * patch *back* on the queue rather than dropping it, and the edit is resent
 * when `share:online` says the folder is reachable again. Anything else is a
 * genuine rejection, and holding onto it would only resend something the
 * backend has already refused.
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
        if (isRetryable(e)) {
          // Put it back, behind anything typed since — newer keystrokes win,
          // but the text that failed to save is not lost.
          patch.current = { ...p, ...(patch.current ?? {}) }
          setStatus('held')
        } else {
          setStatus('error')
        }
        report(e, 'Save failed')
      },
    })
  }, [])

  // The share came back — resend whatever has been waiting. `send` is a no-op
  // when there is no pending patch, so this costs nothing in the common case.
  useEffect(() => {
    if (!isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined
    void listen('share:online', () => send())
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* nothing to listen to */
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [send])

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
    case 'held':
      return 'waiting for the share…'
    case 'error':
      return 'save failed'
    default:
      return ''
  }
}
