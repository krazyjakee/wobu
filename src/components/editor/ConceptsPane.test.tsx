import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type {
  Generation,
  GenerationPage,
  GenerationSummary,
  JobPreview,
  JobProgress,
  LinkEdge,
  NodeSummary,
  QueueSnapshot,
  WobuNode,
} from '../../lib/api'
import { kindDef, kindIndex, node as buildNode, summary } from '../../test/fixtures'
import { chooseOption } from '../Combobox.testing'
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

/**
 * `generation_list` answers with the indexed summary row, not the receipt.
 *
 * The tests are written against whole `Generation` fixtures because that is
 * what a reader needs to see, so the projection the backend does in SQL is
 * done here instead — one place, rather than a second parallel fixture that
 * can drift from the receipt it is supposed to summarise.
 */
function summarise(receipt: Generation): GenerationSummary {
  return {
    id: receipt.id,
    nodeId: receipt.nodeId,
    createdAt: receipt.createdAt,
    preset: receipt.preset,
    viewType: receipt.viewType,
    backend: receipt.backend,
    model: receipt.model,
    seed: receipt.seed,
    promptExcerpt: receipt.compiledPrompt,
    firstAssetId: receipt.outputAssetIds[0] ?? null,
    outputCount: receipt.outputAssetIds.length,
    seedSource: (receipt.params.seedSource as string | undefined) ?? null,
    usedLockedSeed: (receipt.params.usedLockedSeed as boolean | undefined) ?? null,
    sceneSubjectNames: [],
    thumbnailPath: null,
  }
}

function pageOf(receipts: Generation[]): GenerationPage {
  return { items: receipts.map(summarise), total: receipts.length, nextOffset: null }
}

const emptyQueue: QueueSnapshot = { jobs: [], queued: 0, running: 0, retrying: 0 }
let history: Generation[] = []
let worldNodes: NodeSummary[] = []
let worldLinks: LinkEdge[] = []
const kinds = kindIndex([
  kindDef('style_guide'),
  kindDef('world_bible'),
  kindDef('species'),
  kindDef('culture'),
  kindDef('character'),
  kindDef('creature'),
])

