import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { MeshOptions, QueueSnapshot, TurnaroundSheet } from '../../lib/api'
import { node } from '../../test/fixtures'
import { TurnaroundReview } from './TurnaroundReview'

const h = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (e: unknown) => void>(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, handler: (e: unknown) => void) => {
    h.listeners.set(name, handler)
    return Promise.resolve(() => h.listeners.delete(name))
  },
}))

const subject = node({ id: 'kael', name: 'Kael Vantris' })
const VIEWS = [
  'front',
  'left',
  'right',
  'back',
  'top',
  'bottom',
  'left_front',
  'right_front',
] as const
const emptyQueue: QueueSnapshot = { jobs: [], queued: 0, running: 0, retrying: 0 }

function take(viewType: string, seed: number, suffix = '') {
  return {
    generationId: `gen-${viewType}${suffix}`,
    assetId: `asset-${viewType}${suffix}`,
    seed,
    createdAt: suffix ? '2026-08-03T12:00:00Z' : '2026-08-01T12:00:00Z',
    backend: 'comfyui',
    model: 'flux-dev',
  }
}

function fullSheet(over: Partial<TurnaroundSheet> = {}): TurnaroundSheet {
  return {
    views: VIEWS.map((viewType) => ({ viewType, takes: [take(viewType, 11)] })),
    batches: [
      {
        seed: 11,
        createdAt: '2026-08-01T12:00:00Z',
        generationIds: VIEWS.map((viewType) => `gen-${viewType}`),
      },
    ],
    missing: [],
    ...over,
  }
}

const emptySheet: TurnaroundSheet = {
  views: VIEWS.map((viewType) => ({ viewType, takes: [] })),
  batches: [],
  missing: [...VIEWS],
}

const hosted: MeshOptions = {
  provider: 'hunyuan3d',
  label: 'Tencent Hunyuan3D',
  model: '3.1',
  region: 'ap-singapore',
  maxViews: 8,
  faceCountMin: 3_000,
  faceCountMax: 1_500_000,
  defaultFaceCount: 500_000,
  pbr: true,
  generateTypes: ['Normal', 'Geometry'],
  requiresBilling: true,
  ready: true,
  detail: '',
}

let sheet: TurnaroundSheet
let options: MeshOptions

