import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { NodeSummary } from '../../lib/api'
import { resetNodeThumbs } from '../../lib/nodeThumbs'
import { buildGroups, indexNodes } from '../../lib/tree'
import { useUI } from '../../store/ui'
import { kindDef, kindIndex, summary } from '../../test/fixtures'
import { Navigator } from './Navigator'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))

const kinds = kindIndex([kindDef('character', { label: 'Character', plural: 'Characters' })])

function world(count: number): NodeSummary[] {
  return Array.from({ length: count }, (_, index) =>
    summary({ id: `node-${index}`, name: `Node ${index}`, kind: 'character' }),
  )
}

function draw(nodes: NodeSummary[], pinned: NodeSummary[] = []) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <Navigator
        nodes={nodes}
        byId={indexNodes(nodes)}
        pinned={pinned}
        groups={buildGroups(nodes, ['character'], kinds)}
        kinds={kinds}
        loading={false}
        error={null}
        readOnly={false}
        corrupt={[]}
        editedElsewhere={new Map()}
        projectPath="/project"
        onNewNode={() => {}}
      />
    </QueryClientProvider>,
  )
}

function row(name: string): HTMLElement {
  return screen.getByRole('button', { name: new RegExp(`^${name}$`) })
}

beforeEach(() => {
  resetNodeThumbs()
  h.invoke.mockReset()
  h.invoke.mockImplementation((cmd: string) =>
    Promise.resolve(cmd === 'node_thumb_batch' ? { 'node-1': '/thumbs/one.webp' } : null),
  )
  useUI.setState({ filter: '', selectedId: null, collapsedNodes: {}, closedGroups: {} })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

afterEach(() => {
  resetNodeThumbs()
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

describe('navigator row thumbnails', () => {
  it('draws the picture of a node that has one', async () => {
    draw(world(3))

    await waitFor(() =>
      expect(within(row('Node 1')).getByRole('presentation', { hidden: true })).toHaveAttribute(
        'src',
        'asset:///thumbs/one.webp',
      ),
    )
  })

  it('keeps the same slot on every row whether or not there is a picture', async () => {
    // The whole point of the fallback: rows are virtualized against a constant
    // row height, so the box that holds the picture must exist, and be one box,
    // before and after the batch resolves. A slot that appeared only for rows
    // with images would move every row below it the moment one arrived.
    const view = draw(world(3))
    const slots = () => [...view.container.querySelectorAll('.nav-virtual-window .node')]

    const before = slots().map((node) => node.querySelectorAll('.node-thumb').length)
    expect(before).toEqual([1, 1, 1])

    await waitFor(() => expect(view.container.querySelector('.node-thumb img')).not.toBeNull())

    expect(slots().map((node) => node.querySelectorAll('.node-thumb').length)).toEqual([1, 1, 1])
    // The one with a picture and the two without are the same element, in the
    // same place, with only the modifier telling them apart.
    expect(slots().map((node) => node.querySelector('.node-thumb')?.className)).toEqual([
      'node-thumb is-empty',
      'node-thumb has-image',
      'node-thumb is-empty',
    ])
  })

  it('falls back to the kind icon, and never to a gap, when there is no picture', async () => {
    const view = draw(world(3))
    await waitFor(() => expect(h.invoke).toHaveBeenCalled())

    const empty = view.container.querySelectorAll('.node-thumb.is-empty')
    expect(empty).toHaveLength(2)
    for (const slot of empty) expect(slot.querySelector('svg.ic')).not.toBeNull()
  })

  it('leaves the accessible name of a row untouched', async () => {
    draw(world(3))
    await waitFor(() => expect(h.invoke).toHaveBeenCalled())

    // A decorative image inside the row button would otherwise say the node's
    // name a second time, and break every query that matches a row by name.
    expect(row('Node 1')).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: /^Node 1$/ })).toHaveLength(1)
  })

  it('asks once for the visible window rather than once per row', async () => {
    draw(world(400))
    await waitFor(() => expect(h.invoke).toHaveBeenCalled())

    const thumbCalls = h.invoke.mock.calls.filter(([cmd]) => cmd === 'node_thumb_batch')
    expect(thumbCalls).toHaveLength(1)
    const asked = (thumbCalls[0]?.[1] as { nodeIds: string[] }).nodeIds
    expect(asked.length).toBeGreaterThan(0)
    // Bounded by the window, not by the world, and never past the backend's cap.
    expect(asked.length).toBeLessThanOrEqual(100)
    expect(asked.length).toBeLessThan(400)
  })

  it('gives the pinned strip the same slot as the tree', async () => {
    const nodes = world(3)
    const view = draw(nodes, [nodes[1] as NodeSummary])

    await waitFor(() => expect(h.invoke).toHaveBeenCalled())
    const pin = view.container.querySelector('.nav-pinned .node .node-thumb')
    expect(pin).not.toBeNull()
    await waitFor(() =>
      expect(view.container.querySelector('.nav-pinned .node-thumb img')).toHaveAttribute(
        'src',
        'asset:///thumbs/one.webp',
      ),
    )
  })
})
