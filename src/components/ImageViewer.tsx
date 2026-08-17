import { useId } from 'react'
import { Modal } from './Modal'

/** One viewport-bounded original, shared by every image details surface. */
export function ImageViewer({
  src,
  alt,
  title,
  description,
  onClose,
}: {
  src: string
  alt: string
  title: string
  description: string
  onClose: () => void
}) {
  const id = useId()
  const titleId = `${id}-title`
  const descriptionId = `${id}-description`

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
      <img src={src} alt={alt} />
      <button
        className="ibtn image-viewer-close"
        type="button"
        onClick={onClose}
        aria-label="Close full-size image"
        data-modal-initial-focus
      >
        ×
      </button>
    </Modal>
  )
}
