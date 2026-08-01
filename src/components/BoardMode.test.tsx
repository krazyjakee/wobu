import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Asset, NodeSummary, WobuNode } from '../lib/api'
import { kindDef, kindIndex, node as buildNode, summary } from '../test/fixtures'
import { useBoard } from '../store/board'
import { useUI } from '../store/ui'
import { BoardMode, type BoardAttachRequest } from './BoardMode'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))

const nodes: NodeSummary[] = [summary({ id: 'kael', name: 'Kael', kind: 'character' })]
const kinds = kindIndex([kindDef('character', { label: 'Character' })])
let assets: Asset[] = []

function asset(index: number): Asset {
  return {
    id: `asset-${index}`,
    hash: `hash-${index}`,
    kind: 'reference',
    relPath: `assets/originals/asset-${index}.png`,
    thumbPath: `assets/thumbs/asset-${index}.webp`,
    mime: 'image/png',
    width: 1200,
    height: 800,
    bytes: 2048,
    createdAt: '2026-08-01T10:00:00Z',
  }
}

function transfer() {
  const values = new Map<string, string>()
  const types: string[] = []
  return {
    types,
    effectAllowed: 'all',
    dropEffect: 'none',
    setData(type: string, value: string) {
      if (!types.includes(type)) types.push(type)
      values.set(type, value)
    },
    getData(type: string) {
      return values.get(type) ?? ''
    },
  }
}

function open(pendingAttach: BoardAttachRequest | null = null, readOnly = false) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  const onPendingAttach = vi.fn()
  const view = render(
    <QueryClientProvider client={qc}>
      <BoardMode
        projectId="ashfall"
        nodes={nodes}
        kinds={kinds}
        readOnly={readOnly}
        navigatorVisible
        pendingAttach={pendingAttach}
        onPendingAttach={onPendingAttach}
      />
    </QueryClientProvider>,
  )
  return { ...view, onPendingAttach }
}

beforeEach(() => {
  assets = []
  localStorage.removeItem('wobu.board-layouts.v1')
  useBoard.setState({ projects: {} })
  useUI.setState({ selectedId: 'kael', toasts: [] })
  h.invoke.mockReset()
  h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'asset_list') return Promise.resolve(assets)
    if (command === 'asset_thumb') return Promise.resolve(`/thumb/${String(args?.assetId)}.webp`)
    if (command === 'asset_link') {
      return Promise.resolve(
        buildNode({
          id: String(args?.nodeId),
          assetLinks: [{
            assetId: String(args?.assetId),
            role: args?.role as 'mood',
            weight: 1,
            enabled: true,
          }],
        }) satisfies WobuNode,
      )
    }
    if (command === 'node_list') return Promise.resolve(nodes)
    return Promise.resolve(null)
  })
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, value: 800 })
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, value: 600 })
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('BoardMode', () => {
  it('pans with the viewport wheel and zooms around the canvas', async () => {
    const view = open()
    await screen.findByText('No images yet')
    const viewport = view.container.querySelector('.board-viewport')!
    fireEvent.wheel(viewport, { deltaX: 30, deltaY: 20, clientX: 400, clientY: 300 })
    expect(view.container.querySelector('.board-world')).toHaveStyle({
      transform: 'translate(18px, 52px) scale(1)',
    })

    fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))
    expect(screen.getByLabelText('Board zoom')).toHaveTextContent('125%')
  })

  it('culls off-screen images and only asks for mounted thumbnails', async () => {
    assets = Array.from({ length: 80 }, (_, index) => asset(index))
    const view = open()

    await waitFor(() => expect(view.container.querySelectorAll('.board-asset').length).toBeGreaterThan(0))
    expect(view.container.querySelectorAll('.board-asset').length).toBeLessThan(assets.length)
    expect(screen.getByText(/of 80 images in view/)).toBeInTheDocument()
    expect(h.invoke).not.toHaveBeenCalledWith('asset_original', expect.anything())
  })

  it('moves a freely dragged image into board coordinates and persists it', async () => {
    assets = [asset(0)]
    const view = open()
    const tile = await screen.findByRole('article', { name: 'Reference asset asset-0' })
    const dataTransfer = transfer()
    fireEvent.dragStart(tile, { dataTransfer })
    fireEvent.drop(view.container.querySelector('.board-viewport')!, {
      dataTransfer,
      clientX: 600,
      clientY: 450,
    })

    await waitFor(() => {
      const point = useBoard.getState().projects.ashfall?.positions['asset-0']
      expect(point?.x).toBeCloseTo(442)
      expect(point?.y).toBeCloseTo(288)
    })
  })

  it('opens the chosen-role flow when an image is dropped on the selected node chip', async () => {
    assets = [asset(0)]
    const view = open()
    const tile = await screen.findByRole('article', { name: 'Reference asset asset-0' })
    const dataTransfer = transfer()
    fireEvent.dragStart(tile, { dataTransfer })
    fireEvent.drop(screen.getByText('Kael').closest('.board-node-chip')!, { dataTransfer })

    expect(view.onPendingAttach).toHaveBeenCalledWith({ assetId: 'asset-0', nodeId: 'kael' })
  })

  it('attaches a navigator drop with the explicitly chosen role', async () => {
    assets = [asset(0)]
    open({ assetId: 'asset-0', nodeId: 'kael' })
    const roleSelect = await screen.findByLabelText('Reference role')
    const dialog = screen.getByRole('dialog', { name: 'Attach board image' })
    fireEvent.change(roleSelect, { target: { value: 'pose' } })
    fireEvent.click(within(dialog).getByRole('button', { name: 'Attach reference' }))

    await waitFor(() => expect(h.invoke).toHaveBeenCalledWith('asset_link', {
      nodeId: 'kael',
      assetId: 'asset-0',
      role: 'pose',
      weight: undefined,
    }))
  })

  it('does not invent a selected-node target and blocks writes when read-only', async () => {
    assets = [asset(0)]
    useUI.setState({ selectedId: null })
    const view = open({ assetId: 'asset-0', nodeId: 'kael' }, true)
    expect(await screen.findByText(/Select a node in the navigator/)).toBeInTheDocument()
    expect(view.container.querySelector('.board-node-chip')).toBeNull()
    expect(screen.getByRole('button', { name: 'Attach reference' })).toBeDisabled()
  })
})
