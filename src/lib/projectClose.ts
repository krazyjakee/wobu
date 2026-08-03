import * as api from './api'
import { editorWrites, isEditorWritesBlocked } from './editorWrites'
import { report, useUI } from '../store/ui'

export const EDITOR_CLOSE_BLOCKED = 'editor.close_blocked'
export const JOBS_CLOSE_BLOCKED = 'jobs.close_blocked'

/** The only safe ordering for closing a project and staying in the app. */
export async function closeProjectAfterEditorWrites(): Promise<void> {
  await editorWrites.flushAll()
  await api.projectClose()
  useUI.getState().clearBanner(EDITOR_CLOSE_BLOCKED)
}

/** The queue states that mean the backend still has work in hand. */
const UNFINISHED = new Set(['queued', 'running', 'retrying'])

/**
 * Everything the backend has not finished.
 *
 * Asked for at close time rather than read from a mounted `useJobQueue`,
 * because the queue outlives the project and the launcher has no such
 * subscription — and quitting from the launcher with a generation in flight is
 * precisely the case a component-scoped answer would miss.
 */
export async function unfinishedJobs(): Promise<api.JobSnapshot[]> {
  try {
    const { jobs } = await api.jobList()
    return jobs.filter((job) => UNFINISHED.has(job.state))
  } catch {
    // Being unable to enumerate the queue must never become a window that will
    // not close. `src-tauri/src/shutdown.rs` cancels whatever is left on the
    // way out regardless; what is lost here is the chance to warn first.
    return []
  }
}

/**
 * Quitting would destroy work that is still in progress.
 *
 * Not an error in the sense of something going wrong — it is the close being
 * refused once so the user can answer. `docs/15-exit-policy.md` says why the
 * answer is a prompt rather than a wait: a generation can take minutes, and an
 * app that sat on a quit until a provider replied would look hung.
 */
export class QuitHeldByJobs extends Error {
  constructor(readonly jobs: api.JobSnapshot[]) {
    super('Unfinished jobs would be destroyed by quitting')
    this.name = 'QuitHeldByJobs'
  }
}

export function isQuitHeldByJobs(error: unknown): error is QuitHeldByJobs {
  return error instanceof QuitHeldByJobs
}

export interface WindowCloseOptions {
  /** Whether a project is installed; the launcher closes without one. */
  projectOpen: boolean
  /** The user has been warned about unfinished jobs and said to go ahead. */
  discardJobs?: boolean
}

/**
 * The gate every window close runs, in the one order that loses nothing.
 *
 * 1. **Settle every editor write.** Block-and-flush rather than fire-and-forget:
 *    the debounce is cancelled, the patch is sent, and this does not return
 *    until the backend has answered. A write that cannot land throws, which
 *    leaves the workspace and its controlled inputs mounted so the text is
 *    still on screen and still recoverable.
 * 2. **Refuse once for unfinished jobs.** They come *after* the flush so that a
 *    warning the user answers with "stay" has already saved their typing, and
 *    *before* `project_close` so that answering it does not leave them staring
 *    at the launcher.
 * 3. **Stop those jobs rather than orphan them.** Cancelling runs each
 *    adapter's own stop path — ComfyUI's `/interrupt`, a provider's billed
 *    report — which is the difference between a job that ends and one the user
 *    keeps paying for after the window is gone.
 * 4. **Close the project.** Which releases the folder, drops our presence entry
 *    and closes the index.
 */
export async function closeForWindowExit({
  projectOpen,
  discardJobs = false,
}: WindowCloseOptions): Promise<void> {
  await editorWrites.flushAll()

  if (!discardJobs) {
    const held = await unfinishedJobs()
    if (held.length > 0) throw new QuitHeldByJobs(held)
  } else {
    await stopUnfinishedJobs()
  }

  if (projectOpen) await api.projectClose()
  const ui = useUI.getState()
  ui.clearBanner(EDITOR_CLOSE_BLOCKED)
  ui.clearBanner(JOBS_CLOSE_BLOCKED)
}

/**
 * Ask the backend to stop everything still in flight.
 *
 * `allSettled` because a cancel that fails must not hold the window open: the
 * backend's own wind-down closes the queue again on the way out, so the worst a
 * failure here costs is that one job is stopped a moment later than intended.
 */
async function stopUnfinishedJobs(): Promise<void> {
  const held = await unfinishedJobs()
  await Promise.allSettled(held.map((job) => api.jobCancel(job.id)))
}

export interface CloseRefusalHandlers {
  /** Try the same close again, unchanged — for a save that can now succeed. */
  retry: () => void
  /** Close anyway, accepting that the unfinished jobs are stopped. */
  quitAnyway: () => void
}

/**
 * Turn a refused close into something the user can answer.
 *
 * Both refusals keep the window alive and neither is a dead end, which is the
 * rule the whole exit policy is written to: a close that will lose something is
 * always a question, never a silent decision either way.
 *
 * A bare callback is accepted as the retry, for the project-close path: closing
 * a project cannot be held by jobs — the queue is per installation and a
 * generation writes its result by path, so it survives the world being shut —
 * and a caller that has no "quit anyway" to offer should not have to invent one.
 */
export function reportProjectCloseFailure(
  error: unknown,
  handlers: CloseRefusalHandlers | (() => void),
): void {
  const { retry, quitAnyway } =
    typeof handlers === 'function' ? { retry: handlers, quitAnyway: handlers } : handlers

  if (isQuitHeldByJobs(error)) {
    const count = error.jobs.length
    useUI.getState().raiseBanner({
      code: JOBS_CLOSE_BLOCKED,
      text:
        `Wobu did not quit, because ${count} ${count === 1 ? 'job is' : 'jobs are'} still ` +
        'running. Quitting stops them, and whatever a provider has already charged for is not ' +
        'refunded.',
      detail: error.jobs.map((job) => job.label).join(', '),
      retryable: false,
      action: { label: 'Stop them and quit', run: quitAnyway },
    })
    return
  }

  if (!isEditorWritesBlocked(error)) {
    report(error, 'Could not close project')
    return
  }

  const count = Math.max(1, error.writes.length)
  useUI.getState().raiseBanner({
    code: EDITOR_CLOSE_BLOCKED,
    text:
      `Wobu kept this project open because ${count} editor ${count === 1 ? 'write has' : 'writes have'} ` +
      'not saved. Resolve any save error or conflict, then retry.',
    retryable: true,
    sticky: true,
    action: { label: 'Retry save and close', run: retry },
  })
}