function open(queue: QueueSnapshot = emptyQueue, subject: WobuNode = node, readOnly = false) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={qc}>
      <ConceptsPane node={subject} queue={queue} kinds={kinds} readOnly={readOnly} />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  history = []
  worldNodes = [summary({ id: node.id, kind: node.kind, name: node.name })]
  worldLinks = []
  h.invoke.mockReset()
  h.listeners.clear()
  h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'generation_list') return Promise.resolve(pageOf(history))
    if (command === 'generation_get') {
      return Promise.resolve(history.find((item) => item.id === args?.generationId) ?? null)
    }
    if (command === 'node_list') return Promise.resolve(worldNodes)
    if (command === 'node_links') return Promise.resolve(worldLinks)
    if (command === 'asset_thumb') return Promise.resolve(`/thumb-${args?.assetId}`)
    if (command === 'asset_thumb_batch') {
      const ids = (args?.assetIds ?? []) as string[]
      return Promise.resolve(Object.fromEntries(ids.map((id) => [id, `/thumb-${id}`])))
    }
    if (command === 'asset_original') return Promise.resolve(`/original-${args?.assetId}`)
    if (command === 'asset_link') {
      return Promise.resolve({
        ...node,
        assetLinks: [
          ...node.assetLinks,
          {
            assetId: String(args?.assetId),
            role: args?.role,
            weight: 1,
            enabled: true,
          },
        ],
      })
    }
    if (command === 'asset_unlink') return Promise.resolve({ ...node, assetLinks: [] })
    if (command === 'generation_replay') return Promise.resolve('job-replay')
    if (command === 'job_cancel') return Promise.resolve(true)
    return Promise.resolve(null)
  })
  // The concept grid is virtualized, and jsdom reports every box as zero-sized,
  // so without this only the overscan rows mount and the assertions below are
  // about tiles that were never asked to render.
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, value: 900 })
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, value: 600 })
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    },
  )
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('generation history', () => {
  it('renders backend order, reveals prompt and seed, and loads one original only on click', async () => {
    history = [
      generation({
        id: 'new',
        createdAt: '2026-08-02T12:00:00Z',
        compiledPrompt: 'new prompt',
        seed: 9,
      }),
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
    const dialog = await screen.findByRole('dialog', { name: 'Generation details' })
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(dialog).toHaveAccessibleDescription(/Kael · portrait/)
    expect(within(dialog).getByRole('button', { name: 'Close generation details' })).toHaveFocus()
    expect(h.invoke).toHaveBeenCalledWith('asset_original', { assetId: 'asset-1' })
    expect(within(dialog).getByRole('img').getAttribute('src')).toBe('asset:///original-asset-1')
  })

  it('opens the exact snapshot and replays its immutable receipt', async () => {
    history = [
      generation({
        id: 'snapshot',
        params: { aspect: '3:4', width: 768, height: 1024 },
        influenceSnapshot: {
          layers: [
            {
              layer: 'subject',
              nodeId: node.id,
              nodeName: 'Kael then',
              weight: 0.7,
              muted: false,
              fragments: [
                {
                  section: 'appearance',
                  text: 'ash-grey travelling coat',
                  assetId: null,
                  weight: 0.7,
                  target: 'prompt',
                  dropped: false,
                },
              ],
            },
          ],
        },
      }),
    ]
    open()
    fireEvent.click(await screen.findByRole('button', { name: /Open generation from/ }))
    const dialog = await screen.findByRole('dialog', { name: 'Generation details' })
    expect(within(dialog).getByText('Exact recorded stack')).toBeInTheDocument()
    expect(within(dialog).getByText('ash-grey travelling coat')).toBeInTheDocument()
    fireEvent.click(within(dialog).getByRole('button', { name: 'Replay snapshot' }))
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('generation_replay', { generationId: 'snapshot' }),
    )
  })

  it('states whether each persisted result used the lock, its family, or an explicit re-roll', async () => {
    history = [
      generation({ id: 'locked', params: { seedSource: 'locked', usedLockedSeed: true } }),
      generation({ id: 'family', params: { seedSource: 'locked_derived', usedLockedSeed: false } }),
      generation({ id: 'rerolled', params: { seedSource: 'rerolled' } }),
      generation({ id: 'grid', params: { seedSource: 'grid' } }),
      generation({ id: 'replay', params: { seedSource: 'replay', usedLockedSeed: true } }),
    ]
    open()

    expect(await screen.findByText('used locked seed')).toBeTruthy()
    expect(screen.getByText('used locked-seed family')).toBeTruthy()
    expect(screen.getByText('used explicit re-roll')).toBeTruthy()
    expect(screen.getByText('variant seed cell')).toBeTruthy()
    expect(screen.getByText('replayed snapshot')).toBeTruthy()
  })

  it('pins with an explicit role through AssetLink without changing generation history', async () => {
    history = [generation({ id: 'candidate' })]
    open()

    const role = await screen.findByRole('combobox', {
      name: 'Reference role for generation candidate',
    })
    // The field states the current role in words now, not its wire name.
    expect(role).toHaveValue('Full reference')
    chooseOption('Reference role for generation candidate', 'Palette')
    fireEvent.click(screen.getByRole('button', { name: 'Pin generation candidate as Palette' }))

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('asset_link', {
        nodeId: 'kael',
        assetId: 'asset-1',
        role: 'palette',
        weight: undefined,
      }),
    )
    expect(screen.getByRole('button', { name: /Open generation from/ })).toBeInTheDocument()
    expect(h.invoke).not.toHaveBeenCalledWith('generation_delete', expect.anything())
  })

  it('unpins the chosen role without deleting the immutable generation', async () => {
    history = [generation({ id: 'pinned' })]
    open(
      emptyQueue,
      buildNode({
        ...node,
        assetLinks: [{ assetId: 'asset-1', role: 'full_ref', weight: 1, enabled: true }],
      }),
    )

    expect(await screen.findByText('Pinned · Full reference')).toBeInTheDocument()
    fireEvent.click(
      screen.getByRole('button', { name: 'Unpin generation pinned as Full reference' }),
    )
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('asset_unlink', {
        nodeId: 'kael',
        assetId: 'asset-1',
        role: 'full_ref',
      }),
    )
    expect(screen.getByRole('button', { name: /Open generation from/ })).toBeInTheDocument()
  })

  it('states how an upstream pin reaches downstream generations', async () => {
    const species = buildNode({ id: 'vashk', name: 'Vashk', kind: 'species' })
    history = [generation({ id: 'species-concept', nodeId: species.id })]
    worldNodes = [
      summary({ id: 'style', kind: 'style_guide' }),
      summary({ id: 'world', kind: 'world_bible' }),
      summary({ id: 'vashk', name: 'Vashk', kind: 'species' }),
      summary({ id: 'kael', name: 'Kael', kind: 'character' }),
    ]
    worldLinks = [{ fromId: 'kael', toId: 'vashk', role: 'species_of', weight: 1, enabled: true }]
    open(emptyQueue, species)

    expect(
      await screen.findByText(
        'Future generations for this species and 1 character downstream can inherit this appearance-locking full reference.',
      ),
    ).toBeInTheDocument()
  })

  it('leaves pinning visible but unavailable in a read-only project', async () => {
    history = [generation({ id: 'readonly' })]
    open(emptyQueue, node, true)

    expect(await screen.findByLabelText('Reference role for generation readonly')).toBeDisabled()
    expect(
      screen.getByRole('button', { name: 'Pin generation readonly as Full reference' }),
    ).toBeDisabled()
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
        payload: {
          id: 'job-1',
          image: 'data:image/webp;base64,preview',
          step: 12,
        } satisfies JobPreview,
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
