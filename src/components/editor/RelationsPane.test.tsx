import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { WobuNode } from '../../lib/api'
import { resetNodeThumbs } from '../../lib/nodeThumbs'
import { kindDef, kindIndex, node, summary } from '../../test/fixtures'
import { chooseOption, comboboxOptions, filterAndChoose } from '../Combobox.testing'
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

/** Every candidate the target picker is currently offering, list left closed. */
function offeredTargets() {
  const rows = comboboxOptions('Relation target')
  fireEvent.keyDown(screen.getByRole('combobox', { name: 'Relation target' }), { key: 'Escape' })
  return rows
}

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

    // `node_list` is a query, so the candidates arrive a tick after the two
    // controls do. Waiting on the picker becoming usable — rather than on the
    // role control, which is drawn from props and is ready immediately — is
    // what makes this deterministic.
    const target = await screen.findByRole('combobox', { name: 'Relation target' })
    await waitFor(() => expect(target).toBeEnabled())

    expect(offeredTargets()).toEqual(['Human'])

    chooseOption('Relation role', 'Member of')
    expect(offeredTargets()).toEqual(['Guild'])

    chooseOption('Relation role', 'Located in')
    expect(offeredTargets()).toEqual(['Harbour'])
  })

  it('filters the candidates by name and adds the one that was typed for', async () => {
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

    chooseOption('Relation role', 'Located in')
    await waitFor(() =>
      expect(screen.getByRole('combobox', { name: 'Relation target' })).toBeEnabled(),
    )

    expect(filterAndChoose('Relation target', 'harb')).toHaveValue('Harbour')

    fireEvent.click(screen.getByRole('button', { name: 'Add relation' }))
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith(
        'node_link_add',
        expect.objectContaining({ nodeId: 'hero', toId: 'harbour', role: 'located_in' }),
      ),
    )
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

/*
 * The relation row's context menu (#130).
 *
 * Everything on it is already on the row — the jump, the Active/Muted
 * checkbox, Remove. What the menu buys is that the three of them are one
 * gesture apart on a row whose controls are a button, a checkbox, a number
 * field and another button, laid out in four columns.
 */
describe('relation row context menu', () => {
  const related = node({
    id: 'hero',
    kind: 'character',
    name: 'Hero',
    links: [{ toId: 'human', role: 'species_of', weight: 0.6, enabled: true }],
  })

  function openRowMenu(view: ReturnType<typeof draw>, key = 'F10') {
    const row = view.container.querySelector('.relation-row') as HTMLElement
    if (key === 'F10') fireEvent.keyDown(row, { key: 'F10', shiftKey: true })
    else fireEvent.contextMenu(row, { clientX: 20, clientY: 30 })
    return row
  }

  it('offers the row’s own three actions, by keyboard as well as by pointer', async () => {
    const view = draw(related)
    await screen.findByRole('button', { name: /Human/ })

    const row = openRowMenu(view)
    const menu = screen.getByRole('menu', { name: 'Actions for the relation to Human' })
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent?.trim()),
    ).toEqual(['Open Human', 'Mute this influence', 'Remove relation'])

    fireEvent.keyDown(within(menu).getByRole('menuitem', { name: 'Open Human' }), { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
    expect(row).toHaveFocus()
  })

  it('mutes and removes through the same commands as the row’s own controls', async () => {
    const view = draw(related)
    await screen.findByRole('button', { name: /Human/ })

    openRowMenu(view, 'pointer')
    fireEvent.click(screen.getByRole('menuitem', { name: 'Mute this influence' }))
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('node_link_update', {
        nodeId: 'hero',
        toId: 'human',
        role: 'species_of',
        enabled: false,
      }),
    )

    openRowMenu(view, 'pointer')
    fireEvent.click(screen.getByRole('menuitem', { name: 'Remove relation' }))
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('node_link_remove', {
        nodeId: 'hero',
        toId: 'human',
        role: 'species_of',
      }),
    )
  })

  it('says why the menu refuses to write in a read-only project', async () => {
    render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <RelationsPane
          node={related}
          def={kinds.get('character')}
          kinds={kinds}
          readOnly
          onJump={() => {}}
        />
      </QueryClientProvider>,
    )
    await screen.findByRole('button', { name: /Human/ })

    fireEvent.contextMenu(document.querySelector('.relation-row') as HTMLElement, {
      clientX: 20,
      clientY: 30,
    })
    const remove = screen.getByRole('menuitem', { name: 'Remove relation' })
    expect(remove).toHaveAttribute('aria-disabled', 'true')
    fireEvent.focus(remove)
    expect(
      document.getElementById(remove.getAttribute('aria-describedby') ?? ''),
    ).toHaveTextContent(/read-only/)

    fireEvent.click(remove)
    expect(h.invoke).not.toHaveBeenCalledWith('node_link_remove', expect.anything())
    // Jumping is not a write, so it is still offered on a folder nothing can be
    // written to.
    expect(screen.getByRole('menuitem', { name: 'Open Human' })).not.toHaveAttribute(
      'aria-disabled',
    )
  })
})

afterEach(() => resetNodeThumbs())
