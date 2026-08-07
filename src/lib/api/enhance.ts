import { call } from './call'
import type { SectionValue, WobuNode } from './model'
/* ── domain types ─────────────────────────────────────────────────────────── */

/**
 * A structured description, in the one shape it crosses the bridge in. The same
 * type `WobuNode.description` holds, deliberately: a half-written description
 * and a finished one are rendered by the same component.
 */
export interface WobuDescription {
  sections: Record<string, SectionValue>
}

/**
 * The document so far, repainted as it arrives.
 *
 * Whole snapshots rather than appends. Events are fire-and-forget, so a pane
 * that accumulated fragments would be permanently wrong the first time one was
 * dropped — and wrong in a way that reads as the model having written nonsense.
 * Draw whatever the last one of these said and nothing else.
 *
 * Sections arrive in the kind's declared order, and one that has opened but
 * streamed nothing is present and empty, so its heading can appear before its
 * text does. **None of this is saved anywhere.** It is display state; what a
 * node ends up holding is whatever `enhanceAccept` is given.
 */
export interface EnhanceDelta {
  jobId: string
  nodeId: string
  description: WobuDescription
  questions: string[]
}

/** Mirrored from `src-tauri/src/enhance.rs`. */
export const ENHANCE_DELTA = 'enhance:delta'

/**
 * A finished description waiting for an answer — a whole, schema-valid one,
 * which is the only kind there is.
 *
 * Arrives twice over: as the `result` on an Enhance's `job:done`, and from
 * `enhancePending` afterwards. One shape rather than two, because they are the
 * same thing seen a moment apart.
 *
 * `questions` is what the model would otherwise have had to invent, asked
 * instead. It is not part of the description, is never written to the node, and
 * never reaches an image model: it is addressed to whoever wrote the notes.
 */
export interface EnhanceReady {
  /** The id to pass to `enhanceAccept` or `enhanceDiscard`. */
  jobId: string
  nodeId: string
  description: WobuDescription
  questions: string[]
}

/**
 * Everything still waiting to be accepted, for the open project.
 *
 * The catch-up read for the one case `job:done` cannot cover — a webview that
 * reloaded after it fired. That case matters more here than anywhere else in the
 * app, because what was lost has already been *paid for*: without this, a reload
 * mid-review means running the call again to recover an answer the backend is
 * still holding.
 *
 * The whole list, because after a reload there is no job id to look one up by —
 * match on `nodeId` and answer with `jobId`. At most a handful of entries, and
 * empty when nothing is open.
 */
export const enhancePending = () => call<EnhanceReady[]>('enhance_pending')

/**
 * What `enhanceAccept` did.
 *
 * `refusedEdit` is a result, not a failure. The description on disk was written
 * by hand and nobody has said to replace it, so the node comes back untouched
 * and the right response is to show the user what is about to be overwritten and
 * ask — then call again with `force`. A *conflict* is different and arrives as a
 * rejection with `write.conflict`, the same as any other lost save race.
 */
export type EnhanceAccepted =
  { outcome: 'saved'; node: WobuNode } | { outcome: 'refusedEdit'; node: WobuNode }

/**
 * Start an Enhance. Resolves with a job id before any of the work happens.
 *
 * Rejects, without spending anything, when this machine has no key for the
 * provider the project selected (`provider.no_key`, not retryable — the answer
 * is to paste a key in Settings, not to press the button again), when the
 * project is read-only, or when the node is gone. Everything after that is the
 * queue's: watch `job:state` for the job, `enhance:delta` for the text, and
 * `job:done` for an `EnhanceReady`.
 *
 * Stopping it is `jobCancel`, like any other job. That aborts the request rather
 * than discarding its answer, so a Stop actually stops the meter.
 */
export const enhanceStart = (nodeId: string) => call<string>('enhance_start', { nodeId })

/**
 * Write a finished description to its node, stamping the upstream versions it
 * was built from.
 *
 * `description` is what the user is accepting — pass an edited one to save the
 * edit, or omit it for exactly what the model sent. `force` answers a previous
 * `refusedEdit` and means nothing else.
 */
export const enhanceAccept = (jobId: string, description?: WobuDescription, force?: boolean) =>
  call<EnhanceAccepted>('enhance_accept', { jobId, description, force })

/** Reject one. Not an error when there is nothing left to reject. */
export const enhanceDiscard = (jobId: string) => call<void>('enhance_discard', { jobId })
