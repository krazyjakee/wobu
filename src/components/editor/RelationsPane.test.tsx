import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { WobuNode } from '../../lib/api'
import { resetNodeThumbs } from '../../lib/nodeThumbs'
import { kindDef, kindIndex, node, summary } from '../../test/fixtures'
import { RelationsPane } from './RelationsPane'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const kinds = kindIndex([
  kindDef('species', { layer: 'ancestry' }),
  kindDef('culture', { layer: 'culture' }),
  kindDef('setting', { layer: 'place' }),
  kindDef('character', {
    layer: 'subject',
    defaultLinkRoles: ['species_of', 'member_of', 'located_in'],
  }),
])
const subject = node({ id: 'hero', kind: 'character', name: 'Hero' })

function draw(subjectNode: WobuNode) {
  return render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <RelationsPane
        node={subjectNode}
        def={kinds.get('character')}
        kinds={kinds}
        readOnly={false}
        onJump={() => {}}
      />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  resetNodeThumbs()
  h.invoke.mockReset()
  h.invoke.mockImplementation((command: string) => {
    if (command === 'node_list') {
      return Promise.resolve([
        summary({ id: 'hero', kind: 'character', name: 'Hero' }),
        summary({ id: 'human', kind: 'species', name: 'Human' }),
        summary({ id: 'guild', kind: 'culture', name: 'Guild' }),
        summary({ id: 'harbour', kind: 'setting', name: 'Harbour' }),
      ])
    }
    if (command === 'node_backlinks') return Promise.resolve([])
    return Promise.resolve(null)
  })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('influence relation target picker', () => {
  it('offers only nodes in the layer selected by the first dropdown', async () => {
    render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <RelationsPane
          node={subject}
          def={kinds.get('character')}
          kinds={kinds}
          readOnly={false}
          onJump={() => {}}
        />
      </QueryClientProvider>,
    )

    const role = (await screen.findByLabelText('Relation role')) as HTMLSelectElement
    const target = screen.getByLabelText('Relation target')
    expect(within(target).getByRole('option', { name: 'Human' })).toBeInTheDocument()
    expect(within(target).queryByRole('option', { name: 'Guild' })).not.toBeInTheDocument()

    fireEvent.change(role, { target: { value: 'member_of' } })
    expect(within(target).getByRole('option', { name: 'Guild' })).toBeInTheDocument()
    expect(within(target).queryByRole('option', { name: 'Human' })).not.toBeInTheDocument()

    fireEvent.change(role, { target: { value: 'located_in' } })
    expect(within(target).getByRole('option', { name: 'Harbour' })).toBeInTheDocument()
    expect(within(target).queryByRole('option', { name: 'Guild' })).not.toBeInTheDocument()
  })
})

describe('relation row thumbnails', () => {
  const related = node({
    id: 'hero',
    kind: 'character',
    name: 'Hero',
    parentId: 'harbour',
    links: [
      { toId: 'human', role: 'species_of', weight: 1, enabled: true },
      { toId: 'guild', role: 'member_of', weight: 0.5, enabled: true },
    ],
  })

  beforeEach(() => {
    h.invoke.mockImplementation((command: string) => {
      if (command === 'node_list') {
        return Promise.resolve([
          summary({ id: 'hero', kind: 'character', name: 'Hero' }),
          summary({ id: 'human', kind: 'species', name: 'Human' }),
          summary({ id: 'guild', kind: 'culture', name: 'Guild' }),
          summary({ id: 'harbour', kind: 'setting', name: 'Harbour' }),
        ])
      }
      if (command === 'node_backlinks') {
        return Promise.resolve([
          { fromId: 'guild', toId: 'hero', role: 'related_to', weight: 1, enabled: true },
        ])
      }
      if (command === 'node_thumb_batch') return Promise.resolve({ human: '/thumbs/human.webp' })
      return Promise.resolve(null)
    })
  })

  it('draws the picture of the node on the other end of a relation', async () => {
    const view = draw(related)

    await waitFor(() =>
      expect(view.container.querySelector('.relation-target .node-thumb img')).toHaveAttribute(
        'src',
        'asset:///thumbs/human.webp',
      ),
    )
  })

  it('keeps one slot on every row, picture or not, on both sides of the pane', async () => {
    const view = draw(related)
    const slots = () => [...view.container.querySelectorAll('.relation-row')]

    // Parent and both influences carry their box from the first paint, before
    // any name or picture is known: a slot that appeared only once a picture
    // landed would move every row under it.
    expect(slots().map((row) => row.querySelectorAll('.node-thumb').length)).toEqual([1, 1, 1])

    await waitFor(() => expect(slots()).toHaveLength(4))
    await waitFor(() => expect(view.container.querySelector('.node-thumb img')).not.toBeNull())
    expect(slots().map((row) => row.querySelectorAll('.node-thumb').length)).toEqual([1, 1, 1, 1])
    expect(view.container.querySelectorAll('.node-thumb.is-empty')).toHaveLength(3)
    for (const empty of view.container.querySelectorAll('.node-thumb.is-empty')) {
      expect(empty.querySelector('svg.ic')).not.toBeNull()
    }
  })

  it('asks once for both halves of the pane rather than once per row', async () => {
    draw(related)

    await waitFor(() =>
      expect(h.invoke.mock.calls.some(([command]) => command === 'node_thumb_batch')).toBe(true),
    )
    const batches = h.invoke.mock.calls.filter(([command]) => command === 'node_thumb_batch')
    expect(batches).toHaveLength(1)
    // Parent, both link targets and the backlink source — and `guild`, which is
    // on both sides of the pane, asked about once.
    expect((batches[0]?.[1] as { nodeIds: string[] }).nodeIds.sort()).toEqual([
      'guild',
      'harbour',
      'human',
    ])
  })

  it('leaves the jump button announcing only the entity it opens', async () => {
    const view = draw(related)

    await waitFor(() => expect(view.container.querySelector('.node-thumb img')).not.toBeNull())
    // The picture is decoration inside the button, so it must not become part
    // of the button's name — the row already says the entity in text, and a
    // second mention would give the pane two ways to match the same row.
    expect(screen.getAllByRole('button', { name: /Human/ })).toHaveLength(1)
  })
})

afterEach(() => resetNodeThumbs())
