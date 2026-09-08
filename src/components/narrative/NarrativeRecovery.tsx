import { useEffect, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { Modal } from '../Modal'
import { errorMessage } from '../../lib/api'
import { invalidateNarrative, qk } from '../../lib/queries/keys'
import {
  narrativeRecoveryList,
  narrativeRecoveryRestore,
  type RetainedNarrativeDeletion,
} from '../../lib/api/narrativeRecovery'
import './export.css'
import './recovery.css'

export function NarrativeRecovery({
  readOnly,
  onClose,
}: {
  readOnly: boolean
  onClose: () => void
}) {
  const queryClient = useQueryClient()
  const [items, setItems] = useState<RetainedNarrativeDeletion[]>([])
  const [loading, setLoading] = useState(true)
  const [restoring, setRestoring] = useState<string | null>(null)
  const [error, setError] = useState('')
  const [messages, setMessages] = useState<Record<string, string>>({})
  const busy = loading || restoring !== null
  useEffect(() => {
    let mounted = true
    void narrativeRecoveryList()
      .then((rows) => {
        if (mounted) setItems(rows)
      })
      .catch((reason: unknown) => {
        if (mounted) setError(errorMessage(reason))
      })
      .finally(() => {
        if (mounted) setLoading(false)
      })
    return () => {
      mounted = false
    }
  }, [])
  const refresh = async () => {
    if (busy) return
    setLoading(true)
    setError('')
    try {
      setItems(await narrativeRecoveryList())
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setLoading(false)
    }
  }
  const restore = async (item: RetainedNarrativeDeletion) => {
    if (busy || readOnly) return
    setRestoring(item.id)
    setError('')
    try {
      const result = await narrativeRecoveryRestore(item.id)
      setMessages((previous) => ({
        ...previous,
        [item.id]:
          result.status === 'saved'
            ? `Original content restored to ${item.target}.`
            : `The newer current file was kept. The deleted version is retained at ${result.conflictPath}. Review it using conflict recovery.`,
      }))
      setItems((previous) =>
        previous.map((row) => (row.id === item.id ? { ...row, restored: true } : row)),
      )
      invalidateNarrative(queryClient)
      void queryClient.invalidateQueries({ queryKey: qk.conflicts })
      try {
        setItems(await narrativeRecoveryList())
      } catch (reason) {
        setError(
          `Restoration completed, but the history could not be refreshed: ${errorMessage(reason)}`,
        )
      }
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setRestoring(null)
    }
  }
  return (
    <Modal
      onClose={onClose}
      busy={restoring !== null}
      titleId="narrative-recovery-title"
      descriptionId="narrative-recovery-description"
    >
      <section className="nrt-export nrt-recovery">
        <h2 id="narrative-recovery-title">Narrative recovery</h2>
        <p id="narrative-recovery-description">
          Retained deletions preserve original content. Restore requests never replace a newer
          current file; competing versions remain available for review.
        </p>
        <div className="nrt-export-actions">
          <button className="btn" disabled={busy} onClick={() => void refresh()}>
            Refresh history
          </button>
          <button className="btn" disabled={restoring !== null} onClick={onClose}>
            Close
          </button>
        </div>
        {readOnly && (
          <p>
            This project is read-only. Recovery history is available; restoring requires write
            access.
          </p>
        )}
        {loading && <p role="status">Loading retained deletions…</p>}
        {!loading && !items.length && !error && <p>No retained narrative deletions.</p>}
        <ul className="nrt-recovery-list" aria-label="Retained narrative deletions">
          {items.map((item) => (
            <li key={item.id}>
              <h3>{item.name}</h3>
              <p className="nrt-recovery-path">{item.target}</p>
              <p>{item.restored ? 'Restoration requested' : 'Original version retained'}</p>
              <button
                className="btn"
                disabled={busy || readOnly}
                onClick={() => void restore(item)}
              >
                {restoring === item.id
                  ? 'Restoring…'
                  : `${item.restored ? 'Retry restore' : 'Restore'} ${item.name}`}
              </button>
              {messages[item.id] && (
                <p role="status" className="nrt-recovery-path">
                  {messages[item.id]}
                </p>
              )}
            </li>
          ))}
        </ul>
        {error && <p role="alert">{error}</p>}
      </section>
    </Modal>
  )
}
