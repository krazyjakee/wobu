import { call } from './call'
/* ── domain types ─────────────────────────────────────────────────────────── */

/**
 * Everything long-running is a job: it returns an id immediately and reports
 * itself over the events below. Nothing on this side ever waits for one.
 *
 * The shapes here mirror `wobu-jobs` exactly — see `src-tauri/crates/wobu-jobs`,
 * where the reasoning for each of them is written down.
 */
export type JobKind = 'enhance' | 'generate' | 'train_lora' | 'mesh' | 'thumbnail'

/**
 * Whether the attempt that failed cost the user money.
 *
 * This is the field that decides whether the queue retried on its own. It never
 * retries a `charged` or `unknown` failure without being told to in advance,
 * because that would be spending someone's money on a hunch.
 */
export type Billed = 'nothing' | 'charged' | 'unknown'

export interface JobFailure {
  /** The same dotted codes command errors use, so `errorSurface` fits both. */
  code: string
  message: string
  /** Whether another attempt could work. Not whether it should — see `billed`. */
  retryable: boolean
  detail?: string
  /** The provider's own wait hint, in milliseconds. */
  retryAfter?: number
  billed: Billed
  /** What the failed attempt cost, in the backend's words. The queue cannot
   *  price a call, so this is the only thing that says what "again" means. */
  costNote?: string
}

/**
 * Where a job is. Flat rather than nested so a component switches on one field.
 *
 * `retryHeld` on a failure is the interesting one: the job *can* be retried and
 * the queue would not do it, because the attempt was billed. That is an offer to
 * put in front of the user, not an apology.
 */
export type JobState =
  | { state: 'queued' }
  | { state: 'running' }
  | { state: 'retrying'; inMs: number; costsMoney: boolean }
  | { state: 'done' }
  | { state: 'cancelled' }
  | { state: 'failed'; failure: JobFailure; retryHeld: boolean }

export type JobSnapshot = {
  id: string
  kind: JobKind
  label: string
  /** Domain subject, when the task has one. Generate jobs use their node id. */
  subjectId: string | null
  /** Attempts started so far, from 1. Zero while queued. */
  attempt: number
  /** Backend-measured time since the first attempt, frozen at completion. */
  elapsedMs: number
} & JobState

/**
 * The whole queue, sent on every transition. It includes a bounded tail of
 * finished jobs, so the last outcome is still on screen after it happened.
 */
export interface QueueSnapshot {
  jobs: JobSnapshot[]
  queued: number
  running: number
  retrying: number
}

/** Everything that has not finished — the number a status bar shows. */
export function jobDepth(snapshot: QueueSnapshot): number {
  return snapshot.queued + snapshot.running + snapshot.retrying
}

export interface JobProgress {
  id: string
  done: number
  total: number
  /** A backend's own words for the step — "sampling 12/30". */
  note?: string
}

export interface JobPreview {
  id: string
  /** Opaque on purpose: how a latent preview reaches here is #40's decision. */
  image: string
  step?: number
}

export interface JobDone {
  id: string
  kind: JobKind
  label: string
  /** Whatever the job decided its caller needs; absent when the result is on disk. */
  result?: unknown
}

export interface JobFailed {
  id: string
  kind: JobKind
  label: string
  failure: JobFailure
  retryHeld: boolean
}

/**
 * The event names, mirrored from `wobu_jobs::events`.
 *
 * There is deliberately no `job:cancelled` — a user who pressed Stop does not
 * need to be told it stopped, and `job:state` carries it for anything drawing
 * the queue.
 */
export const JOB_EVENTS = {
  state: 'job:state',
  progress: 'job:progress',
  preview: 'job:preview',
  retry: 'job:retry',
  done: 'job:done',
  error: 'job:error',
} as const

/**
 * Stop a job. `false` if there is no such job or it had already finished.
 *
 * Returns as soon as the backend has been told, not when the work has stopped:
 * a job that had not started is over immediately, and one in flight is aborted
 * within the grace it gets to report what it was charged. Cancelling something
 * that has already finished is not an error — the user can press Stop at the
 * instant a job ends, and that race is ordinary.
 */
export const jobCancel = (jobId: string) => call<boolean>('job_cancel', { jobId })

/**
 * The queue as it stands. The `job:state` event is the live signal; this covers
 * the one case it cannot — a webview that reloaded mid-generation.
 */
export const jobList = () => call<QueueSnapshot>('job_list')

/* ── enhance ──────────────────────────────────────────────────────────────── */
