import { useCallback, useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import type { WobuNode } from '../lib/api'
import { errorCode, isRetryable, isTauri } from '../lib/api'
import { useUpsertNode } from '../lib/queries'
import { useSettings } from '../store/settings'
import { report } from '../store/ui'

export type SaveStatus = 'idle' | 'dirty' | 'saving' | 'saved' | 'error' | 'held'

export interface AutosaveOptions {
  /** Overrides the configured debounce. The tests use this; the editor does not. */
  delay?: number
  /** A read-only folder never gets a write — see `src/lib/readOnly.ts`. */
  readOnly?: boolean
}

/**
 * Debounced writes through `node_upsert`. The queued patch is merged onto the
 * freshest node we hold. A `world:changed` refetch of the node being edited
 * becomes the new base even while a patch is queued: the remote edit wins every
 * field the user did not touch, and the queued fields are rebased onto it when
 * the debounce closes. The editor's controlled fields keep the local value
 * under an active cursor; adopting the base here never rewrites the input.
 * Pending edits are flushed when the selected node changes or the pane unmounts.
 *
 * A failure that could still succeed later — the share went away — puts the
 * patch *back* on the queue rather than dropping it, and the edit is resent
 * when `share:online` says the folder is reachable again. Anything else is a
 * genuine rejection, and holding onto it would only resend something the
 * backend has already refused. `write.conflict` is different: the conflict
 * card owns the decision, so the patch is retained until the user acts rather
 * than disappearing from the only writer that still knows it is unsaved.
 *
 * A read-only folder is the one case where nothing is queued in the first
 * place: there is no later in which that write could succeed.
 */
export function useAutosaveNode(node: WobuNode | undefined, options: AutosaveOptions = {}) {
  // The setting is read here rather than passed down from the editor, so that
  // changing it in Settings takes effect on the pane already open. An explicit
  // argument still wins, which is what the tests use.
  const configured = useSettings((s) => s.autosaveDelay)
  const delay = options.delay ?? configured
  const readOnly = options.readOnly ?? false

  const upsert = useUpsertNode()
  const [status, setStatus] = useState<SaveStatus>('idle')

  const latest = useRef<WobuNode | undefined>(node)
  const nextSelection = useRef<{ node: WobuNode | undefined } | null>(null)
  const patch = useRef<Partial<WobuNode> | null>(null)
  const timer = useRef<number | undefined>(undefined)
  const mutate = useRef(upsert.mutate)

  // A refetch of this same node is the remote half of a rebase. Adopting it
  // while `patch` is pending is safe because the patch contains only fields the
  // active control changed, and it is essential because `node_upsert` guards
  // against the index's *current* stamp rather than carrying a client version.
  //
  // A different id is a selection change, not a rebase. Keep the old base until
  // the id effect below flushes it, or A's last sentence would be merged onto B.
  useEffect(() => {
    const base = latest.current
    if (!patch.current || !base || (node && base.id === node.id)) latest.current = node
    else nextSelection.current = { node }
  }, [node])

  useEffect(() => {
    mutate.current = upsert.mutate
  }, [upsert.mutate])

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
        const retryable = isRetryable(e)
        const conflict = errorCode(e) === 'write.conflict'
        if (retryable || conflict) {
          // Put it back, behind anything typed since — newer keystrokes win,
          // but the text that failed to save is not lost.
          patch.current = { ...p, ...(patch.current ?? {}) }
          setStatus(retryable ? 'held' : 'error')
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
      // On a read-only folder nothing is queued at all, rather than queued and
      // rejected at save time. A patch that can never land would otherwise sit
      // in the debounce, be flushed again on unmount, and be resent by
      // `share:online` — three failures for one edit the UI already refused,
      // and a `saving…` label promising something that is not happening.
      if (readOnly) return
      patch.current = { ...(patch.current ?? {}), ...p }
      setStatus('dirty')
      if (timer.current !== undefined) window.clearTimeout(timer.current)
      timer.current = window.setTimeout(() => {
        timer.current = undefined
        send()
      }, delay)
    },
    [delay, readOnly, send],
  )

  // Flush on unmount and whenever the edited node is swapped out.
  const id = node?.id
  useEffect(() => {
    // React runs the previous id effect's cleanup before creating this one.
    // That cleanup flushes A against A; only then is it safe to make B the
    // base for the next edit.
    if (nextSelection.current) {
      latest.current = nextSelection.current.node
      nextSelection.current = null
    }
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
