import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Asset, AssetUsage, NodeSummary, WobuNode } from '../lib/api'
import { kindDef, kindIndex, node as buildNode, summary } from '../test/fixtures'
import { AssetsMode } from './AssetsMode'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))

const nodes: NodeSummary[] = [
  summary({ id: 'kael', name: 'Kael', kind: 'character' }),
  summary({ id: 'vashk', name: 'Vashk', kind: 'species' }),
]
const kinds = kindIndex([
  kindDef('character', { label: 'Character', plural: 'Characters' }),
  kindDef('species', { label: 'Species', plural: 'Species' }),
])
let assets: Asset[] = []
let usages: AssetUsage[] = []

function asset(id: string, kind: Asset['kind'] = 'reference'): Asset {
  return {
    id,
    hash: id,
    kind,
    relPath: `assets/originals/${id}.png`,
    thumbPath: `assets/thumbs/${id}.webp`,
    mime: 'image/png',
    width: 1200,
    height: 800,
    bytes: 2048,
    createdAt: '2026-08-01T10:00:00Z',
  }
}

function usage(
  assetId: string,
  nodeId: string,
  roles: AssetUsage['roles'],
  over: Partial<AssetUsage> = {},
): AssetUsage {
  const linked = nodes.find((node) => node.id === nodeId)!
  return {
    assetId,
    nodeId,
    nodeName: linked.name,
    nodeKind: linked.kind,
    nodeTags: [],
    roles,
    cover: false,
    ...over,
  }
}

function role(value: AssetUsage['roles'][number]['role'], weight = 1) {
  return { role: value, weight, enabled: true }
}

function open(readOnly = false) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  return render(
    <QueryClientProvider client={qc}>
      <AssetsMode nodes={nodes} kinds={kinds} readOnly={readOnly} onJump={vi.fn()} />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  assets = []
  usages = []
  h.invoke.mockReset()
  h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'asset_list') return Promise.resolve(assets)
    if (command === 'asset_usage_list') return Promise.resolve(usages)
    if (command === 'asset_thumb') return Promise.resolve(`/thumb/${String(args?.assetId)}.webp`)
    if (command === 'asset_link') {
      return Promise.resolve(
        buildNode({
          id: String(args?.nodeId),
          assetLinks: [
            {
              assetId: String(args?.assetId),
              role: args?.role as 'full_ref',
              weight: 1,
              enabled: true,
            },
          ],
        }) satisfies WobuNode,
      )
    }
    if (command === 'asset_delete') return Promise.resolve(null)
    if (command === 'node_list') return Promise.resolve(nodes)
    return Promise.resolve(null)
  })
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

