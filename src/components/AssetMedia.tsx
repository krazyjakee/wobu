import { convertFileSrc } from '@tauri-apps/api/core'
import { useAssetThumb } from '../lib/queries'

/**
 * A thumbnail request owned by the media that displays it.
 *
 * Consumers keep this component inside their virtualized/card window, so an
 * off-screen tile never mounts and never asks Rust for a thumbnail. Originals
 * deliberately do not go through this component: comparison and history load
 * those only after the user opens the corresponding surface.
 */
export function LazyAssetThumbnail({
  assetId,
  alt,
  loadingLabel,
  missingLabel,
  errorLabel,
}: {
  assetId: string | null
  alt: string
  loadingLabel: string
  missingLabel: string
  errorLabel: string
}) {
  const thumb = useAssetThumb(assetId)

  if (thumb.data) {
    return <img src={convertFileSrc(thumb.data)} alt={alt} loading="lazy" decoding="async" />
  }
  if (!assetId) return <span>{missingLabel}</span>
  return <span>{thumb.isError ? errorLabel : loadingLabel}</span>
}
