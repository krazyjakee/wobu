import { create } from 'zustand'
import type { JobSnapshot } from './api'
import { costsMoney, failureNotice, jobKindLabel } from './failureCopy'
import { toast, useUI } from '../store/ui'

/**
 * The notification centre — #142.
 *
 * A toast is the right surface for "saved" and the wrong one for "that cost you
 * money and produced nothing": it is gone in eight seconds, it cannot be
 * scrolled back to, and a user who was looking at another window when it
 * appeared never learns the thing happened at all. Since #144 removed the
 * global generation-history tab there is no other place a failure can land
 * either — a job whose subject was deleted, or which never had one, has no node
 * to be listed under.
 *
 * So: every failure is *recorded* here, durably, and *announced* by a toast.
 * The toast is the interruption; this is the record. The store is module-level
 * rather than mounted, so switching modes, closing a pane or navigating away
 * cannot lose it — which is the acceptance criterion "notification history
 * survives navigating between modes", satisfied structurally rather than by
 * everybody remembering to keep it mounted.
 */

export interface NotificationAction {
  label: string
  run: () => void
}

export interface Notification {
  id: number
  /**
   * Identity of the underlying event, not of this row.
   *
   * The queue re-sends the whole snapshot on every transition, so a job that
   * failed once appears in every snapshot afterwards. Recording is idempotent
   * on this key, which is what stops one failure becoming forty rows.
   */
  key: string
  at: number
  title: string
  /** What to do next. Always present; see `failureCopy`. */
  guidance: string
  /** The backend's own sentence, kept because it is evidence. */
  reason?: string
  /** Technical remainder. Folded away until asked for. */
  detail?: string
  /** Non-null only when the user's money was, or may have been, spent. */
  charge?: string | null
  action?: NotificationAction
  read: boolean
}

interface CentreState {
  entries: Notification[]
  open: boolean
  setOpen: (open: boolean) => void
  /** `true` when this is new, `false` when the same event is already recorded. */
  record: (entry: Omit<Notification, 'id' | 'at' | 'read'>) => boolean
  markAllRead: () => void
  dismiss: (id: number) => void
  clear: () => void
}

/**
 * How many failures the centre remembers.
 *
 * Long enough to cover an afternoon of a misconfigured backend failing every
 * job, short enough that it is a list rather than a database. The oldest go
 * first; a failure nobody looked at in sixty events is not the one they are
 * looking for.
 */
const LIMIT = 60

let seq = 0

export const useNotifications = create<CentreState>((set, get) => ({
  entries: [],
  open: false,
  setOpen: (open) => set({ open }),
  record: (entry) => {
    if (get().entries.some((existing) => existing.key === entry.key)) return false
    set((s) => ({
      entries: [{ ...entry, id: ++seq, at: Date.now(), read: false }, ...s.entries].slice(0, LIMIT),
    }))
    return true
  },
  markAllRead: () =>
    set((s) => ({
      entries: s.entries.some((entry) => !entry.read)
        ? s.entries.map((entry) => (entry.read ? entry : { ...entry, read: true }))
        : s.entries,
    })),
  dismiss: (id) => set((s) => ({ entries: s.entries.filter((entry) => entry.id !== id) })),
  clear: () => set({ entries: [] }),
}))

export function unreadCount(entries: Notification[]): number {
  return entries.reduce((total, entry) => total + (entry.read ? 0 : 1), 0)
}

/** Whether anything unread cost money — the one thing that changes the badge. */
export function unreadCostsMoney(entries: Notification[]): boolean {
  return entries.some((entry) => !entry.read && Boolean(entry.charge))
}

/** Failures whose fix is a setting, not another attempt at the same request. */
const SETTINGS_CODES = new Set([
  'provider.no_key',
  'provider.bad_key',
  'provider.keychain_unavailable',
  'provider.billing_required',
  'provider.rate_limited',
])

/**
 * Where the user has to go to make the next attempt work.
 *
 * There is no `job_retry` command, so nothing here re-runs anything: the honest
 * affordance is one that takes the user to the control that starts the job, and
 * is labelled as doing that. A button marked "Retry" which silently navigated
 * would be worse than no button, because the user would walk away believing a
 * second attempt was already running.
 */
