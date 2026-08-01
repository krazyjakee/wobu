import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Generation, JobPreview, JobProgress, QueueSnapshot } from '../../lib/api'
import { node as buildNode } from '../../test/fixtures'
import { ConceptsPane } from './ConceptsPane'

const h = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, receive: (event: { payload: unknown }) => void) => {
    h.listeners.set(name, receive)
    return Promise.resolve(() => h.listeners.delete(name))
  },
}))

const node = buildNode({ id: 'kael', name: 'Kael' })

function generation(over: Partial<Generation> & Pick<Generation, 'id'>): Generation {
  return {
    nodeId: node.id,
    createdAt: '2026-08-01T12:00:00Z',
    preset: 'portrait',
    viewType: null,
    userPrompt: '',
    compiledPrompt: 'ash-lit wanderer',
    negativePrompt: '',
    backend: 'comfyui',
    model: 'flux-dev',
    seed: 42,
    params: {},
    outputAssetIds: ['asset-1'],
    influenceSnapshot: { layers: [] },
    ...over,
  }
}

const emptyQueue: QueueSnapshot = { jobs: [], queued: 0, running: 0, retrying: 0 }
let history: Generation[] = []

function open(queue: QueueSnapshot = emptyQueue) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={qc}>
      <ConceptsPane node={node} queue={queue} />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  history = []
  h.invoke.mockReset()
  h.listeners.clear()
  h.invoke.mockImplementation((command: string, args?: { assetId?: string }) => {
    if (command === 'generation_list') return Promise.resolve(history)
    if (command === 'asset_thumb') return Promise.resolve(`/thumb-${args?.assetId}`)
    if (command === 'asset_original') return Promise.resolve(`/original-${args?.assetId}`)
    if (command === 'job_cancel') return Promise.resolve(true)
    return Promise.resolve(null)
  })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('generation history', () => {
  it('renders backend order, reveals prompt and seed, and loads one original only on click', async () => {
    history = [
      generation({ id: 'new', createdAt: '2026-08-02T12:00:00Z', compiledPrompt: 'new prompt', seed: 9 }),
      generation({ id: 'old', outputAssetIds: ['asset-2'], compiledPrompt: 'old prompt', seed: 7 }),
    ]
    open()

    const tiles = await screen.findAllByRole('button', { name: /Open generation from/ })
    expect(tiles.map((tile) => tile.getAttribute('aria-label'))).toEqual([
      'Open generation from 2026-08-02T12:00:00Z',
      'Open generation from 2026-08-01T12:00:00Z',
    ])
    expect(screen.getByText('new prompt')).toBeTruthy()
    expect(screen.getByText('seed 9')).toBeTruthy()
    expect(h.invoke).not.toHaveBeenCalledWith('asset_original', expect.anything())

    fireEvent.click(tiles[0] as HTMLElement)
    const dialog = await screen.findByRole('dialog', { name: 'Full-resolution concept' })
    expect(h.invoke).toHaveBeenCalledWith('asset_original', { assetId: 'asset-1' })
    expect(within(dialog).getByRole('img').getAttribute('src')).toBe('asset:///original-asset-1')
  })
})

describe('live generation tiles', () => {
  const running: QueueSnapshot = {
    queued: 0,
    running: 1,
    retrying: 0,
    jobs: [
      {
        id: 'job-1',
        kind: 'generate',
        label: 'Generate Kael',
        subjectId: node.id,
        state: 'running',
        attempt: 1,
        elapsedMs: 800,
      },
    ],
  }

  it('shows latent previews and measured progress for this node', async () => {
    open(running)
    await waitFor(() => expect(h.listeners.has('job:preview')).toBe(true))

    act(() => {
      h.listeners.get('job:preview')?.({
        payload: { id: 'job-1', image: 'data:image/webp;base64,preview', step: 12 } satisfies JobPreview,
      })
      h.listeners.get('job:progress')?.({
        payload: { id: 'job-1', done: 12, total: 30, note: 'sampling 12/30' } satisfies JobProgress,
      })
    })

    expect(screen.getByAltText('Live preview for Generate Kael')).toHaveProperty(
      'src',
      'data:image/webp;base64,preview',
    )
    expect(screen.getByLabelText('40% complete')).toBeTruthy()
    expect(screen.getByText('sampling 12/30')).toBeTruthy()
  })

  it('cancels the queue job from its own tile', async () => {
    open(running)
    fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(h.invoke).toHaveBeenCalledWith('job_cancel', { jobId: 'job-1' }))
  })

  it('keeps a failed generation visible with the provider error', async () => {
    open({
      jobs: [
        {
          ...running.jobs[0]!,
          state: 'failed',
          failure: {
            code: 'provider.unavailable',
            message: 'ComfyUI stopped answering.',
            retryable: true,
            billed: 'nothing',
          },
          retryHeld: false,
        },
      ],
      queued: 0,
      running: 0,
      retrying: 0,
    })
    expect(await screen.findByText('ComfyUI stopped answering.')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull()
  })
})
