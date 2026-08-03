import type { ReactNode } from 'react'
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

/**
 * The fixed slot an entity row keeps for its picture, whether it has one.
 *
 * The box is sized in CSS (`styles/thumbs.css`) and is present in the DOM
 * either way, so a list of a thousand rows has the same geometry before and
 * after its thumbnails resolve. That is not a detail: a row that grows when its
 * image lands moves everything below it, and in a virtualized list it also
 * invalidates the scroll arithmetic that put the row there in the first place.
 * Nothing here is allowed to depend on the image's own dimensions.
 *
 * The picture is decoration and is marked as such. Every list this appears in
 * already says the entity's name in text beside it, so an `alt` here would only
 * make screen readers announce it twice — and would change the accessible name
 * of the row button it sits inside.
 *
 * `path` rather than a node id, and no hook of its own: the batching lives one
 * level up in the list (`useNodeThumbs`), which is what keeps this from
 * becoming one IPC per row.
 */
export function NodeThumbnail({
  path,
  fallback,
  className,
}: {
  /** Absolute thumbnail path, or `null` for "no picture, or not resolved yet". */
  path: string | null
  /** Drawn in the same box when there is no picture — usually the kind icon. */
  fallback: ReactNode
  className?: string
}) {
  const cls = ['node-thumb', path ? 'has-image' : 'is-empty', className].filter(Boolean).join(' ')
  return (
    <span className={cls} aria-hidden>
      {path ? <img src={convertFileSrc(path)} alt="" loading="lazy" decoding="async" /> : fallback}
    </span>
  )
}