function jobAction(job: JobSnapshot, code: string): NotificationAction | undefined {
  if (SETTINGS_CODES.has(code)) {
    return {
      label: 'Open Settings',
      run: () => {
        useNotifications.getState().setOpen(false)
        useUI.getState().setMode('settings')
      },
    }
  }
  const subject = job.subjectId
  if (!subject) return undefined
  if (job.kind === 'train_lora') {
    return {
      label: 'Open in Forge',
      run: () => {
        useNotifications.getState().setOpen(false)
        useUI.getState().select(subject)
        useUI.getState().setMode('forge')
      },
    }
  }
  const tab = job.kind === 'mesh' ? 'three' : job.kind === 'enhance' ? 'notes' : 'concepts'
  return {
    label: 'Show the entity',
    run: () => {
      useNotifications.getState().setOpen(false)
      useUI.getState().select(subject)
      useUI.getState().setMode('library')
      useUI.getState().setTab(tab)
    },
  }
}

/**
 * Announcing a job failure, once, on both surfaces.
 *
 * Called for every failed job in every queue snapshot; the key is what makes
 * that safe. A retry that fails again bumps the attempt, so it is a genuinely
 * new event and gets its own row.
 */
export function reportJobFailure(job: JobSnapshot): void {
  if (job.state !== 'failed') return
  const { failure } = job
  // A cancellation is the user's own decision arriving as a queue failure. The
  // exit gate cancels everything in flight on quit (docs/15-exit-policy.md), so
  // surfacing these would mean a wall of red for a deliberate act — and the
  // receipt for a cancelled paid job is written by the backend regardless.
  if (failure.code === 'cancelled') return

  const notice = failureNotice(job.kind, failure)
  const fresh = useNotifications.getState().record({
    key: `job:${job.id}#${job.attempt}`,
    title: notice.title,
    guidance: notice.guidance,
    reason: notice.reason,
    detail: notice.detail,
    charge: notice.charge,
    action: jobAction(job, failure.code),
  })
  if (!fresh) return

  // The toast states the cost in its own text rather than behind a disclosure.
  // A charge the user has to click to discover is a charge they will not see.
  const text = notice.charge
    ? `${jobKindLabel(job.kind)} failed. ${notice.charge}`
    : `${jobKindLabel(job.kind)} failed — ${job.label}`
  announce(() =>
    toast(text, 'error', {
      detail: failure.message,
      // Money, or anything worth another attempt, waits for the user rather
      // than expiring while they are in another window.
      persistent: costsMoney(failure) || failure.retryable,
      action: {
        label: 'What now?',
        run: () => useNotifications.getState().setOpen(true),
      },
    }),
  )
}

/**
 * Toasts raised from here are already in the centre, and the mirror below must
 * not file them twice. Zustand calls subscribers synchronously inside `set`, so
 * a flag held across the call is exact rather than hopeful.
 */
let announcing = false

function announce(push: () => void): void {
  announcing = true
  try {
    push()
  } finally {
    announcing = false
  }
}

/**
 * Every other error toast, mirrored into the centre.
 *
 * `report` in `src/store/ui.ts` is what every failed command goes through, and
 * its toast has the same problem this centre exists to fix. Mirroring rather
 * than changing `report` keeps one decision in one place — `report` still owns
 * *whether* something is a toast or a banner — while making the toast's text
 * durable. Banners are deliberately not mirrored: they persist already, and a
 * condition that is still true is not history.
 */
export function subscribeErrorToasts(): () => void {
  return useUI.subscribe((state, previous) => {
    if (state.toasts === previous.toasts) return
    // Each toast is judged once, at the moment it becomes an error, rather than
    // on every subsequent change to the list. Without that, a job failure this
    // module raised itself would be skipped by `announcing` when it appeared
    // and then filed a second time the next time any other toast moved.
    const before = new Map(previous.toasts.map((item) => [item.id, item.kind === 'error']))
    for (const item of state.toasts) {
      if (item.kind !== 'error') continue
      const seen = before.get(item.id)
      if (seen === true) continue
      if (seen === undefined && announcing) continue
      useNotifications.getState().record({
        key: `toast:${item.id}`,
        title: item.text,
        guidance:
          'Wobu could not finish something it was asked to do. The message above is the whole of what went wrong; if it names a place to fix, that is where to go.',
        detail: item.detail,
        charge: null,
      })
    }
  })
}
