import { act, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { MediaAudition } from './MediaAudition'
import type { MediaAudition as Audition } from '../../../lib/api/narrativeMedia'
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string) => `asset://${path}` }))
const audition = {
  path: '/assets/a.wav',
  current: true,
  take: {
    audio: { hash: 'a', bytes: 4 },
    info: { duration_ms: 1000, sample_rate: 8000 },
    spoken_text: 'Hello',
    row: { key: { locale: 'en' } },
  },
  timing: null,
} as Audition
const create = vi.fn(() => 'blob:take')
const revoke = vi.fn()
beforeEach(() => {
  vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => {})
  create.mockClear()
  revoke.mockClear()
  vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: create, revokeObjectURL: revoke }))
})
afterEach(() => vi.unstubAllGlobals())
it('loads only the requested file and releases its Blob URL when the player closes', async () => {
  const fetcher = vi.fn().mockResolvedValue(new Response(new Uint8Array(4)))
  vi.stubGlobal('fetch', fetcher)
  const view = render(<MediaAudition audition={audition} />)
  await waitFor(() =>
    expect(view.container.querySelector('audio')).toHaveAttribute('src', 'blob:take'),
  )
  expect(fetcher).toHaveBeenCalledWith(
    'asset:///assets/a.wav',
    expect.objectContaining({ cache: 'no-store' }),
  )
  view.unmount()
  expect(fetcher.mock.calls[0]![1].signal.aborted).toBe(true)
  expect(revoke).toHaveBeenCalledWith('blob:take')
  expect(HTMLMediaElement.prototype.pause).toHaveBeenCalled()
})
it('aborts an old take and never publishes its late bytes after changing takes', async () => {
  let finish!: (value: ArrayBuffer) => void
  const pending = new Promise<ArrayBuffer>((resolve) => {
    finish = resolve
  })
  const fetcher = vi
    .fn()
    .mockResolvedValueOnce({
      ok: true,
      body: new ReadableStream({
        async start(c) {
          c.enqueue(new Uint8Array(await pending))
          c.close()
        },
      }),
    })
    .mockResolvedValueOnce({ ok: false })
  vi.stubGlobal('fetch', fetcher)
  const view = render(<MediaAudition audition={audition} />)
  await waitFor(() => expect(fetcher).toHaveBeenCalledTimes(1))
  view.rerender(<MediaAudition audition={{ ...audition, path: '/assets/b.wav' }} />)
  await screen.findByText('The recording file could not be read.')
  await act(async () => finish(new ArrayBuffer(4)))
  expect(fetcher.mock.calls[0]![1].signal.aborted).toBe(true)
  expect(create).not.toHaveBeenCalled()
  expect(view.container.querySelector('audio')).not.toHaveAttribute('src')
})
it('refuses a file replaced with a different size after backend verification', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(new Uint8Array(8))))
  render(<MediaAudition audition={audition} />)
  await screen.findByText(/recording file size changed/)
  expect(create).not.toHaveBeenCalled()
})