describe('AssetsMode', () => {
  it('virtualizes the all-assets grid and requests thumbnails only for mounted tiles', async () => {
    assets = Array.from({ length: 40 }, (_, index) => asset(`asset-${index}`))
    const view = open()

    await waitFor(() =>
      expect(view.container.querySelectorAll('.asset-library-tile').length).toBeGreaterThan(0),
    )
    const mounted = view.container.querySelectorAll('.asset-library-tile').length
    expect(mounted).toBeLessThan(assets.length)
    expect(view.container.querySelector('.asset-grid')).toHaveStyle({ height: '2160px' })
    const firstTile = screen.getByRole('button', { name: 'Select reference asset asset-0' })
    expect(firstTile).toHaveAttribute('aria-pressed', 'false')
    expect(firstTile.querySelector('img')).toHaveAttribute('loading', 'lazy')
    expect(h.invoke).not.toHaveBeenCalledWith('asset_original', expect.anything())
  })

  it('preserves the asset preview error label', async () => {
    assets = [asset('broken')]
    h.invoke.mockImplementation((command: string) => {
      if (command === 'asset_list') return Promise.resolve(assets)
      if (command === 'asset_usage_list') return Promise.resolve([])
      if (command === 'asset_thumb') return Promise.reject(new Error('thumbnail failed'))
      return Promise.resolve(null)
    })
    open()

    expect(await screen.findByText('Preview failed')).toBeInTheDocument()
  })

  it('filters by kind, role, node, linked-node tag, and orphan state', async () => {
    assets = [asset('reference'), asset('generated', 'generated'), asset('upload', 'upload')]
    usages = [
      usage('reference', 'kael', [role('full_ref')], { nodeTags: ['hero'] }),
      usage('upload', 'vashk', [role('palette')], { nodeTags: ['ancestry'] }),
    ]
    open()
    await screen.findByRole('button', { name: 'Select reference asset reference' })

    fireEvent.change(screen.getByLabelText('Filter assets by kind'), {
      target: { value: 'generated' },
    })
    expect(screen.getByRole('button', { name: 'Select generated asset generated' })).toBeVisible()
    expect(screen.queryByRole('button', { name: 'Select reference asset reference' })).toBeNull()

    fireEvent.change(screen.getByLabelText('Filter assets by kind'), { target: { value: 'all' } })
    fireEvent.change(screen.getByLabelText('Filter assets by role'), {
      target: { value: 'palette' },
    })
    expect(screen.getByRole('button', { name: 'Select upload asset upload' })).toBeVisible()

    fireEvent.change(screen.getByLabelText('Filter assets by role'), { target: { value: 'all' } })
    fireEvent.change(screen.getByLabelText('Filter assets by node'), { target: { value: 'kael' } })
    expect(screen.getByRole('button', { name: 'Select reference asset reference' })).toBeVisible()

    fireEvent.change(screen.getByLabelText('Filter assets by node'), { target: { value: 'all' } })
    fireEvent.change(screen.getByLabelText('Filter assets by linked node tag'), {
      target: { value: 'ancestry' },
    })
    expect(screen.getByRole('button', { name: 'Select upload asset upload' })).toBeVisible()

    fireEvent.change(screen.getByLabelText('Filter assets by linked node tag'), {
      target: { value: 'all' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Orphans 1' }))
    expect(screen.getByRole('button', { name: 'Select generated asset generated' })).toBeVisible()
    expect(screen.queryByRole('button', { name: 'Select upload asset upload' })).toBeNull()
  })

  it('shows every linked node, role, weight, cover, and linked-node tag', async () => {
    assets = [asset('shared')]
    usages = [
      usage('shared', 'kael', [role('full_ref'), role('palette', 0.6)], {
        cover: true,
        nodeTags: ['hero'],
      }),
      usage('shared', 'vashk', [role('silhouette', 0.8)], { nodeTags: ['ancestry'] }),
    ]
    open()
    const assetButton = await screen.findByRole('button', {
      name: 'Select reference asset shared',
    })
    expect(assetButton).toHaveAttribute('aria-pressed', 'false')
    fireEvent.click(assetButton)
    expect(assetButton).toHaveAttribute('aria-pressed', 'true')
    const details = screen.getByRole('complementary', { name: 'Details for asset shared' })

    expect(within(details).getByText('Used by 2 nodes')).toBeInTheDocument()
    expect(within(details).getByText('Full reference · 1.00')).toBeInTheDocument()
    expect(within(details).getByText('Palette · 0.60')).toBeInTheDocument()
    expect(within(details).getByText('Silhouette · 0.80')).toBeInTheDocument()
    expect(within(details).getByText('Cover')).toBeInTheDocument()
    expect(within(details).getByText('#hero')).toBeInTheDocument()
    expect(within(details).getByText('#ancestry')).toBeInTheDocument()
    expect(within(details).queryByRole('button', { name: 'Delete…' })).toBeNull()
  })

  it('attaches an orphan to the chosen node and role through the real link command', async () => {
    assets = [asset('loose', 'upload')]
    open()
    fireEvent.click(await screen.findByRole('button', { name: 'Select upload asset loose' }))
    const details = screen.getByRole('complementary', { name: 'Details for asset loose' })
    fireEvent.change(within(details).getByLabelText('Role'), { target: { value: 'pose' } })
    fireEvent.click(within(details).getByRole('button', { name: 'Attach' }))

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('asset_link', {
        nodeId: 'kael',
        assetId: 'loose',
        role: 'pose',
        weight: undefined,
      }),
    )
  })

  it('requires confirmation before deleting an orphan and explains generated receipts', async () => {
    assets = [asset('loose-generation', 'generated')]
    open()
    fireEvent.click(
      await screen.findByRole('button', { name: 'Select generated asset loose-generation' }),
    )
    fireEvent.click(screen.getByRole('button', { name: 'Delete…' }))

    const confirm = screen.getByRole('alertdialog', { name: 'Delete orphaned asset?' })
    expect(
      within(confirm).getByText(/immutable generation receipt will remain/),
    ).toBeInTheDocument()
    expect(h.invoke).not.toHaveBeenCalledWith('asset_delete', expect.anything())
    fireEvent.click(within(confirm).getByRole('button', { name: 'Delete permanently' }))
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('asset_delete', { assetId: 'loose-generation' }),
    )
  })

  it('withholds attach and delete mutations in a read-only project', async () => {
    assets = [asset('readonly')]
    open(true)
    fireEvent.click(await screen.findByRole('button', { name: 'Select reference asset readonly' }))
    const details = screen.getByRole('complementary', { name: 'Details for asset readonly' })

    expect(within(details).getByRole('button', { name: 'Attach' })).toBeDisabled()
    expect(within(details).getByRole('button', { name: 'Delete…' })).toBeDisabled()
  })
})
