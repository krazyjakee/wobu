import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Generation, LoraStatus, ProjectSummary, QueueSnapshot } from '../lib/api'
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
  id: 'project',
  name: 'Ashfall',
  path: '/project',
  onNetworkShare: false,
  readOnly: false,
  lastOpenedAt: null,
}
const queue: QueueSnapshot = { jobs: [], queued: 0, running: 0, retrying: 0 }
let lora: LoraStatus
const preset = {
  id: 'portrait',
  label: 'Portrait',
  kinds: ['character'],
  defaultFor: ['character'],
  priorities: [],
  framing: 'portrait framing',
  aspect: '3:4',
  images: 1,
  views: [],
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
    id,
    nodeId: 'kael',
    createdAt: '2026-08-01T12:00:00Z',
    preset: 'portrait',
    viewType: null,
    userPrompt: '',
    compiledPrompt: prompt,
    negativePrompt: '',
    backend: 'comfyui',
    model: 'flux-dev',
    seed,
    params: {},
    outputAssetIds: [assetId],
    influenceSnapshot: { layers: [] },
  }
}

function Harness({
  openProject = project,
  jobQueue = queue,
}: {
  openProject?: ProjectSummary
  jobQueue?: QueueSnapshot
}) {
  const selectedId = useUI((state) => state.selectedId)
  const selected = nodes.find((node) => node.id === selectedId) ?? null
  return (
    <ForgeMode
      project={openProject}
      nodes={nodes}
      selected={selected}
      kinds={kinds}
      queue={jobQueue}
      onJump={() => {}}
    />
  )
}

