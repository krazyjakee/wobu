import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Generation, ProjectSummary, QueueSnapshot } from '../lib/api'
import { useUI } from '../store/ui'
import { kindDef, kindIndex, summary } from '../test/fixtures'
import { ForgeMode } from './ForgeMode'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const nodes = [summary({ id: 'kael', name: 'Kael' }), summary({ id: 'mira', name: 'Mira' })]
const kinds = kindIndex([kindDef('character')])
const project: ProjectSummary = {
  id: 'project', name: 'Ashfall', path: '/project', onNetworkShare: false, readOnly: false,
  lastOpenedAt: null,
}
const queue: QueueSnapshot = { jobs: [], queued: 0, running: 0, retrying: 0 }
const preset = {
  id: 'portrait', label: 'Portrait', kinds: ['character'], defaultFor: ['character'],
  priorities: [], framing: 'portrait framing', aspect: '3:4', images: 1, views: [],
  imageConstraints: null,
}
const generations: Generation[] = [
  receipt('one', 'asset-one', 11, 'first portrait'),
  receipt('two', 'asset-two', 22, 'second portrait'),
  ...Array.from({ length: 58 }, (_, index) =>
    receipt(`extra-${index}`, `asset-extra-${index}`, 100 + index, `portrait variation ${index}`),
  ),
]

function receipt(id: string, assetId: string, seed: number, prompt: string): Generation {
  return {
    id, nodeId: 'kael', createdAt: '2026-08-01T12:00:00Z', preset: 'portrait',
    viewType: null, userPrompt: '', compiledPrompt: prompt, negativePrompt: '',
    backend: 'comfyui', model: 'flux-dev', seed, params: {}, outputAssetIds: [assetId],
    influenceSnapshot: { layers: [] },
  }
}

function Harness() {
  const selectedId = useUI((state) => state.selectedId)
  const selected = nodes.find((node) => node.id === selectedId) ?? null
  return (
    <ForgeMode
      project={project}
      nodes={nodes}
      selected={selected}
      kinds={kinds}
      queue={queue}
      onJump={() => {}}
    />
  )
}

beforeEach(() => {
  h.invoke.mockReset()
  useUI.setState({ selectedId: 'kael' })
  h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'preset_list') return Promise.resolve([preset])
    if (command === 'status_bar_backend') return Promise.resolve({
      image: { provider: 'comfyui', label: 'ComfyUI', model: 'flux-dev', contextTokens: null },
      text: { provider: 'anthropic', label: 'Anthropic', model: 'claude', contextTokens: 1 },
      health: { state: 'connected', externalQueue: 0 },
    })
    if (command === 'influence_resolve') return Promise.resolve({
      subjectId: args?.subjectId,
      preset,
      layers: [{
        layer: 'subject', nodeId: args?.subjectId, name: 'Kael', kind: 'character',
        reached: 'subject', distance: 0, weight: 1, slider: 1,
        fragments: [{
          layer: 'subject', nodeId: args?.subjectId, sourceName: 'Kael', section: 'appearance',
          text: 'ash-grey coat', assetId: null, weight: 1, target: 'prompt', sendable: true,
        }],
      }],
    })
    if (command === 'prompt_compile') return Promise.resolve({
      subjectId: args?.subjectId, preset, prompt: 'ash-grey coat', negative: '',
      spans: [{
        layer: 'subject', nodeId: args?.subjectId, sourceName: 'Kael', section: 'appearance',
        text: 'ash-grey coat', assetId: null, weight: 1, target: 'prompt', sendable: true,
      }],
      dropped: [], overflow: null,
    })
    if (command === 'image_reference_report') return Promise.resolve({
      buckets: [], layers: [], cost: null,
      spend: { ceilingUsdMicros: null, spentUsdMicros: 0, reservedUsdMicros: 0, remainingUsdMicros: null, pendingReservations: 0, oldestReservationAt: null, ledgerLocked: false },
      lockedSeed: null,
    })
    if (command === 'generation_list') {
      return Promise.resolve(args?.nodeId === 'kael' ? generations : [])
    }
    if (command === 'asset_thumb') return Promise.resolve(`/thumb-${args?.assetId}`)
    if (command === 'asset_original') return Promise.resolve(`/original-${args?.assetId}`)
    if (command === 'generate_start') return Promise.resolve('job-forge')
    return Promise.resolve(null)
  })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('Forge mode', () => {
  it('keeps one subject selected while moving between Forge histories', async () => {
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <Harness />
      </QueryClientProvider>,
    )
    fireEvent.change(screen.getByLabelText('Forge subject'), { target: { value: 'mira' } })
    expect(useUI.getState().selectedId).toBe('mira')
    expect(await screen.findByRole('region', { name: 'Forge results for Mira' })).toBeInTheDocument()
  })

  it('reuses the attributed Inspector and queues its one-axis batch', async () => {
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <Harness />
      </QueryClientProvider>,
    )
    expect(await screen.findByRole('region', { name: 'Forge generation controls' })).toBeInTheDocument()
    expect(screen.getByRole('region', { name: 'Compiled prompt' })).toHaveTextContent('ash-grey coat')
    fireEvent.change(screen.getByLabelText('Variant grid'), { target: { value: 'seed' } })
    const generate = await screen.findByRole('button', { name: 'Generate' })
    await waitFor(() => expect(generate).toBeEnabled())
    fireEvent.click(generate)
    await waitFor(() => expect(h.invoke).toHaveBeenCalledWith(
      'generate_start',
      expect.objectContaining({ subjectId: 'kael', grid: expect.objectContaining({ axis: 'seed' }) }),
    ))
  })

  it('virtualizes receipts and compares selected originals side by side', async () => {
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <Harness />
      </QueryClientProvider>,
    )
    fireEvent.click(await screen.findByRole('button', { name: 'Select generation one for comparison' }))
    fireEvent.click(screen.getByRole('button', { name: 'Select generation two for comparison' }))
    fireEvent.click(screen.getByRole('button', { name: 'Compare selected · 2' }))
    expect(await screen.findByRole('dialog', { name: 'Compare Forge results' })).toBeInTheDocument()
    expect(await screen.findByAltText('first portrait')).toHaveAttribute('src', 'asset:///original-asset-one')
    expect(await screen.findByAltText('second portrait')).toHaveAttribute('src', 'asset:///original-asset-two')
  })

  it('mounts and fetches thumbnails only for the virtualized result window', async () => {
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <Harness />
      </QueryClientProvider>,
    )
    const mounted = await screen.findAllByRole('button', { name: /generation .* for comparison/ })
    expect(mounted.length).toBeGreaterThan(0)
    expect(mounted.length).toBeLessThan(generations.length)
    await waitFor(() => {
      const thumbnailIds = new Set(
        h.invoke.mock.calls
          .filter(([command]) => command === 'asset_thumb')
          .map(([, args]) => (args as { assetId: string }).assetId),
      )
      expect(thumbnailIds.size).toBeGreaterThan(0)
      expect(thumbnailIds.size).toBeLessThan(generations.length)
    })
  })
})
