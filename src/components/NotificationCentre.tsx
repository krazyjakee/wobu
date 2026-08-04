import { useEffect, useRef, useState } from 'react'
import {
  subscribeErrorToasts,
  unreadCostsMoney,
  unreadCount,
  useNotifications,
  type Notification,
} from '../lib/notifications'
import { relativeTime } from '../lib/time'
import { Icon } from './Icon'
import { IconButton, TipButton } from './Tooltip'

/**
 * The reviewable half of the error surface — #142.
 *
 * Toasts announce, banners hold a broken condition open, and neither of them is
 * a *record*: a failure that happened while the user was in another window, or
 * eight seconds ago, or before they switched modes, has nowhere to be read
 * back. That is the gap this closes, and since #144 removed the global
 * generation-history tab it is the only surface a failure with no node context
 * has at all.
 *
 * Mounted from the status bar, which is on screen in every mode, so the trigger
 * and its unread count follow the user around rather than living in one tab.
 */
export function NotificationCentre() {
  const entries = useNotifications((s) => s.entries)
  const open = useNotifications((s) => s.open)
  const setOpen = useNotifications((s) => s.setOpen)
  const markAllRead = useNotifications((s) => s.markAllRead)
  const clear = useNotifications((s) => s.clear)
  const dismiss = useNotifications((s) => s.dismiss)
  const panel = useRef<HTMLDivElement>(null)
  const unread = unreadCount(entries)
  const owed = unreadCostsMoney(entries)

  // Every failed command already goes through `report`; this is what makes its
  // toast durable without moving that decision out of the store.
  useEffect(() => subscribeErrorToasts(), [])

  // Reading the list is what marks it read, rather than a button nobody presses.
  useEffect(() => {
    if (!open) return
    panel.current?.focus()
    const timer = window.setTimeout(markAllRead, 600)
    return () => window.clearTimeout(timer)
  }, [open, markAllRead, entries.length])

  return (
    <>
      <TipButton
        className={owed ? 'status-link notif-bell is-charged' : 'status-link notif-bell'}
        aria-expanded={open}
        aria-label={
          unread === 0
            ? `Notifications, nothing unread, ${entries.length} in history`
            : owed
              ? `Notifications, ${unread} unread including a billed failure`
              : `Notifications, ${unread} unread`
        }
        tip={bellTip(entries.length, unread, owed)}
        onClick={() => setOpen(!open)}
      >
        · notifications{unread > 0 && <b className="notif-count">{unread}</b>}
      </TipButton>

      {open && (
        <div
          className="notif-panel"
          role="dialog"
          aria-label="Notifications"
          tabIndex={-1}
          ref={panel}
          onKeyDown={(event) => {
            if (event.key === 'Escape') setOpen(false)
          }}
        >
          <header className="notif-head">
            <b>Notifications</b>
            <span className="notif-space" />
            {entries.length > 0 && (
              <button className="btn-mini" type="button" onClick={clear}>
                Clear all
              </button>
            )}
            <IconButton
              className="notif-x"
              label="Close notifications"
              onClick={() => setOpen(false)}
            >
              <Icon name="x" size="sm" />
            </IconButton>
          </header>

          {entries.length === 0 ? (
            <p className="notif-empty">
              Nothing to report. Failed generations, rejected saves and anything that cost money
              without producing a result are kept here.
            </p>
          ) : (
            <ul className="notif-list">
              {entries.map((entry) => (
                <NotificationRow key={entry.id} entry={entry} onDismiss={() => dismiss(entry.id)} />
              ))}
            </ul>
          )}
        </div>
      )}
    </>
  )
}

/**
 * What the trigger says on hover, as opposed to what it says to a screen
 * reader. The label is a count; this is what the count is *for*, which is the
 * part nobody guesses from the word "notifications" in a status bar.
 */
function bellTip(total: number, unread: number, owed: boolean): string {
  if (owed) return 'Something failed after it had already been billed. Open this and read it.'
  if (unread > 0) return `${unread} unread. Failures and rejected saves are kept here.`
  if (total > 0) return `Nothing new. ${total} kept, oldest last.`
  return 'Failed generations, rejected saves and anything that cost money are kept here.'
}

function NotificationRow({ entry, onDismiss }: { entry: Notification; onDismiss: () => void }) {
  const [detailOpen, setDetailOpen] = useState(false)

  return (
    <li className={entry.charge ? 'notif-item is-charged' : 'notif-item'}>
      <div className="notif-row">
        <b className="notif-title">{entry.title}</b>
        <span className="notif-when">{relativeTime(entry.at)}</span>
        <IconButton
          className="notif-x"
          label={`Dismiss: ${entry.title}`}
          tip="Remove this from the history. It is not retried and nothing is undone."
          onClick={onDismiss}
        >
          <Icon name="x" size="sm" />
        </IconButton>
      </div>
      {/* The money line is first and unfolded on purpose. It is the one fact a
          user must not have to open anything to discover. */}
      {entry.charge && <p className="notif-charge">{entry.charge}</p>}
      <p className="notif-guide">{entry.guidance}</p>
      {entry.reason && <p className="notif-reason">{entry.reason}</p>}
      <div className="notif-acts">
        {entry.action && (
          <button className="btn-mini" type="button" onClick={entry.action.run}>
            {entry.action.label}
          </button>
        )}
        {entry.detail && (
          <button
            className="notif-more"
            type="button"
            aria-expanded={detailOpen}
            onClick={() => setDetailOpen((value) => !value)}
          >
            {detailOpen ? 'Hide details' : 'Details'}
          </button>
        )}
      </div>
      {detailOpen && entry.detail && <code className="notif-detail">{entry.detail}</code>}
    </li>
  )
}
