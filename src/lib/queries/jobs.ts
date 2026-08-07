import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import * as api from '../api'
import type { QueueSnapshot } from '../api'
import { reportJobFailure } from '../notifications'
/* ── keys ─────────────────────────────────────────────────────────────────── */

/**
 * The queue, live.
 *
 * Not a `useQuery`: there is nothing to invalidate and nothing to refetch. The
 * backend sends the whole queue on every transition, so this is a subscription
 * with one catch-up read for the case events cannot cover — a webview that
 * reloaded while three generations were in flight.
 *
 * Whole snapshots rather than accumulated deltas, deliberately. A queue
 * reassembled on this side from `progress`/`done`/`error` would be wrong the
 * first time an event was dropped or arrived out of order, and it would be
 * wrong in a way that shows: a job stuck on screen that finished minutes ago.
 *
 * This is also where every failure is surfaced (#142). Reading them out of the
 * snapshot rather than subscribing to `job:error` is the whole point: the
 * snapshot carries a bounded tail of finished jobs and is re-read by
 * `jobList()` on mount, so a webview that reloaded mid-generation still learns
 * that the job it left running had failed. An event listener would have been
 * asleep for exactly that, which is the case #142 was filed about.
 */
export function useJobQueue(): QueueSnapshot {
  const [snapshot, setSnapshot] = useState<QueueSnapshot>(EMPTY_QUEUE)

  useEffect(() => {
    if (!api.isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined

    // `reportJobFailure` is idempotent per job attempt, so re-running it over
    // every snapshot — the same failed job is in all of them until it falls off
    // the tail — announces each failure exactly once.
    const take = (current: QueueSnapshot) => {
      setSnapshot(current)
      for (const job of current.jobs) reportJobFailure(job)
    }

    void listen<QueueSnapshot>(api.JOB_EVENTS.state, (event) => take(event.payload))
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* nothing to listen to yet; the catch-up read below still applies */
      })

    void api
      .jobList()
      .then((current) => {
        if (!disposed) take(current)
      })
      .catch(() => {
        /* no queue to ask — an empty one is the right answer */
      })

    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])

  return snapshot
}

/** Shared so that an idle queue is referentially stable and renders nothing. */
const EMPTY_QUEUE: QueueSnapshot = { jobs: [], queued: 0, running: 0, retrying: 0 }

/* ── enhance ──────────────────────────────────────────────────────────────── */
