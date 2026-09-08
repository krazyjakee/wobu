import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { NarrativeMedia } from './NarrativeMedia'
import { type MediaView, type MediaTake } from '../../lib/api/narrativeMedia'
import { advanceProjectSession } from '../../lib/projectSession'
import { useMediaNotes } from './media/notesDrafts'
const h = vi.hoisted(() => ({ invoke: vi.fn(), save: vi.fn(), open: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: h.open, save: h.save }))
let view: MediaView
function take(): MediaTake {
  return {
    row: structuredClone(view.rows[0]!),
    spoken_text: 'The harbor is quiet.',
    audio: { path: 'assets/media/clip.wav', hash: 'hash', bytes: 16044 },
    timing: { path: 'assets/media/timing.json', hash: 'timing', bytes: 100 },
    info: { channels: 1, sample_rate: 8000, frames: 8000, duration_ms: 1000 },
    actor: 'Actor',
  }
}
beforeEach(() => {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation(async () => new Response(new Uint8Array(16044))),
  )
  vi.stubGlobal(
    'URL',
    Object.assign(URL, {
      createObjectURL: vi.fn(() => 'blob:recording'),
      revokeObjectURL: vi.fn(),
    }),
  )
  h.invoke.mockReset()
  h.save.mockReset()
  h.open.mockReset()
  useMediaNotes.setState({ entries: {} })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  view = {
    policy: { version: 1, required: {}, notes: {}, timing: [] },
    policy_guard: 'policy-v1',
    locale: 'en',
    bindings: {},
    diagnostics: [],
    rows: [
      {
        version: 1,
        key: { id: 'line', locale: 'en', form: 'other' },
        source: {
          id: 'line',
          slot: 'slot',
          container: 'asset',
          speaker: 'Mara',
          text: 'The harbor is quiet.',
          revision: 'source-v1',
          guard: 'locked-v1',
          context: 'Harbor',
          delivery_notes: 'Quiet',
          placeholders: [],
          ready: true,
        },
        origin: 'en',
        translation_guard: null,
        text: 'The harbor is quiet.',
        notes: { pronunciation: '', delivery: 'Quiet' },
        parameters: {},
        media_guard: null,
        audio_path: 'recordings/line.wav',
        timing_path: null,
        audio_hash: null,
        timing_hash: null,
        ready: true,
      },
    ],
  }
  h.open.mockResolvedValue('/prepared')
  h.invoke.mockImplementation(async (command: string, args: Record<string, unknown>) => {
    if (command === 'narrative_media_get') return structuredClone(view)
    if (command === 'narrative_media_preview')
      return [{ key: 'unknown', code: 'unknown', message: 'Unknown row will be skipped.' }]
    if (command === 'narrative_media_import') {
      view.bindings['en/line/other'] = { version: 1, key: view.rows[0]!.key, history: [take()] }
      return { applied: ['en/line/other'], diagnostics: [], conflicts: {} }
    }
    if (command === 'narrative_media_audition')
      return {
        path: '/world/assets/media/clip.wav',
        take: take(),
        current: true,
        timing: {
          version: 1,
          audio_hash: 'hash',
          duration_ms: 1000,
          cues: [{ start_ms: 100, end_ms: 800, kind: 'viseme', value: 'aa' }],
        },
      }
    if (command === 'narrative_media_policy') {
      if (args.expected !== view.policy_guard) throw new Error('Recording policy changed')
      view.policy = args.policy as MediaView['policy']
      return null
    }
    return 'recording manifest'
  })
})
function draw(readOnly = false) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return {
    ...render(
      <QueryClientProvider client={client}>
        <NarrativeMedia projectKey="/world" readOnly={readOnly} onClose={() => {}} />
      </QueryClientProvider>,
    ),
    client,
  }
}
it('previews guarded partial imports and auditions actual timed media through one explicit request', async () => {
  draw()
  await screen.findByText('The harbor is quiet.')
  expect(screen.getByRole('button', { name: 'Import eligible takes' })).toBeDisabled()
  fireEvent.change(screen.getByLabelText('Manifest contents'), {
    target: { value: 'returned recording' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Choose prepared files directory…' }))
  await screen.findByText('/prepared')
  fireEvent.click(screen.getByRole('button', { name: 'Preview media import' }))
  await screen.findByText(/Unknown row will be skipped/)
  fireEvent.click(screen.getByRole('button', { name: 'Import eligible takes' }))
  await screen.findByText(/1 takes imported/)
  expect(h.invoke.mock.calls.filter(([name]) => name === 'narrative_media_audition')).toHaveLength(
    0,
  )
  fireEvent.click(screen.getByRole('button', { name: 'Audition latest take' }))
  const section = await screen.findByRole('region', { name: 'Recording audition' })
  const audio = section.querySelector('audio')!
  await waitFor(() => expect(audio).toHaveAttribute('src', 'blob:recording'))
  Object.defineProperty(audio, 'currentTime', { value: 0.25 })
  fireEvent.timeUpdate(audio)
  expect(within(section).getByText('viseme: aa (100–800 ms)')).toBeInTheDocument()
  fireEvent.error(audio)
  expect(within(section).getByRole('alert')).toHaveTextContent('Audio playback failed')
})
it('pages large catalogs and retains note drafts with their original guard through navigation and refresh', async () => {
  const original = view.rows[0]!
  view.rows = Array.from({ length: 51 }, (_, i) => ({
    ...original,
    key: { ...original.key, id: `line-${i}` },
    text: `Phrase ${i}`,
  }))
  const first = draw()
  await screen.findByText('51 matching of 51 recording rows')
  expect(document.querySelectorAll('.nrt-media-line')).toHaveLength(25)
  fireEvent.change(screen.getAllByLabelText('Pronunciation')[0]!, { target: { value: 'Har-bor' } })
  fireEvent.click(screen.getByRole('button', { name: 'Next page' }))
  fireEvent.click(screen.getByRole('button', { name: 'Previous page' }))
  expect(screen.getAllByLabelText('Pronunciation')[0]).toHaveValue('Har-bor')
  view.policy_guard = 'policy-v2'
  await first.client.invalidateQueries({ queryKey: ['narrative_media'] })
  fireEvent.click(screen.getAllByRole('button', { name: 'Save recording notes' })[0]!)
  await screen.findByText('Recording policy changed')
  expect(h.invoke).toHaveBeenCalledWith(
    'narrative_media_policy',
    expect.objectContaining({ expected: 'policy-v1' }),
  )
  first.unmount()
  draw()
  await screen.findByText('51 matching of 51 recording rows')
  expect(screen.getAllByLabelText('Pronunciation')[0]).toHaveValue('Har-bor')
})
it('saves explicit audio timing and fallback requirements and disables read-only writes', async () => {
  const first = draw()
  await screen.findByText('The harbor is quiet.')
  fireEvent.click(screen.getByLabelText('Require recordings for this locale in Release'))
  await waitFor(() =>
    expect(
      screen.getByLabelText('Require timing / lip-sync sidecars for this locale'),
    ).toBeEnabled(),
  )
  fireEvent.click(screen.getByLabelText('Require timing / lip-sync sidecars for this locale'))
  await waitFor(() => expect(view.policy.timing).toEqual(['en']))
  fireEvent.click(
    screen.getByLabelText('Explicitly allow text-only fallback for missing or outdated takes'),
  )
  await waitFor(() => expect(view.policy.required.en).toBe(true))
  first.unmount()
  draw(true)
  await screen.findByText('The harbor is quiet.')
  expect(screen.getByLabelText('Require recordings for this locale in Release')).toBeDisabled()
  expect(screen.getAllByRole('button', { name: 'Save recording notes' })[0]).toBeDisabled()
})
it('does not export or leak a result after the project changes while choosing a destination', async () => {
  let choose!: (value: string) => void
  h.save.mockImplementation(
    () =>
      new Promise((resolve) => {
        choose = resolve
      }),
  )
  draw()
  await screen.findByText('The harbor is quiet.')
  fireEvent.click(screen.getByRole('button', { name: 'Export recording script…' }))
  advanceProjectSession()
  choose('/other/recording.csv')
  await waitFor(() => expect(h.save).toHaveBeenCalled())
  expect(h.invoke.mock.calls.filter(([name]) => name === 'narrative_media_export')).toHaveLength(0)
})
it('shows loading and failed readiness instead of a false all-clear', async () => {
  h.invoke.mockRejectedValue(new Error('Unreadable production record'))
  draw()
  expect(screen.getByRole('status')).toHaveTextContent('Reading recording readiness')
  await screen.findByText('Unreadable production record')
  expect(screen.queryByText(/matching of/)).toBeNull()
})