function open(queue: QueueSnapshot = emptyQueue, readOnly = false) {
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <TurnaroundReview node={subject} queue={queue} readOnly={readOnly} />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  sheet = fullSheet()
  options = { ...hosted }
  h.invoke.mockReset()
  h.listeners.clear()
  h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'turnaround_sheet') return Promise.resolve(sheet)
    if (command === 'mesh_options') return Promise.resolve(options)
    if (command === 'asset_thumb') return Promise.resolve(`/thumb-${String(args?.assetId)}.webp`)
    if (command === 'generate_start') return Promise.resolve('job-generate')
    if (command === 'mesh_start') return Promise.resolve('job-mesh')
    if (command === 'job_cancel') return Promise.resolve(true)
    return Promise.resolve(null)
  })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('the turnaround review', () => {
  it('offers the whole sheet when the entity has never had one', async () => {
    sheet = emptySheet
    open()
    fireEvent.click(await screen.findByRole('button', { name: 'Generate turnaround' }))
    await waitFor(() => {
      expect(h.invoke).toHaveBeenCalledWith('generate_start', {
        subjectId: 'kael',
        preset: 'turnaround',
      })
    })
  })

  it('re-rolls one view as one image on a seed of its own', async () => {
    // The Turnaround preset locks a single seed across all eight views, so the
    // *only* way "this back view came out wrong" can be answered is one tagged
    // generation with a different seed. Sending the preset again would be eight
    // more images and a different character.
    open()
    fireEvent.click(await screen.findByRole('button', { name: 'Re-roll the back view' }))
    await waitFor(() => expect(h.invoke).toHaveBeenCalledWith('generate_start', expect.anything()))
    const [, args] = h.invoke.mock.calls.find(([command]) => command === 'generate_start') as [
      string,
      { views: string[]; seed: number; preset: string },
    ]
    expect(args.preset).toBe('turnaround')
    expect(args.views).toEqual(['back'])
    expect(args.seed).toBeGreaterThan(0)
  })

  it('will not reconstruct until every required view has a take', async () => {
    sheet = fullSheet({
      views: VIEWS.map((viewType) => ({
        viewType,
        takes: viewType === 'top' ? [] : [take(viewType, 11)],
      })),
      batches: [],
      missing: ['top'],
    })
    open()
    // Refused with `aria-disabled` rather than `disabled`, so the reason is
    // reachable by hover and by focus instead of being unreachable by both.
    const button = await screen.findByRole('button', { name: 'Reconstruct mesh' })
    expect(button).toHaveAttribute('aria-disabled', 'true')
    fireEvent.focusIn(button)
    expect(screen.getByRole('tooltip')).toHaveTextContent('Still missing: top')
    expect(screen.getByRole('status')).toHaveTextContent('Not rendered yet: top')
  })

  it('holds a paid reconstruction behind an explicit consent, then sends the reviewed views', async () => {
    // Hunyuan3D charges per submitted job and the international API does not
    // report the amount back, so there is nothing to show the user up front.
    // Consent is the only honest gate.
    open()
    const button = await screen.findByRole('button', { name: 'Reconstruct mesh' })
    expect(button).toHaveAttribute('aria-disabled', 'true')

    fireEvent.click(screen.getByRole('checkbox', { name: /charges for every submitted job/ }))
    fireEvent.change(screen.getByLabelText('Face count'), { target: { value: '250000' } })
    fireEvent.change(screen.getByLabelText('Reconstruction mode'), {
      target: { value: 'Geometry' },
    })
    expect(button).not.toHaveAttribute('aria-disabled')
    fireEvent.click(button)

    await waitFor(() => {
      expect(h.invoke).toHaveBeenCalledWith('mesh_start', {
        nodeId: 'kael',
        generationIds: VIEWS.map((viewType) => `gen-${viewType}`),
        faceCount: 250_000,
        enablePbr: false,
        generateType: 'Geometry',
        acceptCost: true,
      })
    })
  })

  it('sends only what a single-image backend takes, and says which', async () => {
    // The local tier is a different quality tier rather than a fallback. It
    // reconstructs from the front view alone, and showing eight tiles while
    // silently sending one would be a worse mesh with nothing on screen to
    // explain it.
    options = {
      ...hosted,
      provider: 'comfyui',
      label: 'Local Hunyuan3D 2.1 (ComfyUI)',
      model: 'hunyuan3d-dit-v2-1.ckpt',
      maxViews: 1,
      pbr: false,
      generateTypes: ['Geometry'],
      requiresBilling: false,
    }
    open()
    expect(await screen.findByText(/sending 1 view \(front\)/)).toBeInTheDocument()
    expect(screen.getByLabelText('left view')).toHaveTextContent('not sent to this provider')

    fireEvent.click(screen.getByRole('button', { name: 'Reconstruct mesh' }))
    await waitFor(() => {
      expect(h.invoke).toHaveBeenCalledWith(
        'mesh_start',
        expect.objectContaining({ generationIds: ['gen-front'] }),
      )
    })
  })

  it('reports a running reconstruction and can stop it', async () => {
    const queue: QueueSnapshot = {
      ...emptyQueue,
      running: 1,
      jobs: [
        {
          id: 'job-mesh',
          kind: 'mesh',
          label: 'Mesh Kael Vantris',
          subjectId: 'kael',
          attempt: 1,
          elapsedMs: 4_000,
          state: 'running',
        },
      ],
    }
    open(queue)
    const active = await screen.findByText(/Mesh Kael Vantris · working/)
    expect(active).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Reconstruct mesh' })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await waitFor(() => expect(h.invoke).toHaveBeenCalledWith('job_cancel', { jobId: 'job-mesh' }))
  })

  it('shows what a failed reconstruction cost rather than only that it failed', async () => {
    const queue: QueueSnapshot = {
      ...emptyQueue,
      jobs: [
        {
          id: 'job-mesh',
          kind: 'mesh',
          label: 'Mesh Kael Vantris',
          subjectId: 'kael',
          attempt: 1,
          elapsedMs: 90_000,
          state: 'failed',
          retryHeld: true,
          failure: {
            code: 'provider.bad_response',
            message: 'Tencent Hunyuan3D finished the job and returned no mesh.',
            retryable: true,
            billed: 'charged',
          },
        },
      ],
    }
    open(queue)
    const message = await screen.findByText(/returned no mesh/)
    expect(message).toHaveTextContent('You were charged for this attempt.')
  })

  it('cycles between takes for one view and reconstructs from the chosen one', async () => {
    sheet = fullSheet({
      views: VIEWS.map((viewType) => ({
        viewType,
        takes:
          viewType === 'back' ? [take('back', 99, '-b'), take('back', 11)] : [take(viewType, 11)],
      })),
    })
    open()
    const back = within(await screen.findByLabelText('back view'))
    expect(back.getByRole('button', { name: 'Cycle back take' })).toHaveTextContent('take 1/2')
    fireEvent.click(back.getByRole('button', { name: 'Cycle back take' }))
    expect(back.getByRole('button', { name: 'Cycle back take' })).toHaveTextContent('take 2/2')

    fireEvent.click(screen.getByRole('checkbox', { name: /charges for every submitted job/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Reconstruct mesh' }))
    await waitFor(() => {
      expect(h.invoke).toHaveBeenCalledWith(
        'mesh_start',
        expect.objectContaining({
          generationIds: VIEWS.map((viewType) =>
            viewType === 'back' ? 'gen-back' : `gen-${viewType}`,
          ),
        }),
      )
    })
  })

  it('re-reads the sheet when a receipt for this entity lands', async () => {
    // The 3D tab subscribes for itself: `useGenerations` invalidates the mesh
    // gallery, but it is only mounted by the Concepts tab, and a reroll started
    // here finishes while the user is looking at this one.
    open()
    await screen.findByLabelText('front view')
    const before = h.invoke.mock.calls.filter(([c]) => c === 'turnaround_sheet').length
    h.listeners.get('generation:recorded')?.({
      payload: { subjectId: 'kael', generation: null, asset: null },
    })
    await waitFor(() => {
      expect(h.invoke.mock.calls.filter(([c]) => c === 'turnaround_sheet').length).toBe(before + 1)
    })
  })

  it('says why it cannot run instead of failing at the provider', async () => {
    options = {
      ...hosted,
      ready: false,
      detail: 'Tencent Hunyuan3D is selected for 3D, but this machine is missing its keys.',
    }
    open()
    expect(await screen.findByText(/missing its keys/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('checkbox', { name: /charges for every submitted job/ }))
    expect(screen.getByRole('button', { name: 'Reconstruct mesh' })).toHaveAttribute(
      'aria-disabled',
      'true',
    )
  })

  it('refuses every control on a read-only project', async () => {
    open(emptyQueue, true)
    expect(await screen.findByRole('button', { name: 'Re-roll the back view' })).toBeDisabled()
    const mesh = screen.getByRole('button', { name: 'Reconstruct mesh' })
    expect(mesh).toHaveAttribute('aria-disabled', 'true')
    fireEvent.focusIn(mesh)
    expect(screen.getByRole('tooltip')).toHaveTextContent('read-only')
  })
})
