import { useState } from 'react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { errorCode, errorMessage, syncAccept, syncAcceptCancel } from '../lib/api'
import { Icon } from './Icon'
import { Modal } from './Modal'

type Step = 'idle' | 'probing' | 'accepting' | 'opening'

export function AcceptTicketSheet({
  onClose,
  onOpen,
}: {
  onClose: () => void
  onOpen: (root: string) => Promise<unknown>
}) {
  const [token, setToken] = useState('')
  const [step, setStep] = useState<Step>('idle')
  const [cancelling, setCancelling] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const busy = step !== 'idle'

  async function accept() {
    const pasted = token.trim()
    if (!pasted) {
      setError('Paste the complete Wobu share ticket.')
      return
    }
    setError(null)
    setStep('probing')
    try {
      let accepted = await syncAccept(pasted)
      if (!accepted) throw new Error('Wobu returned no result for this ticket.')

      if (!accepted.joined) {
        setStep('idle')
        const destination = await openDialog({
          directory: true,
          multiple: false,
          title: 'Choose where to save the shared project',
        })
        if (typeof destination !== 'string') return
        setStep('accepting')
        accepted = await syncAccept(pasted, destination)
        if (!accepted) throw new Error('Wobu returned no result for this ticket.')
      }

      if (!accepted.root) {
        throw new Error('The project joined, but its local folder could not be found.')
      }
      setStep('opening')
      await onOpen(accepted.root)
      onClose()
    } catch (reason) {
      if (errorCode(reason) !== 'cancelled') setError(errorMessage(reason))
      setStep('idle')
      setCancelling(false)
    }
  }

  async function cancel() {
    setCancelling(true)
    try {
      await syncAcceptCancel()
    } catch (reason) {
      setError(errorMessage(reason))
      setCancelling(false)
    }
  }

  const progress =
    step === 'probing'
      ? 'Checking the ticket and looking for an existing copy…'
      : step === 'accepting'
        ? 'Connecting to the other machine and copying the project…'
        : 'Opening the shared project…'

  return (
    <Modal
      titleId="accept-ticket-title"
      descriptionId="accept-ticket-description"
      onClose={onClose}
      busy={busy}
      busyMessage={busy ? progress : undefined}
    >
      <h2 id="accept-ticket-title">Accept a share ticket</h2>
      <p id="accept-ticket-description">
        Paste a ticket from someone you trust. If this machine does not already have the project,
        you will choose a folder and Wobu will create a local copy there.
      </p>

      <div className="share-warning" role="note">
        <strong>A ticket contains access credentials.</strong>
        <p>
          Keep it private. Tickets do not expire and cannot be revoked individually. The sender can
          stop sharing, but that does not remove copies already downloaded.
        </p>
      </div>

      <div className="field">
        <label htmlFor="accept-ticket">Share ticket</label>
        <textarea
          id="accept-ticket"
          rows={4}
          value={token}
          placeholder="Paste the ticket here…"
          disabled={busy}
          data-modal-initial-focus
          onChange={(event) => setToken(event.target.value)}
        />
      </div>

      {busy && (
        <div className="accept-progress" role="status" aria-live="polite">
          <div className="lch-bar-track">
            <div className="lch-bar-fill is-indeterminate" />
          </div>
          <p>{progress}</p>
        </div>
      )}

      {error && (
        <div className="sheet-err" role="alert">
          {error}
        </div>
      )}

      <div className="sheet-actions">
        {step === 'accepting' ? (
          <button className="btn" onClick={() => void cancel()} disabled={cancelling}>
            {cancelling ? 'Cancelling…' : 'Cancel transfer'}
          </button>
        ) : (
          <button className="btn btn-ghost" onClick={onClose} disabled={busy}>
            Cancel
          </button>
        )}
        <button className="btn btn-primary" onClick={() => void accept()} disabled={busy}>
          <Icon name="link" size="sm" />
          Accept ticket
        </button>
      </div>
    </Modal>
  )
}
