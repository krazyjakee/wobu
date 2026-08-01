import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Asset, ImportedAsset } from './api'
import { ASSET_TRANSFER_CHUNK_BYTES, ASSET_TRANSFER_MAX_BYTES, assetImportBytes } from './api'

type InvokeCall = {
  command: string
  bytes: number | null
  headers?: Record<string, string>
}

const h = vi.hoisted(() => ({ calls: [] as InvokeCall[] }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (
    command: string,
    body?: Record<string, unknown> | ArrayBuffer,
    options?: { headers?: Record<string, string> },
  ) => {
    h.calls.push({
      command,
      bytes: body instanceof ArrayBuffer ? body.byteLength : null,
      headers: options?.headers,
    })
    if (command === 'asset_import_transfer_begin') {
      const totalBytes = Number((body as Record<string, unknown>).totalBytes)
      return Promise.resolve({ transferId: 'transfer-1', receivedBytes: 0, totalBytes })
    }
    if (command === 'asset_import_transfer_chunk') {
      const offset = Number(options?.headers?.['x-wobu-offset'])
      const totalBytes = largeBlobSize
      return Promise.resolve({
        transferId: 'transfer-1',
        receivedBytes: offset + (body as ArrayBuffer).byteLength,
        totalBytes,
      })
    }
    if (command === 'asset_import_transfer_finish') return Promise.resolve(imported())
    if (command === 'asset_import_transfer_cancel') return Promise.resolve(null)
    return Promise.reject(new Error(`unexpected command ${command}`))
  },
}))

let largeBlobSize = 0

function imported(): ImportedAsset {
  const asset: Asset = {
    id: 'pasted',
    hash: 'abc',
    kind: 'reference',
    relPath: 'assets/originals/ab/abc.png',
    thumbPath: null,
    mime: 'image/png',
    width: 4096,
    height: 4096,
    bytes: largeBlobSize,
    createdAt: '2026-08-01T12:00:00Z',
  }
  return { asset, deduped: false }
}

function virtualBlob(size: number) {
  largeBlobSize = size
  let wholeReads = 0
  let largestSlice = 0
  const blob = {
    size,
    arrayBuffer: () => {
      wholeReads += 1
      throw new Error('the whole Blob must never be read')
    },
    slice: (start: number, end: number) => {
      const bytes = end - start
      largestSlice = Math.max(largestSlice, bytes)
      return { arrayBuffer: () => Promise.resolve(new ArrayBuffer(bytes)) }
    },
  } as unknown as Blob
  return { blob, wholeReads: () => wholeReads, largestSlice: () => largestSlice }
}

beforeEach(() => {
  h.calls.length = 0
  largeBlobSize = 0
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

afterEach(() => {
  vi.restoreAllMocks()
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

describe('raw pasted-image transfer', () => {
  it('moves a representative 64 MiB file in bounded raw chunks without Array.from', async () => {
    const source = virtualBlob(64 * 1024 * 1024)
    const progress: number[] = []
    const arrayFrom = vi.spyOn(Array, 'from')

    await assetImportBytes(source.blob, 'reference', {
      onProgress: (value) => progress.push(value.receivedBytes),
    })

    const chunks = h.calls.filter((call) => call.command === 'asset_import_transfer_chunk')
    expect(chunks).toHaveLength(64)
    expect(Math.max(...chunks.map((call) => call.bytes ?? 0))).toBe(ASSET_TRANSFER_CHUNK_BYTES)
    expect(source.largestSlice()).toBe(ASSET_TRANSFER_CHUNK_BYTES)
    expect(source.wholeReads()).toBe(0)
    expect(arrayFrom).not.toHaveBeenCalled()
    expect(progress.at(-1)).toBe(64 * 1024 * 1024)
    expect(h.calls.at(-1)?.command).toBe('asset_import_transfer_finish')
  })

  it('cancels between acknowledged chunks and never finishes the session', async () => {
    const source = virtualBlob(3 * ASSET_TRANSFER_CHUNK_BYTES)
    const controller = new AbortController()

    await expect(
      assetImportBytes(source.blob, 'reference', {
        signal: controller.signal,
        onProgress: (progress) => {
          if (progress.receivedBytes >= ASSET_TRANSFER_CHUNK_BYTES) controller.abort()
        },
      }),
    ).rejects.toMatchObject({ name: 'AbortError' })

    expect(h.calls.map((call) => call.command)).toEqual([
      'asset_import_transfer_begin',
      'asset_import_transfer_chunk',
      'asset_import_transfer_cancel',
    ])
  })

  it('rejects beyond the declared bound before allocating or invoking Rust', async () => {
    const source = virtualBlob(ASSET_TRANSFER_MAX_BYTES + 1)

    await expect(assetImportBytes(source.blob)).rejects.toThrow('512 MiB')
    expect(source.largestSlice()).toBe(0)
    expect(source.wholeReads()).toBe(0)
    expect(h.calls).toEqual([])
  })
})