beforeEach(() => {
  h.invoke.mockReset()
  useUI.setState({ selectedId: 'kael' })
  lora = {
    subjectId: 'kael',
    pinnedCount: 15,
    invalidPinnedCount: 2,
    requiredCount: 15,
    eligible: true,
    trainerState: 'available',
    trainerDetail: 'wobu-lora-trainer is ready for the selected model.',
    selectedModel: 'flux-dev',
    pin: null,
    applicationState: 'none',
    applicationDetail: 'No trained LoRA is attached to this entity.',
  }
  h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'preset_list') return Promise.resolve([preset])
    if (command === 'status_bar_backend')
      return Promise.resolve({
        image: { provider: 'comfyui', label: 'ComfyUI', model: 'flux-dev', contextTokens: null },
        text: { provider: 'anthropic', label: 'Anthropic', model: 'claude', contextTokens: 1 },
        health: { state: 'connected', externalQueue: 0 },
      })
    if (command === 'influence_resolve')
      return Promise.resolve({
        subjectId: args?.subjectId,
        preset,
        layers: [
          {
            layer: 'subject',
            nodeId: args?.subjectId,
            name: 'Kael',
            kind: 'character',
            reached: 'subject',
            distance: 0,
            weight: 1,
            slider: 1,
            fragments: [
              {
                layer: 'subject',
                nodeId: args?.subjectId,
                sourceName: 'Kael',
                section: 'appearance',
                text: 'ash-grey coat',
                assetId: null,
                weight: 1,
                target: 'prompt',
                sendable: true,
              },
            ],
          },
        ],
      })
    if (command === 'prompt_compile')
      return Promise.resolve({
        subjectId: args?.subjectId,
        preset,
        prompt: 'ash-grey coat',
        negative: '',
        spans: [
          {
            layer: 'subject',
            nodeId: args?.subjectId,
            sourceName: 'Kael',
            section: 'appearance',
            text: 'ash-grey coat',
            assetId: null,
            weight: 1,
            target: 'prompt',
            sendable: true,
          },
        ],
        dropped: [],
        overflow: null,
      })
    if (command === 'image_reference_report')
      return Promise.resolve({
        buckets: [],
        layers: [],
        cost: null,
        spend: {
          ceilingUsdMicros: null,
          spentUsdMicros: 0,
          reservedUsdMicros: 0,
          remainingUsdMicros: null,
          pendingReservations: 0,
          oldestReservationAt: null,
          ledgerLocked: false,
        },
        lockedSeed: null,
      })
    if (command === 'generation_list') {
      return Promise.resolve(args?.nodeId === 'kael' ? generations : [])
    }
    if (command === 'asset_thumb') return Promise.resolve(`/thumb-${args?.assetId}`)
    if (command === 'asset_original') return Promise.resolve(`/original-${args?.assetId}`)
    if (command === 'generate_start') return Promise.resolve('job-forge')
    if (command === 'scene_generate_start') return Promise.resolve('job-scene')
    if (command === 'lora_status') return Promise.resolve({ ...lora, subjectId: args?.subjectId })
    if (command === 'lora_train_start') return Promise.resolve('job-lora')
    return Promise.resolve(null)
  })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('Forge mode', () => {
  it('keeps one subject selected while moving between Forge histories', async () => {
    render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <Harness />
      </QueryClientProvider>,
    )
    fireEvent.change(screen.getByLabelText('Forge subject'), { target: { value: 'mira' } })
    expect(useUI.getState().selectedId).toBe('mira')
    expect(
      await screen.findByRole('region', { name: 'Forge results for Mira' }),
    ).toBeInTheDocument()
  })

  it('reuses the attributed Inspector and queues its one-axis batch', async () => {
    render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <Harness />
      </QueryClientProvider>,
    )
    expect(
      await screen.findByRole('region', { name: 'Forge generation controls' }),
    ).toBeInTheDocument()
    await waitFor(() =>
      expect(screen.getByRole('region', { name: 'Compiled prompt' })).toHaveTextContent(
        'ash-grey coat',
      ),
    )
    fireEvent.change(screen.getByLabelText('Variant grid'), { target: { value: 'seed' } })
    const generate = await screen.findByRole('button', { name: 'Generate' })
    await waitFor(() => expect(generate).toBeEnabled())
    fireEvent.click(generate)
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith(
        'generate_start',
        expect.objectContaining({
          subjectId: 'kael',
          grid: expect.objectContaining({ axis: 'seed' }),
        }),
      ),
    )
  })

  it('queues an ordered multi-entity scene from the selected Forge subject', async () => {
    render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <Harness />
      </QueryClientProvider>,
    )

    fireEvent.click(screen.getByText('Compose a multi-entity scene'))
    fireEvent.click(screen.getByRole('checkbox', { name: 'Mira' }))
    fireEvent.change(screen.getByPlaceholderText('Crossing the flooded market at blue hour…'), {
      target: { value: 'Crossing the flooded market' },
    })
    fireEvent.change(screen.getByLabelText('Scene aspect'), { target: { value: '3:2' } })
    fireEvent.click(screen.getByRole('button', { name: 'Generate scene' }))

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('scene_generate_start', {
        subjectIds: ['kael', 'mira'],
        prompt: 'Crossing the flooded market',
        aspect: '3:2',
      }),
    )
    expect(await screen.findByRole('status')).toHaveTextContent('Scene queued with 2 entities.')
  })

  it('shows LoRA readiness and queues eligible local training', async () => {
    render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <Harness />
      </QueryClientProvider>,
    )

    const card = await screen.findByRole('region', { name: 'LoRA for Kael' })
    expect(card).toHaveTextContent('15 / 15 valid full references · 2 invalid')
    expect(card).toHaveTextContent('wobu-lora-trainer is ready for the selected model.')
    expect(card).toHaveTextContent('flux-dev')
    expect(card).toHaveTextContent('No trained LoRA is attached to this entity.')
    fireEvent.click(screen.getByRole('button', { name: 'Train LoRA for Kael' }))
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('lora_train_start', { subjectId: 'kael' }),
    )
  })

  it('offers re-training but disables it while training is active or the project is read-only', async () => {
    lora.pin = {
      hash: 'abc',
      relPath: 'assets/lora/abc.safetensors',
      bytes: 42,
      trainer: 'wobu-lora-trainer',
      protocol: 1,
      baseModel: 'flux-dev',
      modelFamily: 'flux',
      providerName: 'kael.safetensors',
      triggerToken: 'wobu_kael',
      inputAssetHashes: ['input'],
      createdAt: '2026-08-01T12:00:00Z',
      strength: 0.8,
    }
    const activeQueue: QueueSnapshot = {
      jobs: [
        {
          id: 'job-lora',
          kind: 'train_lora',
          label: 'Train LoRA for Kael',
          subjectId: 'kael',
          attempt: 1,
          elapsedMs: 100,
          state: 'running',
        },
      ],
      queued: 0,
      running: 1,
      retrying: 0,
    }
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const view = render(
      <QueryClientProvider client={client}>
        <Harness jobQueue={activeQueue} />
      </QueryClientProvider>,
    )

    const retrain = await screen.findByRole('button', { name: 'Re-train LoRA for Kael' })
    expect(retrain).toBeDisabled()
    expect(retrain).toHaveAttribute('title', 'Training is already active for this entity.')

    view.rerender(
      <QueryClientProvider client={client}>
        <Harness openProject={{ ...project, readOnly: true }} />
      </QueryClientProvider>,
    )
    expect(screen.getByRole('button', { name: 'Re-train LoRA for Kael' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Re-train LoRA for Kael' })).toHaveAttribute(
      'title',
      'This project is read-only.',
    )
  })

  it('virtualizes receipts and compares selected originals side by side', async () => {
    render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <Harness />
      </QueryClientProvider>,
    )
    fireEvent.click(
      await screen.findByRole('button', { name: 'Select generation one for comparison' }),
    )
    fireEvent.click(screen.getByRole('button', { name: 'Select generation two for comparison' }))
    fireEvent.click(screen.getByRole('button', { name: 'Compare selected · 2' }))
    expect(await screen.findByRole('dialog', { name: 'Compare Forge results' })).toBeInTheDocument()
    expect(await screen.findByAltText('first portrait')).toHaveAttribute(
      'src',
      'asset:///original-asset-one',
    )
    expect(await screen.findByAltText('second portrait')).toHaveAttribute(
      'src',
      'asset:///original-asset-two',
    )
  })

  it('mounts and fetches thumbnails only for the virtualized result window', async () => {
    render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
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
