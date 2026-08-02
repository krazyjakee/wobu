import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Generation, LoraStatus, ProjectSummary, QueueSnapshot } from '../lib/api'
import { useUI } from '../store/ui'
import { kindDef, kindIndex, summary } from '../test/fixtures'
import { ForgeMode } from './ForgeMode'
import { Inspector } from './Inspector'

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
let imageConfigured: boolean
let paidGenerationBlocked: boolean
let spendStatusState: 'ready' | 'pending' | 'failed'
let aspectRatios: string[]
let aspectFallback: string
let aspectDimensions: [number, number]
let flexibleAspect: boolean
let presetAspect: string
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

function renderForge(openProject: ProjectSummary = project) {
  return render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <Harness openProject={openProject} />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  h.invoke.mockReset()
  useUI.setState({ selectedId: 'kael' })
  imageConfigured = true
  paidGenerationBlocked = false
  spendStatusState = 'ready'
  aspectRatios = ['1:1', '3:2', '2:3', '3:4', '4:3', '4:5', '5:4', '9:16', '16:9', '21:9']
  aspectFallback = '1:1'
  aspectDimensions = [2048, 2048]
  flexibleAspect = true
  presetAspect = '3:4'
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
    if (command === 'preset_list') return Promise.resolve([{ ...preset, aspect: presetAspect }])
    if (command === 'status_bar_backend')
      return Promise.resolve({
        image: imageConfigured
          ? { provider: 'comfyui', label: 'ComfyUI', model: 'flux-dev', contextTokens: null }
          : null,
        text: { provider: 'anthropic', label: 'Anthropic', model: 'claude', contextTokens: 1 },
        health: { state: 'connected', externalQueue: 0 },
      })
    if (command === 'influence_resolve')
      return Promise.resolve({
        subjectId: args?.subjectId,
        preset: { ...preset, aspect: presetAspect },
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
        preset: { ...preset, aspect: presetAspect },
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
    if (command === 'image_reference_report') {
      if (!imageConfigured) return Promise.resolve(null)
      return Promise.resolve({
        buckets: [],
        layers: [],
        cost: paidGenerationBlocked
          ? {
              perImageUsdMicros: 25_000,
              batchUsdMicros: 25_000,
              images: 1,
              variesByCell: false,
              conservativeFallback: false,
              checkedAt: '2026-08-01T12:00:00Z',
            }
          : null,
        lockedSeed: null,
      })
    }
    if (command === 'image_generation_capabilities') {
      if (!imageConfigured) return Promise.reject(new Error('No image provider'))
      const vocabulary = ['1:1', '3:2', '2:3', '3:4', '4:3', '4:5', '5:4', '9:16', '16:9', '21:9']
      return Promise.resolve({
        provider: 'comfyui',
        model: typeof args?.model === 'string' ? args.model : 'flux-dev',
        aspectRatios,
        flexibleAspect,
        previews: vocabulary.map((requestedAspect) => {
          const actualAspect = aspectRatios.includes(requestedAspect)
            ? requestedAspect
            : aspectFallback
          return {
            requestedAspect,
            actualAspect,
            width: aspectDimensions[0],
            height: aspectDimensions[1],
            substituted: requestedAspect !== actualAspect,
          }
        }),
      })
    }
    if (command === 'spend_status') {
      if (spendStatusState === 'pending') return new Promise(() => {})
      if (spendStatusState === 'failed') return Promise.reject(new Error('ledger unavailable'))
      return Promise.resolve({
        ceilingUsdMicros: null,
        spentUsdMicros: 0,
        reservedUsdMicros: 0,
        remainingUsdMicros: null,
        pendingReservations: 0,
        oldestReservationAt: null,
        ledgerLocked: false,
      })
    }
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

  it.each([
    ['Command', { metaKey: true }],
    ['Control', { ctrlKey: true }],
  ])('queues Generate with %s-Enter from the Forge Inspector', async (_platform, modifier) => {
    renderForge()
    const generate = await screen.findByRole('button', { name: 'Generate' })
    await waitFor(() => expect(generate).toBeEnabled())

    fireEvent.keyDown(window, { key: 'Enter', ...modifier })

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith(
        'generate_start',
        expect.objectContaining({ subjectId: 'kael', preset: 'portrait' }),
      ),
    )
  })

  it('queues Generate with Command-Enter from the Library Inspector surface', async () => {
    render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <Inspector project={project} selected={nodes[0]!} kinds={kinds} onJump={() => {}} />
      </QueryClientProvider>,
    )
    const generate = await screen.findByRole('button', { name: 'Generate' })
    await waitFor(() => expect(generate).toBeEnabled())

    fireEvent.keyDown(window, { key: 'Enter', metaKey: true })

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith(
        'generate_start',
        expect.objectContaining({ subjectId: 'kael', preset: 'portrait' }),
      ),
    )
  })

  it('does not Generate while the shortcut originates in an editable control', async () => {
    renderForge()
    const generate = await screen.findByRole('button', { name: 'Generate' })
    await waitFor(() => expect(generate).toBeEnabled())

    fireEvent.keyDown(screen.getByLabelText('Extra shot prompt'), {
      key: 'Enter',
      metaKey: true,
    })

    expect(h.invoke).not.toHaveBeenCalledWith('generate_start', expect.anything())
  })

  it('does not Generate from a read-only project', async () => {
    renderForge({ ...project, readOnly: true })
    const generate = await screen.findByRole('button', { name: 'Generate' })
    await waitFor(() => expect(generate).toBeDisabled())

    fireEvent.keyDown(window, { key: 'Enter', ctrlKey: true })

    expect(h.invoke).not.toHaveBeenCalledWith('generate_start', expect.anything())
  })

  it('does not Generate without a configured image provider or reference report', async () => {
    imageConfigured = false
    renderForge()
    const generate = await screen.findByRole('button', { name: 'Generate' })
    await waitFor(() => expect(generate).toBeDisabled())

    fireEvent.keyDown(window, { key: 'Enter', metaKey: true })

    expect(h.invoke).not.toHaveBeenCalledWith('generate_start', expect.anything())
  })

  it('does not Generate until paid-provider cost consent is available', async () => {
    paidGenerationBlocked = true
    renderForge()
    const generate = await screen.findByRole('button', { name: /^Generate · est/ })
    await waitFor(() => expect(generate).toBeDisabled())

    fireEvent.keyDown(window, { key: 'Enter', ctrlKey: true })

    expect(h.invoke).not.toHaveBeenCalledWith('generate_start', expect.anything())
  })

  it.each(['pending', 'failed'] as const)(
    'keeps paid Generate disabled while spend status is %s',
    async (state) => {
      paidGenerationBlocked = true
      spendStatusState = state
      renderForge()
      const generate = await screen.findByRole('button', { name: /^Generate · est/ })
      await waitFor(() => expect(generate).toBeDisabled())

      fireEvent.keyDown(window, { key: 'Enter', ctrlKey: true })

      expect(h.invoke).not.toHaveBeenCalledWith('generate_start', expect.anything())
    },
  )

  it('does not Generate an invalid variant grid', async () => {
    renderForge()
    const generate = await screen.findByRole('button', { name: 'Generate' })
    await waitFor(() => expect(generate).toBeEnabled())
    fireEvent.change(screen.getByLabelText('Variant grid'), { target: { value: 'seed' } })
    fireEvent.change(screen.getByLabelText('Cell values · comma separated'), {
      target: { value: 'only-one' },
    })
    await waitFor(() => expect(generate).toBeDisabled())

    fireEvent.keyDown(window, { key: 'Enter', metaKey: true })

    expect(h.invoke).not.toHaveBeenCalledWith('generate_start', expect.anything())
  })

  it('repairs an unsupported saved aspect when provider capabilities switch before queueing', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={client}>
        <Harness />
      </QueryClientProvider>,
    )
    const aspect = await screen.findByRole('combobox', { name: /^Aspect$/ })
    await waitFor(() => expect(aspect).toHaveValue('3:4'))

    flexibleAspect = false
    aspectRatios = ['1:1', '16:9']
    aspectFallback = '1:1'
    aspectDimensions = [1024, 1024]
    await client.invalidateQueries({ queryKey: ['image_generation_capabilities'] })

    expect(
      await screen.findByText('Unsupported saved aspect 3:4 was replaced with 1:1.'),
    ).toBeInTheDocument()
    await waitFor(() => expect(aspect).toHaveValue('1:1'))
    expect(
      within(aspect.closest('.shot-controls') as HTMLElement).getByText(
        /Actual output · 1:1 · 1024×1024px/,
      ),
    ).toBeInTheDocument()
    const generate = screen.getByRole('button', { name: 'Generate' })
    await waitFor(() => expect(generate).toBeEnabled())
    fireEvent.click(generate)
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith(
        'generate_start',
        expect.objectContaining({ aspect: '1:1' }),
      ),
    )
  })

  it('invalidates saved aspect-grid cells when the provider no longer offers them', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={client}>
        <Harness />
      </QueryClientProvider>,
    )
    const generate = await screen.findByRole('button', { name: 'Generate' })
    await waitFor(() => expect(generate).toBeEnabled())
    fireEvent.change(screen.getByLabelText('Variant grid'), { target: { value: 'aspect' } })
    expect(screen.getByLabelText('Cell values · comma separated')).toHaveValue('1:1, 3:2, 2:3')

    flexibleAspect = false
    aspectRatios = ['1:1']
    aspectFallback = '1:1'
    await client.invalidateQueries({ queryKey: ['image_generation_capabilities'] })

    expect(
      await screen.findByText('3:2 is not supported by the selected image backend.'),
    ).toBeInTheDocument()
    expect(generate).toBeDisabled()
    fireEvent.click(generate)
    expect(h.invoke).not.toHaveBeenCalledWith('generate_start', expect.anything())
  })

  it('labels the bounded validation strategy used for flexible image backends', async () => {
    renderForge()
    const aspect = await screen.findByRole('combobox', { name: /^Aspect$/ })
    await waitFor(() =>
      expect(aspect.closest('.shot-controls')).toHaveTextContent(
        "Flexible backend · using Wobu's curated, validated aspect choices.",
      ),
    )
    expect(aspect.querySelectorAll('option')).toHaveLength(10)
  })

  it('debounces custom model capability probes and keeps queueing disabled while they settle', async () => {
    renderForge()
    const generate = await screen.findByRole('button', { name: 'Generate' })
    await waitFor(() => expect(generate).toBeEnabled())
    h.invoke.mockClear()

    const model = screen.getByLabelText('Model')
    fireEvent.change(model, { target: { value: 'custom-a' } })
    fireEvent.change(model, { target: { value: 'custom-ab' } })
    fireEvent.change(model, { target: { value: 'custom-abc' } })
    expect(generate).toBeDisabled()
    expect(h.invoke).not.toHaveBeenCalledWith('image_generation_capabilities', expect.anything())

    await waitFor(
      () =>
        expect(h.invoke).toHaveBeenCalledWith('image_generation_capabilities', {
          model: 'custom-abc',
        }),
      { timeout: 1_000 },
    )
    expect(
      h.invoke.mock.calls.filter(([command]) => command === 'image_generation_capabilities'),
    ).toHaveLength(1)
    await waitFor(() => expect(generate).toBeEnabled())
  })

  it('repairs a malformed saved aspect before the queue can receive it', async () => {
    presetAspect = 'wide please'
    renderForge()
    expect(h.invoke).not.toHaveBeenCalledWith('generate_start', expect.anything())
    expect(
      await screen.findByText('Malformed saved aspect wide please was replaced with 1:1.'),
    ).toBeInTheDocument()
    const generate = screen.getByRole('button', { name: 'Generate' })
    await waitFor(() => expect(generate).toBeEnabled())
    fireEvent.click(generate)
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith(
        'generate_start',
        expect.objectContaining({ aspect: '1:1' }),
      ),
    )
    expect(h.invoke).not.toHaveBeenCalledWith(
      'generate_start',
      expect.objectContaining({ aspect: 'wide please' }),
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
    const sceneAspect = screen.getByLabelText('Scene aspect')
    await waitFor(() => expect(sceneAspect).toBeEnabled())
    fireEvent.change(sceneAspect, { target: { value: '3:2' } })
    const generateScene = screen.getByRole('button', { name: 'Generate scene' })
    await waitFor(() => expect(generateScene).toBeEnabled())
    fireEvent.click(generateScene)

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('scene_generate_start', {
        subjectIds: ['kael', 'mira'],
        prompt: 'Crossing the flooded market',
        aspect: '3:2',
      }),
    )
    expect(await screen.findByText('Scene queued with 2 entities.')).toBeInTheDocument()
  })

  it('uses provider negotiation for composition and shows substituted dimensions before queueing', async () => {
    flexibleAspect = false
    aspectRatios = ['1:1']
    aspectFallback = '1:1'
    aspectDimensions = [768, 768]
    renderForge()

    fireEvent.click(screen.getByText('Compose a multi-entity scene'))
    fireEvent.click(screen.getByRole('checkbox', { name: 'Mira' }))
    expect(
      await screen.findByText('Unsupported saved aspect 16:9 was replaced with 1:1.'),
    ).toBeInTheDocument()
    expect(screen.getByLabelText('Scene aspect').querySelectorAll('option')).toHaveLength(1)
    expect(screen.getByText('Compose a multi-entity scene').closest('details')).toHaveTextContent(
      'Actual output · 1:1 · 768×768px',
    )
    const generateScene = screen.getByRole('button', { name: 'Generate scene' })
    await waitFor(() => expect(generateScene).toBeEnabled())
    fireEvent.click(generateScene)

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith(
        'scene_generate_start',
        expect.objectContaining({ aspect: '1:1' }),
      ),
    )
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
    const first = await screen.findByRole('button', {
      name: 'Select generation one for comparison',
    })
    expect(first).toHaveAttribute('aria-pressed', 'false')
    await waitFor(() => expect(first.querySelector('img')).toHaveAttribute('loading', 'lazy'))
    expect(h.invoke).not.toHaveBeenCalledWith('asset_original', expect.anything())
    fireEvent.click(first)
    fireEvent.click(screen.getByRole('button', { name: 'Select generation two for comparison' }))
    expect(h.invoke).not.toHaveBeenCalledWith('asset_original', expect.anything())
    fireEvent.click(screen.getByRole('button', { name: 'Compare selected · 2' }))
    const comparison = await screen.findByRole('dialog', { name: 'Compare Forge results' })
    expect(comparison).toHaveAttribute('aria-modal', 'true')
    expect(comparison).toHaveAccessibleDescription(/Full-resolution comparison/)
    expect(within(comparison).getByRole('button', { name: 'Close Forge comparison' })).toHaveFocus()
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

  it('keeps the Forge preview loading label while a mounted thumbnail is pending', async () => {
    const defaultInvoke = h.invoke.getMockImplementation()!
    h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === 'asset_thumb') return new Promise(() => {})
      return defaultInvoke(command, args)
    })
    renderForge()

    const first = await screen.findByRole('button', {
      name: 'Select generation one for comparison',
    })
    expect(within(first).getByText('Loading preview…')).toBeInTheDocument()
    expect(h.invoke).not.toHaveBeenCalledWith('asset_original', expect.anything())
  })
})
