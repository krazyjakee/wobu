import { useId } from 'react'
import { useContextMenu } from '../hooks/useContextMenu'
import { ImageContextMenu } from './ImageContextMenu'
import { Modal } from './Modal'

/** One viewport-bounded original, shared by every image details surface. */
export function ImageViewer({
  src,
  alt,
  title,
  description,
  actions,
  onClose,
}: {
  src: string
  alt: string
  title: string
  description: string
  actions?: {
    assetId: string
    originalPath: string | null
    menuLabel: string
    deleteLabel: string
    deleteDisabledReason?: string | null
    onDelete: () => void
  }
  onClose: () => void
}) {
  const id = useId()
  const titleId = `${id}-title`
  const descriptionId = `${id}-description`
  const menu = useContextMenu<void>()

  return (
    <Modal
      className="image-viewer"
      scrimClassName="image-viewer-scrim"
      titleId={titleId}
      descriptionId={descriptionId}
      onClose={onClose}
    >
      <h2 id={titleId} className="modal-sr-only">
        {title}
      </h2>
      <p id={descriptionId} className="modal-sr-only">
        {description}
      </p>
      <img
        src={src}
        alt={alt}
        tabIndex={actions ? 0 : undefined}
        {...(actions ? menu.trigger() : {})}
      />
      <button
        className="ibtn image-viewer-close"
        type="button"
        onClick={onClose}
        aria-label="Close full-size image"
        data-modal-initial-focus
      >
        ×
      </button>
      {actions && menu.anchor && (
        <ImageContextMenu
          anchor={menu.anchor}
          onClose={menu.close}
          assetId={actions.assetId}
          originalPath={actions.originalPath}
          label={actions.menuLabel}
          deleteLabel={actions.deleteLabel}
          deleteDisabledReason={actions.deleteDisabledReason}
          onDelete={actions.onDelete}
        />
      )}
    </Modal>
  )
}
