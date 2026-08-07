import { invoke } from '@tauri-apps/api/core'
import { call, isTauri } from './call'
import type { Asset, AssetKind, AssetRole, AssetUsage, WobuNode } from './model'
/* ── domain types ─────────────────────────────────────────────────────────── */

/**
 * What an import did, as opposed to what it produced.
 *
 * `deduped` is the only part a caller cannot work out for itself: the asset
 * comes back identical whether the bytes were written or were already there,
 * which is exactly what content addressing is for.
 */
export interface ImportedAsset {
  asset: Asset
  /** True when the picture was already in the folder and nothing was written. */
  deduped: boolean
}

/**
 * Import a file by path — a drop, or a file picker result.
 *
 * The path is read and then discarded. What the file was called has no bearing
 * on where it lands or on the id it gets, so importing the same picture twice
 * under two names is a no-op the second time.
 *
 * Rejects with `asset.not_an_image` for anything that is not PNG, JPEG, GIF or
 * WebP; the format is read out of the file's header, not its extension.
 */
export const assetImport = (path: string, kind: AssetKind = 'reference') =>
  call<ImportedAsset>('asset_import', { path, kind })

export const ASSET_TRANSFER_CHUNK_BYTES = 1024 * 1024

export const ASSET_TRANSFER_MAX_BYTES = 512 * 1024 * 1024

export interface AssetTransferProgress {
  transferId: string
  receivedBytes: number
  totalBytes: number
}

export interface AssetTransferOptions {
  signal?: AbortSignal
  onProgress?: (progress: AssetTransferProgress) => void
}

/**
 * Import a paste/browser drop without ever expanding it into a JSON number
 * array—or reading the whole Blob into a second webview buffer.
 *
 * Tauri's top-level ArrayBuffer invoke body is binary IPC. Chunks are strictly
 * backpressured: the next slice is not read until Rust has appended the current
 * one to its temp file. The desktop shell therefore holds at most one 1 MiB JS
 * chunk and one 1 MiB Rust IPC body in addition to the browser-owned Blob.
 */
export async function assetImportBytes(
  blob: Blob,
  kind: AssetKind = 'reference',
  options: AssetTransferOptions = {},
): Promise<ImportedAsset> {
  if (blob.size <= 0 || blob.size > ASSET_TRANSFER_MAX_BYTES) {
    throw new Error(
      `Pasted images must be between 1 byte and ${ASSET_TRANSFER_MAX_BYTES / 1024 / 1024} MiB.`,
    )
  }
  throwIfAssetTransferAborted(options.signal)
  const started = await call<AssetTransferProgress>('asset_import_transfer_begin', {
    totalBytes: blob.size,
    kind,
  })
  options.onProgress?.(started)

  try {
    for (let offset = 0; offset < blob.size; offset += ASSET_TRANSFER_CHUNK_BYTES) {
      throwIfAssetTransferAborted(options.signal)
      const end = Math.min(blob.size, offset + ASSET_TRANSFER_CHUNK_BYTES)
      const chunk = await blob.slice(offset, end).arrayBuffer()
      throwIfAssetTransferAborted(options.signal)
      const progress = await rawCall<AssetTransferProgress>('asset_import_transfer_chunk', chunk, {
        'x-wobu-transfer-id': started.transferId,
        'x-wobu-offset': String(offset),
      })
      options.onProgress?.(progress)
    }
    throwIfAssetTransferAborted(options.signal)
    return await call<ImportedAsset>('asset_import_transfer_finish', {
      transferId: started.transferId,
    })
  } catch (error) {
    await call<void>('asset_import_transfer_cancel', { transferId: started.transferId }).catch(
      () => undefined,
    )
    throw error
  }
}

function rawCall<T>(cmd: string, body: ArrayBuffer, headers: Record<string, string>): Promise<T> {
  if (!isTauri()) {
    return Promise.reject(
      new Error(
        `Not running inside Tauri — the "${cmd}" command is unavailable. Launch with \`npm run tauri dev\`.`,
      ),
    )
  }
  return invoke<T>(cmd, body, { headers })
}

function throwIfAssetTransferAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted) throw new DOMException('The image import was cancelled.', 'AbortError')
}

/** Every blob in the open project, newest first. */
export const assetList = () => call<Asset[]>('asset_list')

/** Every node/role/cover using every asset, for library filters and details. */
export const assetUsageList = () => call<AssetUsage[]>('asset_usage_list')

/** Permanently delete one orphan; the backend refuses every linked/cover use. */
export const assetDelete = (assetId: string) => call<void>('asset_delete', { assetId })

/** Thumbnail path for grids; null when the asset is absent or cannot decode. */
export const assetThumb = (assetId: string) => call<string | null>('asset_thumb', { assetId })

/** Full-resolution path, fetched only when a viewer is opened. */
export const assetOriginal = (assetId: string) => call<string | null>('asset_original', { assetId })

/**
 * Attach a reference image to a node in a role.
 *
 * All four calls below return the saved node — attaching a reference is an edit
 * to that node's Markdown, so it goes through the same guarded write as any
 * other and can reject with `write.conflict` in exactly the same way.
 *
 * `weight` is 0.0–1.0 and defaults to 1.0; anything outside the range is
 * clamped rather than refused. Rejects with `asset.not_found` if the id names
 * no blob in this project — an asset id is derived from a file's hash, so one
 * that matches nothing here matches nothing anywhere.
 */
export const assetLink = (nodeId: string, assetId: string, role: AssetRole, weight?: number) =>
  call<WobuNode>('asset_link', { nodeId, assetId, role, weight })

/**
 * Detach one.
 *
 * The picture itself is untouched: assets are content-addressed and shared
 * between nodes, so removing the last link is not a reason to delete the file.
 * Rejects with `asset.not_found` when there is no such link, which is what a
 * panel showing a reference somebody else already removed will get.
 */
export const assetUnlink = (nodeId: string, assetId: string, role: AssetRole) =>
  call<WobuNode>('asset_unlink', { nodeId, assetId, role })

/** Choose the image on a node's card, or pass `null` to clear it. */
export const assetSetCover = (nodeId: string, assetId: string | null) =>
  call<WobuNode>('asset_set_cover', { nodeId, assetId })

/* ── influence ────────────────────────────────────────────────────────────── */
