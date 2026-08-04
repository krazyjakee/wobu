import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ProjectSummary } from '../lib/api'
import { resetNodeThumbs } from '../lib/nodeThumbs'
import { kindDef, kindIndex, summary } from '../test/fixtures'
import { Inspector } from './Inspector'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const kinds = kindIndex([kindDef('character'), kindDef('culture', { layer: 'culture' })])
const selected = summary({ id: 'kael', name: 'Kael' })
const project: ProjectSummary = {
  id: 'project',
  name: 'Ashfall',
  path: '/project',
  onNetworkShare: false,
  readOnly: false,
  lastOpenedAt: null,
}
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

/** Two entities and the shot: only the entities can have a picture. */
function layer(nodeId: string | null, name: string, layerName: string) {
  return {
    layer: layerName,
    nodeId,
    name,
    kind: nodeId ? 'character' : null,
    reached: 'subject',
    distance: 0,
    weight: 1,
    slider: 1,
    fragments: [],
  }
}

function draw(onJump: (id: string) => void = () => {}) {
  return render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <Inspector project={project} selected={selected} kinds={kinds} onJump={onJump} />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  resetNodeThumbs()
  h.invoke.mockReset()
  h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'preset_list') return Promise.resolve([preset])
    if (command === 'status_bar_backend') {
      return Promise.resolve({
        image: { provider: 'comfyui', label: 'ComfyUI', model: 'flux-dev', contextTokens: null },
        text: { provider: 'anthropic', label: 'Anthropic', model: 'claude', contextTokens: 1 },
        health: { state: 'connected', externalQueue: 0 },
      })
    }
    if (command === 'influence_resolve') {
      return Promise.resolve({
        subjectId: args?.subjectId,
        preset,
        layers: [
          layer('guild', 'Harbour Guild', 'culture'),
          layer('kael', 'Kael', 'subject'),
          layer(null, 'Portrait', 'shot'),
        ],
      })
    }
    if (command === 'prompt_compile') {
      return Promise.resolve({
        subjectId: args?.subjectId,
        preset,
        prompt: 'ash-grey coat',
        negative: '',
        spans: [],
        dropped: [],
        overflow: null,
      })
    }
    if (command === 'image_reference_report') {
      return Promise.resolve({ buckets: [], layers: [], cost: null, lockedSeed: null })
    }
    if (command === 'image_generation_capabilities') {
      return Promise.resolve({
        provider: 'comfyui',
        model: 'flux-dev',
        aspectRatios: ['3:4'],
        flexibleAspect: false,
        previews: [
          {
            requestedAspect: '3:4',
            actualAspect: '3:4',
            width: 1536,
            height: 2048,
            substituted: false,
          },
        ],
      })
    }
    if (command === 'spend_status') {
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
    if (command === 'node_thumb_batch') return Promise.resolve({ kael: '/thumbs/kael.webp' })
    return Promise.resolve(null)
  })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

afterEach(() => {
  resetNodeThumbs()
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

describe('influence stack thumbnails', () => {
  it('draws the picture of the entity a layer comes from', async () => {
    const view = draw()

    await waitFor(() =>
      expect(view.container.querySelector('.layer-h .node-thumb img')).toHaveAttribute(
        'src',
        'asset:///thumbs/kael.webp',
      ),
    )
  })

  it('keeps every layer the same height, picture or coloured dot', async () => {
    const view = draw()
    const headers = () => [...view.container.querySelectorAll('.layer-h')]

    await waitFor(() => expect(headers()).toHaveLength(3))
    expect(headers().map((head) => head.querySelectorAll('.node-thumb').length)).toEqual([1, 1, 1])

    await waitFor(() => expect(view.container.querySelector('.node-thumb img')).not.toBeNull())

    // The layer with a picture, the one without and the shot are the same box
    // in the same place; the layer dot is what an empty one falls back to, so
    // nothing below a resolved thumbnail moves.
    expect(headers().map((head) => head.querySelectorAll('.node-thumb').length)).toEqual([1, 1, 1])
    const empty = view.container.querySelectorAll('.layer-h .node-thumb.is-empty')
    expect(empty).toHaveLength(2)
    for (const slot of empty) expect(slot.querySelector('.layer-dot')).not.toBeNull()
  })

  it('asks once for the whole stack and never about the shot', async () => {
    draw()

    await waitFor(() =>
      expect(h.invoke.mock.calls.some(([command]) => command === 'node_thumb_batch')).toBe(true),
    )
    const batches = h.invoke.mock.calls.filter(([command]) => command === 'node_thumb_batch')
    expect(batches).toHaveLength(1)
    // The shot layer has no entity behind it, so there is nothing to ask about.
    expect((batches[0]?.[1] as { nodeIds: string[] }).nodeIds).toEqual(['guild', 'kael'])
  })
})

/*
 * The layer heading's context menu (#130).
 *
 * A layer is a `<details>`, and both of the things you can do to one — mute it,
 * open the entity it came from — are inside the fold. On a stack of six that is
 * a disclosure and a scroll before the first click. The heading is on screen
 * either way, so the menu goes there.
 */
describe('influence layer context menu', () => {
  function heading(view: ReturnType<typeof draw>, index: number) {
    return view.container.querySelectorAll('.layer-h')[index] as HTMLElement
  }

  it('mutes the layer and opens its source from the heading, by key or pointer', async () => {
    const jumped = vi.fn()
    const view = draw(jumped)
    await waitFor(() => expect(view.container.querySelectorAll('.layer-h')).toHaveLength(3))

    fireEvent.keyDown(heading(view, 0), { key: 'F10', shiftKey: true })
    const menu = screen.getByRole('menu', { name: 'Actions for the culture layer' })
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent?.trim()),
    ).toEqual(['Mute this layer', 'Open source'])

    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Mute this layer' }))
    // Muting re-resolves the stack, so the layers are drawn again from the
    // answer rather than from the click.
    await waitFor(() => expect(view.container.querySelectorAll('.layer.is-muted')).toHaveLength(1))
    // The card's own button now says the same thing the menu will next time.
    expect(screen.getAllByRole('button', { name: 'Unmute' })).toHaveLength(1)

    fireEvent.contextMenu(heading(view, 1), { clientX: 10, clientY: 20 })
    fireEvent.click(screen.getByRole('menuitem', { name: 'Open source' }))
    expect(jumped).toHaveBeenCalledWith('kael')
  })

  it('refuses to open a source the shot layer does not have, and says why', async () => {
    const view = draw()
    await waitFor(() => expect(view.container.querySelectorAll('.layer-h')).toHaveLength(3))

    fireEvent.contextMenu(heading(view, 2), { clientX: 10, clientY: 20 })
    const menu = screen.getByRole('menu', { name: 'Actions for the shot layer' })
    const source = within(menu).getByRole('menuitem', { name: 'Open source' })
    expect(source).toHaveAttribute('aria-disabled', 'true')

    fireEvent.focus(source)
    expect(
      document.getElementById(source.getAttribute('aria-describedby') ?? ''),
    ).toHaveTextContent(/the shot itself/)
    // Muting the shot is a real thing to do, so the menu is not pointless here.
    expect(within(menu).getByRole('menuitem', { name: 'Mute this layer' })).not.toHaveAttribute(
      'aria-disabled',
    )
  })
})
