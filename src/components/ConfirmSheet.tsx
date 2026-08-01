import { Modal } from './Modal'

export function ConfirmSheet({
  title,
  body,
  confirmLabel,
  danger,
  busy,
  onCancel,
  onConfirm,
}: {
  title: string
  body: string
  confirmLabel: string
  danger?: boolean
  busy?: boolean
  onCancel: () => void
  onConfirm: () => void
}) {
  return (
    <Modal
      role="alertdialog"
      titleId="confirm-sheet-title"
      descriptionId="confirm-sheet-description"
      onClose={onCancel}
      busy={busy}
      busyMessage={busy ? 'Please wait for this operation to finish before closing.' : undefined}
    >
      <h2 id="confirm-sheet-title">{title}</h2>
      <p id="confirm-sheet-description">{body}</p>
      <div className="sheet-actions">
        <button
          className="btn btn-ghost"
          onClick={onCancel}
          disabled={busy}
          data-modal-initial-focus
        >
          Cancel
        </button>
        <button
          className={danger ? 'btn' : 'btn btn-primary'}
          style={danger ? { borderColor: '#7d2b36', color: '#f0b3bb' } : undefined}
          onClick={onConfirm}
          disabled={busy}
        >
          {confirmLabel}
        </button>
      </div>
    </Modal>
  )
}
