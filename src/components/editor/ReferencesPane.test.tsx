import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Asset, AssetLink, WobuNode } from '../../lib/api'
import { node as buildNode } from '../../test/fixtures'
import { ReferencesPane } from './ReferencesPane'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: () => Promise.resolve(() => {}) }),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: () => Promise.resolve(null) }))

const autosave = {
  queue: vi.fn<(patch: Partial<WobuNode>) => void>(),
  flush: vi.fn<() => void>(),
  status: 'idle' as const,
}

function asset(id: string, over: Partial<Asset> = {}): Asset {
  return {
    id,
    hash: id,
    kind: 'reference',
    relPath: `assets/originals/${id}.png`,
    thumbPath: `assets/thumbs/${id}.webp`,
    mime: 'image/png',
    width: 1200,
    height: 800,
    bytes: 2048,
    createdAt: '2026-08-01T10:00:00Z',
    ...over,
  }
}

function link(assetId: string, role: AssetLink['role'] = 'full_ref'): AssetLink {
  return { assetId, role, weight: 1, enabled: true }
}

function renderPane(node: WobuNode, assets: Asset[], readOnly = false) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'asset_list') return Promise.resolve(assets)
    if (command === 'asset_thumb') return Promise.resolve(`/thumb/${String(args?.assetId)}.webp`)
    if (command === 'asset_set_cover') {
      return Promise.resolve({ ...node, coverAssetId: args?.assetId as string | null })
    }
    if (command === 'asset_link') {
      const attached = link(String(args?.assetId), args?.role as AssetLink['role'])
      return Promise.resolve({ ...node, assetLinks: [...node.assetLinks, attached] })
    }
    return Promise.resolve(null)
  })
  return render(
    <QueryClientProvider client={qc}>
      <ReferencesPane node={node} readOnly={readOnly} autosave={autosave} />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  h.invoke.mockReset()
  autosave.queue.mockReset()
  autosave.flush.mockReset()
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, value: 800 })
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

describe('ReferencesPane', () => {
  it('windows a large thumbnail grid instead of mounting every tile', async () => {
    const links = Array.from({ length: 40 }, (_, index) => link(`asset-${index}`))
    const assets = links.map((item) => asset(item.assetId))
    const view = renderPane(buildNode({ id: 'kael', assetLinks: links }), assets)

    await waitFor(() =>
      expect(view.container.querySelectorAll('.reference-tile').length).toBeGreaterThan(0),
    )
    const mounted = view.container.querySelectorAll('.reference-tile').length
    expect(mounted).toBeGreaterThan(0)
    expect(mounted).toBeLessThan(links.length)
    expect(view.container.querySelector('.reference-grid')).toHaveStyle({ height: '4228px' })
  })

  it('edits role and weight, reorders, removes, and chooses a cover', async () => {
    const links = [link('one', 'silhouette'), link('two', 'palette')]
    const node = buildNode({ id: 'kael', assetLinks: links })
    renderPane(node, [asset('one'), asset('two')])

    fireEvent.change(await screen.findByLabelText('Role for reference 1'), {
      target: { value: 'material' },
    })
    expect(autosave.queue).toHaveBeenLastCalledWith({
      assetLinks: [{ ...links[0], role: 'material' }, links[1]],
    })

    expect(screen.getByRole('slider', { name: 'Weight for reference 1' })).toHaveAttribute(
      'aria-valuetext',
      '100 percent',
    )

    fireEvent.change(screen.getByLabelText('Weight for reference 1'), {
      target: { value: '0.35' },
    })
    expect(autosave.queue).toHaveBeenLastCalledWith({
      assetLinks: [{ ...links[0], role: 'material', weight: 0.35 }, links[1]],
    })

    fireEvent.click(screen.getAllByRole('button', { name: '→' })[0] as HTMLElement)
    expect(autosave.queue).toHaveBeenLastCalledWith({
      assetLinks: [links[1], { ...links[0], role: 'material', weight: 0.35 }],
    })

    fireEvent.click(screen.getAllByRole('button', { name: 'Set cover' })[0] as HTMLElement)
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('asset_set_cover', {
        nodeId: 'kael',
        assetId: 'two',
      }),
    )

    fireEvent.click(screen.getAllByRole('button', { name: 'Remove' })[0] as HTMLElement)
    expect(autosave.queue).toHaveBeenLastCalledWith({
      assetLinks: [{ ...links[0], role: 'material', weight: 0.35 }],
    })
  })

  it('imports pasted images, attaches them, and reports each file outcome', async () => {
    const node = buildNode({ id: 'kael' })
    renderPane(node, [])
    const good = imageFile('good.png', 1)
    const bad = imageFile('bad.png', 2)

    h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === 'asset_list') return Promise.resolve([])
      if (command === 'asset_thumb') return Promise.resolve(`/thumb/${String(args?.assetId)}.webp`)
      if (command === 'asset_import_bytes') {
        const bytes = args?.bytes as number[]
        if (bytes[0] === 2) {
          return Promise.reject({
            code: 'asset.not_an_image',
            message: 'The file is not a supported image.',
            retryable: false,
          })
        }
        return Promise.resolve({ asset: asset('pasted-good'), deduped: false })
      }
      if (command === 'asset_link') {
        return Promise.resolve({ ...node, assetLinks: [link(String(args?.assetId))] })
      }
      return Promise.resolve(null)
    })

    fireEvent.paste(screen.getByRole('region', { name: 'References for kael' }), {
      clipboardData: { files: [good, bad] },
    })

    expect(await screen.findByText('good.png')).toBeInTheDocument()
    expect(screen.getByText('bad.png')).toBeInTheDocument()
    await screen.findByText('Imported · attached')
    await screen.findByText('Failed')
    expect(screen.getByText(/Import failed: The file is not a supported image/)).toBeInTheDocument()
    expect(h.invoke).toHaveBeenCalledWith(
      'asset_link',
      expect.objectContaining({ nodeId: 'kael', assetId: 'pasted-good', role: 'full_ref' }),
    )
  })

  it('suppresses import controls and drop affordances when read-only', () => {
    const node = buildNode({ id: 'kael' })
    renderPane(node, [], true)
    const region = screen.getByRole('region', { name: 'References for kael' })

    expect(screen.getByRole('button', { name: 'Add images…' })).toBeDisabled()
    fireEvent.dragEnter(region, { dataTransfer: { types: ['Files'] } })
    expect(screen.queryByText('Drop images to import and attach')).not.toBeInTheDocument()
    fireEvent.paste(region, { clipboardData: { files: [imageFile('ignored.png', 1)] } })
    expect(h.invoke).not.toHaveBeenCalledWith('asset_import_bytes', expect.anything())
  })
})

function imageFile(name: string, marker: number): File {
  const file = new File([new Uint8Array([marker])], name, { type: 'image/png' })
  Object.defineProperty(file, 'arrayBuffer', {
    value: () => Promise.resolve(Uint8Array.from([marker]).buffer),
  })
  return file
}
