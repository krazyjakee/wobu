import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Generation } from '../lib/api'
import { summary } from '../test/fixtures'
import { HistoryMode } from './HistoryMode'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const generations: Generation[] = [
  {
    id: 'portrait-9',
    nodeId: 'kael',
    createdAt: '2026-08-02T12:00:00Z',
    preset: 'portrait',
    viewType: null,
    userPrompt: '',
    compiledPrompt: 'portrait nine',
    negativePrompt: '',
    backend: 'comfyui',
    model: 'flux-dev',
    seed: 9,
    params: {
      aspect: '3:4',
      width: 768,
      height: 1024,
      loras: [
        {
          nodeId: 'kael',
          contentHash: '0123456789abcdef',
          providerName: 'kael.safetensors',
          triggerToken: 'wobu_kael',
          strength: 0.8,
        },
        { providerName: 'malformed-applied' },
      ],
      loraDowngrades: [
        {
          nodeId: 'mira',
          contentHash: 'fedcba9876543210',
          state: 'model_mismatch',
          detail: 'The LoRA was trained for flux-schnell, not flux-dev.',
        },
        { nodeId: 'bad', contentHash: 'bad', state: 'malformed-downgrade' },
      ],
    },
    outputAssetIds: ['asset-9'],
    influenceSnapshot: { layers: [] },
  },
  {
    id: 'sheet-7',
    nodeId: 'kael',
    createdAt: '2026-07-01T12:00:00Z',
    preset: 'character_sheet',
    viewType: null,
    userPrompt: '',
    compiledPrompt: 'sheet seven',
    negativePrompt: '',
    backend: 'gemini',
    model: 'gemini-image',
    seed: 7,
    params: {},
    outputAssetIds: [],
    influenceSnapshot: { layers: [] },
  },
]

beforeEach(() => {
  h.invoke.mockReset()
  h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'generation_list_all') return Promise.resolve(generations)
    if (command === 'asset_thumb') return Promise.resolve(`/thumb-${args?.assetId}`)
    if (command === 'asset_original') return Promise.resolve(`/original-${args?.assetId}`)
    return Promise.resolve(null)
  })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('project generation history', () => {
  it('filters immutable receipts by preset and seed and opens their detail', async () => {
    render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <HistoryMode
          nodes={[summary({ id: 'kael', name: 'Kael' })]}
          readOnly={false}
          onJump={() => {}}
        />
      </QueryClientProvider>,
    )
    expect(
      await screen.findAllByRole('button', { name: /Open generation/ }, { timeout: 5_000 }),
    ).toHaveLength(2)
    const portrait = screen.getByRole('button', { name: 'Open generation portrait-9' })
    await waitFor(() => expect(portrait.querySelector('img')).toHaveAttribute('loading', 'lazy'))
    expect(portrait.querySelector('img')).toHaveAttribute('alt', 'portrait nine')
    expect(screen.getByText('No preview')).toBeInTheDocument()
    expect(h.invoke).not.toHaveBeenCalledWith('asset_original', expect.anything())
    fireEvent.change(screen.getByLabelText('Filter history by preset'), {
      target: { value: 'portrait' },
    })
    expect(screen.getAllByRole('button', { name: /Open generation/ })).toHaveLength(1)
    fireEvent.change(screen.getByLabelText('Filter history by seed'), { target: { value: '9' } })
    fireEvent.click(screen.getByRole('button', { name: 'Open generation portrait-9' }))
    const detail = await screen.findByRole('dialog', { name: 'Generation details' })
    expect(detail).toHaveAccessibleDescription(
      `Kael · portrait · comfyui / flux-dev · seed 9 · ${new Date(generations[0]!.createdAt).toLocaleString()}`,
    )
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('asset_original', { assetId: 'asset-9' }),
    )
  })

  it('shows applied and downgraded LoRAs while ignoring malformed receipt entries', async () => {
    render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <HistoryMode
          nodes={[summary({ id: 'kael', name: 'Kael' })]}
          readOnly={false}
          onJump={() => {}}
        />
      </QueryClientProvider>,
    )
    fireEvent.click(
      await screen.findByRole('button', { name: 'Open generation portrait-9' }, { timeout: 5_000 }),
    )

    const receipt = await screen.findByRole('region', { name: 'Recorded LoRA application' })
    expect(receipt).toHaveTextContent('kael.safetensors')
    expect(receipt).toHaveTextContent('0123456789ab… · strength 0.8 · node kael')
    expect(receipt).toHaveTextContent('model mismatch')
    expect(receipt).toHaveTextContent('The LoRA was trained for flux-schnell, not flux-dev.')
    expect(receipt).not.toHaveTextContent('malformed-applied')
    expect(receipt).not.toHaveTextContent('malformed-downgrade')
  })
})
