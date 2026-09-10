import { useEffect, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import { useContextMenu } from '../../hooks/useContextMenu'
import * as api from '../../lib/api'
import type { Asset, AssetLink } from '../../lib/api'
import { ConfirmSheet } from '../ConfirmSheet'
import { ImageContextMenu } from '../ImageContextMenu'
import { ImageViewer } from '../ImageViewer'
import { Modal } from '../Modal'

export function ReferenceDetail({
  asset,
  link,
  nodeName,
  position,
  cover,
  thumbnailSrc,
  roleName,
  sizeLabel,
  readOnly,
  onDelete,
  onClose,
}: {
  asset: Asset | undefined
  link: AssetLink
  nodeName: string
  position: number
  cover: boolean
  thumbnailSrc: string | null
  roleName: string
  sizeLabel: string | null
  readOnly: boolean
  onDelete: () => void
  onClose: () => void
}) {
  const [originalPath, setOriginalPath] = useState<string | null>(null)
  const [originalSrc, setOriginalSrc] = useState<string | null>(null)
  const [loadingOriginal, setLoadingOriginal] = useState(true)
  const [openError, setOpenError] = useState<string | null>(null)
  const [fullSize, setFullSize] = useState(false)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const menu = useContextMenu<void>()

  useEffect(() => {
    let disposed = false
    void api
      .assetOriginal(link.assetId)
      .then((path) => {
        if (disposed) return
        if (path) {
          setOriginalPath(path)
          setOriginalSrc(convertFileSrc(path))
        } else setOpenError('The original is no longer in the project folder.')
      })
      .catch((error) => {
        if (!disposed) setOpenError(api.errorMessage(error))
      })
      .finally(() => {
        if (!disposed) setLoadingOriginal(false)
      })
    return () => {
      disposed = true
    }
  }, [link.assetId])

  const previewSrc = originalSrc ?? thumbnailSrc
  const description = `${nodeName} · ${roleName} · reference ${position}`
  const imageAlt = `${roleName} reference for ${nodeName}`
  const deleteDisabledReason = readOnly
    ? 'This project folder is read-only, so the reference cannot be deleted.'
    : null
  const requestDelete = () => {
    setFullSize(false)
    setConfirmDelete(true)
  }

  return (
    <Modal
      className="generation-detail reference-detail"
      scrimClassName="generation-detail-scrim"
      titleId="reference-detail-title"
      descriptionId="reference-detail-description"
      onClose={onClose}
    >
      <header className="generation-detail-head">
        <div>
          <h2 id="reference-detail-title">Reference details</h2>
          <p id="reference-detail-description">{description}</p>
        </div>
        <button
          className="ibtn"
          type="button"
          onClick={onClose}
          aria-label="Close reference details"
          data-modal-initial-focus
        >
          ×
        </button>
      </header>

      <div className="reference-detail-body">
        <section
          className="reference-detail-preview"
          aria-label="Reference image"
          tabIndex={-1}
          {...menu.trigger()}
        >
          {previewSrc ? (
            <button
              className="reference-detail-image"
              type="button"
              disabled={!originalSrc}
              onClick={() => setFullSize(true)}
              aria-label="View reference image full size"
            >
              <img src={previewSrc} alt={imageAlt} />
              <span>
                {loadingOriginal
                  ? 'Loading original…'
                  : originalSrc
                    ? 'View full size'
                    : 'Original unavailable'}
              </span>
            </button>
          ) : (
            <div className="reference-detail-image is-missing">
              {loadingOriginal ? 'Loading original…' : 'Image unavailable'}
            </div>
          )}
          {openError && <p className="inline-error">Could not open it: {openError}</p>}
          {menu.anchor && (
            <ImageContextMenu
              anchor={menu.anchor}
              onClose={menu.close}
              assetId={link.assetId}
              originalPath={originalPath}
              label={`Actions for reference ${position} image`}
              deleteLabel="Delete reference…"
              deleteDisabledReason={deleteDisabledReason}
              onDelete={requestDelete}
            />
          )}
        </section>

        <section className="reference-detail-meta" aria-label="Reference information">
          <h3>How this image is used</h3>
          <dl>
            <div>
              <dt>Role</dt>
              <dd>{roleName}</dd>
            </div>
            <div>
              <dt>Influence</dt>
              <dd>{Math.round(link.weight * 100)}%</dd>
            </div>
            <div>
              <dt>Status</dt>
              <dd>{link.enabled ? 'Active' : 'Muted'}</dd>
            </div>
            <div>
              <dt>Cover</dt>
              <dd>{cover ? 'Yes' : 'No'}</dd>
            </div>
          </dl>

          <h3>Image information</h3>
          <dl>
            {asset && (
              <>
                <div>
                  <dt>Dimensions</dt>
                  <dd>
                    {asset.width}×{asset.height}
                  </dd>
                </div>
                <div>
                  <dt>Size</dt>
                  <dd>{sizeLabel}</dd>
                </div>
                <div>
                  <dt>Added</dt>
                  <dd>{new Date(asset.createdAt).toLocaleString()}</dd>
                </div>
                <div>
                  <dt>Type</dt>
                  <dd>{asset.kind}</dd>
                </div>
              </>
            )}
            <div>
              <dt>Asset ID</dt>
              <dd>
                <code>{link.assetId}</code>
              </dd>
            </div>
          </dl>
        </section>
      </div>

      {fullSize && originalSrc && (
        <ImageViewer
          src={originalSrc}
          alt={imageAlt}
          title="Full-size reference image"
          description={`The original ${roleName.toLocaleLowerCase()} reference for ${nodeName}. Press Escape, or use Close, to go back to the reference details.`}
          actions={{
            assetId: link.assetId,
            originalPath,
            menuLabel: `Actions for reference ${position} image`,
            deleteLabel: 'Delete reference…',
            deleteDisabledReason,
            onDelete: requestDelete,
          }}
          onClose={() => setFullSize(false)}
        />
      )}

      {confirmDelete && (
        <ConfirmSheet
          title="Delete this reference?"
          body="It will no longer influence this entity. The image stays in the Asset Library and anywhere else it is used."
          confirmLabel="Delete reference"
          danger
          onCancel={() => setConfirmDelete(false)}
          onConfirm={() => {
            onDelete()
            onClose()
          }}
        />
      )}
    </Modal>
  )
}
