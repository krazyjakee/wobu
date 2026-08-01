import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { MeshConcept } from '../../lib/api'
import { node } from '../../test/fixtures'
import { ThreePane } from './ThreePane'

const h = vi.hoisted(() => ({ invoke: vi.fn(), save: vi.fn(), reveal: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: h.save }))
vi.mock('@tauri-apps/plugin-opener', () => ({ revealItemInDir: h.reveal }))
vi.mock('./MeshViewport', () => ({
  default: ({ url, turntable, wireframe }: { url: string; turntable: boolean; wireframe: boolean }) => (
    <div
      data-testid="mesh-viewport"
      data-url={url}
      data-turntable={String(turntable)}
      data-wireframe={String(wireframe)}
    />
  ),
}))

const subject = node({ id: 'kael', name: 'Kael Vantris' })
const views = [
  'front',
  'left',
  'right',
  'back',
  'top',
  'bottom',
  'left_front',
  'right_front',
].map((viewType, index) => ({
  generationId: `generation-${index}`,
  viewType,
  assetId: `image-${index}`,
}))
const concept: MeshConcept = {
  generationId: 'mesh-generation',
  createdAt: '2026-08-01T12:00:00Z',
  backend: 'Tencent Hunyuan3D',
  model: '3.1',
  asset: {
    id: 'mesh-asset',
    hash: '7b'.repeat(32),
    bytes: 5_242_880,
    createdAt: '2026-08-01T12:00:00Z',
  },
  turnaround: views,
}

function open(meshes: MeshConcept[] = [concept]) {
  h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'mesh_concepts') return Promise.resolve(meshes)
    if (command === 'mesh_asset_path') return Promise.resolve('/project/assets/meshes/model.glb')
    if (command === 'mesh_source_path') return Promise.resolve('/project/assets/meshes/canonical.glb')
    if (command === 'asset_thumb') return Promise.resolve(`/thumbs/${String(args?.assetId)}.webp`)
    if (command === 'mesh_export') return Promise.resolve(null)
    return Promise.resolve(null)
  })
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <ThreePane node={subject} />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  h.invoke.mockReset()
  h.save.mockReset()
  h.reveal.mockReset()
  h.save.mockResolvedValue('/exports/kael.glb')
  h.reveal.mockResolvedValue(undefined)
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('the 3D pane', () => {
  it('loads one GLB, exposes viewer controls, and shows its recorded turnaround', async () => {
    open()

    const viewer = await screen.findByTestId('mesh-viewport')
    expect(viewer.dataset.url).toBe('asset:///project/assets/meshes/model.glb')
    expect(viewer.dataset.turntable).toBe('true')
    expect(viewer.dataset.wireframe).toBe('false')
    expect(h.invoke).toHaveBeenCalledWith('mesh_concepts', { nodeId: subject.id })
    expect(h.invoke).toHaveBeenCalledWith('mesh_asset_path', { assetId: 'mesh-asset' })

    const sheet = within(screen.getByLabelText('Source turnaround sheet'))
    expect(await sheet.findByAltText('front turnaround view')).toBeTruthy()
    expect(sheet.getAllByRole('img')).toHaveLength(8)

    fireEvent.click(screen.getByRole('button', { name: 'Turntable' }))
    fireEvent.click(screen.getByRole('button', { name: 'Wireframe' }))
    expect(viewer.dataset.turntable).toBe('false')
    expect(viewer.dataset.wireframe).toBe('true')
  })

  it('reveals the canonical file and exports a validated copy', async () => {
    open()
    await screen.findByTestId('mesh-viewport')

    fireEvent.click(screen.getByRole('button', { name: 'Reveal GLB' }))
    fireEvent.click(screen.getByRole('button', { name: 'Export copy…' }))
    await waitFor(() => {
      expect(h.reveal).toHaveBeenCalledWith('/project/assets/meshes/canonical.glb')
      expect(h.save).toHaveBeenCalled()
      expect(h.invoke).toHaveBeenCalledWith('mesh_export', {
        assetId: 'mesh-asset',
        destination: '/exports/kael.glb',
      })
    })
  })

  it('does not guess a source sheet when the receipt omitted it', async () => {
    open([{ ...concept, turnaround: [] }])
    const sheet = within(await screen.findByLabelText('Source turnaround sheet'))
    expect(sheet.getByText(/did not record a complete source sheet/)).toBeTruthy()
    expect(sheet.queryByRole('img')).toBeNull()
  })
})
