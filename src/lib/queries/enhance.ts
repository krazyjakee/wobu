import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient, type UseQueryResult } from '@tanstack/react-query'
import { listen } from '@tauri-apps/api/event'
import * as api from '../api'
import { invalidateWorld, qk } from './keys'
/* ── keys ─────────────────────────────────────────────────────────────────── */

/**
 * Start an Enhance and get a job id back.
 *
 * Nothing here waits for the description. The call returns before the provider
 * is even asked, and the answer arrives over `enhance:delta` and `job:done` —
 * which is what lets the user carry on editing a different node while one is
 * running, and what makes Stop a real stop rather than an abandoned promise.
 *
 * Nothing is invalidated on success either, because nothing has changed yet: an
 * Enhance writes when it is *accepted*.
 */
export function useEnhance() {
  return useMutation({ mutationFn: (nodeId: string) => api.enhanceStart(nodeId) })
}

/**
 * Write a finished description to its node.
 *
 * `refusedEdit` comes back as a *success*, not a rejection, and the caller has
 * to handle it: the description on disk was written by hand and nobody has said
 * to replace it. Show what is about to be overwritten, and call again with
 * `force` if the user says yes. Treating it as a failure would turn a question
 * into a dialog to dismiss.
 */
export function useAcceptEnhanced() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { jobId: string; description?: api.WobuDescription; force?: boolean }) =>
      api.enhanceAccept(v.jobId, v.description, v.force),
    onSuccess: (accepted) => {
      // A refusal changed nothing on disk, so there is nothing to invalidate and
      // refetching would only flicker the pane the question is being asked in.
      if (accepted.outcome !== 'saved') return
      qc.setQueryData(qk.node(accepted.node.id), accepted.node)
      // The description state moved to `fresh`, which the navigator draws, and
      // a description is the whole input to the influence engine.
      void qc.invalidateQueries({ queryKey: qk.nodes })
      void qc.invalidateQueries({ queryKey: ['influence_resolve'] })
      void qc.invalidateQueries({ queryKey: ['prompt_compile'] })
      // It is no longer waiting — the backend dropped it the moment the write
      // landed, and a stale entry here would offer to accept it twice.
      void qc.invalidateQueries({ queryKey: qk.enhancePending })
    },
    onError: (e) => {
      if (api.errorCode(e) === 'write.conflict') invalidateWorld(qc)
    },
  })
}

export function useDiscardEnhanced() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (jobId: string) => api.enhanceDiscard(jobId),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: qk.enhancePending })
    },
  })
}

/**
 * Descriptions this process is still holding, waiting to be accepted.
 *
 * The catch-up read after a reload, and the reason it exists is money: the
 * `job:done` that carried the answer is gone with the old page, and without this
 * the only route back to a description that has already been paid for is paying
 * again. Match on `nodeId`, answer with `jobId`.
 *
 * `staleTime: Infinity` because nothing on this side changes it but the two
 * mutations above, and both invalidate. A description that arrives while the app
 * is running arrives on `job:done`; this is only ever for the ones that did not.
 */
export function useEnhancePending(enabled: boolean): UseQueryResult<api.EnhanceReady[]> {
  return useQuery({
    queryKey: qk.enhancePending,
    queryFn: () => api.enhancePending(),
    enabled,
    staleTime: Infinity,
  })
}

/**
 * The description one Enhance has streamed so far.
 *
 * A subscription rather than a query, for the same reason `useJobQueue` is one:
 * there is nothing to fetch and nothing to invalidate, and the backend already
 * sends whole snapshots. Rendering the payload as-is is the whole contract —
 * accumulating fragments on this side would be reassembling state the other
 * side is already sending correctly.
 *
 * `null` until the first frame, and again whenever `jobId` changes, so a pane
 * switched from one node to another never shows the previous node's text.
 * **Nothing here has been saved.** The node keeps whatever it had until
 * `useAcceptEnhanced` runs.
 */
export function useEnhanceStream(jobId: string | null): api.EnhanceDelta | null {
  const [stream, setStream] = useState<{ jobId: string; delta: api.EnhanceDelta } | null>(null)

  useEffect(() => {
    if (!jobId || !api.isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined

    void listen<api.EnhanceDelta>(api.ENHANCE_DELTA, (event) => {
      // Filtered here rather than by subscribing per job: one listener per
      // mounted pane is cheap, and two enhances can be in flight at once —
      // which is exactly when showing the wrong one would be hardest to spot.
      if (event.payload.jobId === jobId) setStream({ jobId, delta: event.payload })
    })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* nothing to listen to; the pane simply shows no progress */
      })

    return () => {
      disposed = true
      unlisten?.()
    }
  }, [jobId])

  return stream?.jobId === jobId ? stream.delta : null
}
