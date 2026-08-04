import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { NodeSummary } from '../../lib/api'
import { useFavourites } from '../../lib/favourites'
import { buildGroups, indexNodes } from '../../lib/tree'
import { useUI } from '../../store/ui'
import { kindDef, kindIndex, summary } from '../../test/fixtures'
import { Navigator } from './Navigator'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))

const kinds = kindIndex([
  kindDef('character', { label: 'Character', plural: 'Characters' }),
  kindDef('culture', { label: 'Culture', plural: 'Cultures' }),
])

const ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'

function world(count: number): NodeSummary[] {
  return Array.from({ length: count }, (_, index) =>
    summary({ id: `n-${index}`, name: `${ALPHABET[index % 26]}${index}`, kind: 'character' }),
  )
}

function draw(nodes: NodeSummary[]) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <Navigator
        nodes={nodes}
        byId={indexNodes(nodes)}
        pinned={[]}
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

const rowNames = (view: ReturnType<typeof draw>): string[] =>
  [...view.container.querySelectorAll('.nav-virtual-window .node .nm')].map(
    (node) => node.textContent ?? '',
  )

const bandLabels = (view: ReturnType<typeof draw>): string[] =>
  [...view.container.querySelectorAll('.band > .group-h')].map((band) =>
    (band.firstElementChild?.nextSibling?.textContent ?? '').trim(),
  )

beforeEach(() => {
  h.invoke.mockReset()
  h.invoke.mockImplementation(() => Promise.resolve(null))
  useUI.setState({
    filter: '',
    selectedId: null,
    recentIds: [],
    collapsedNodes: {},
    closedGroups: {},
    bands: {},
  })
  useFavourites.setState({ byProject: {} })
})

describe('navigator structure at scale', () => {
  it('replaces a long group with an index the reader can take in at once', () => {
    const view = draw(world(300))

    // Nothing is hidden — every entity is one click away — but the navigator
    // opens on a list short enough to read rather than on three hundred rows.
    expect(rowNames(view)).toEqual([])
    const labels = bandLabels(view)
    expect(labels.length).toBeGreaterThan(1)
    expect(labels.length).toBeLessThanOrEqual(12)

    const first = view.container.querySelector('.band > .group-h') as HTMLElement
    fireEvent.click(first)
    const opened = rowNames(view)
    expect(opened.length).toBeGreaterThan(0)
    expect(opened.length).toBeLessThan(300)
    expect(opened[0]).toBe('A0')
  })

  it('leaves a group small enough to scan exactly as it was', () => {
    const view = draw(world(12))
    expect(bandLabels(view)).toEqual([])
    expect(rowNames(view)).toHaveLength(12)
  })

  it('opens the heading the selection is filed under, wherever the jump came from', () => {
    // The palette, a breadcrumb and a backlink all arrive as a bare `select`,
    // and a row that is not in the list cannot be scrolled to.
    const view = draw(world(300))
    expect(rowNames(view)).toEqual([])

    act(() => useUI.getState().select('n-260'))

    expect(rowNames(view)).toContain('A260')
    expect(screen.getByRole('button', { name: 'A260' })).toHaveAttribute('aria-current', 'true')
  })

  it('says how big the world is, and how much a filter left of it', () => {
    const view = draw(world(300))
    expect(view.container.querySelector('.nav-count')?.textContent).toBe('300 entities')

    act(() => useUI.getState().setFilter('A26'))
    expect(view.container.querySelector('.nav-count')?.textContent).toBe('2 of 300 shown')
  })

  it('collapses and expands the whole tree from one control', () => {
    const view = draw(world(12))
    const button = () => screen.getByRole('button', { name: /^(Collapse|Expand) all$/ })

    fireEvent.click(button())
    expect(rowNames(view)).toEqual([])
    expect(button()).toHaveTextContent('Expand all')

    fireEvent.click(button())
    expect(rowNames(view)).toHaveLength(12)
  })
})

/**
 * The star on a row.
 *
 * Found by class rather than by name: it is deliberately `aria-hidden` and has
 * no title of its own — the row is the button, a nested one would be invalid,
 * and its tooltip is hover-only with the row's context menu as the keyboard
 * route. See the comment beside it in `Navigator.tsx`.
 */
function starOn(name: string): HTMLElement {
  const row = screen.getAllByRole('button', { name })[0] as HTMLElement
  const star = row.querySelector('.fav.star')
  if (!star) throw new Error(`no star on the row for ${name}`)
  return star as HTMLElement
}

describe('navigator favourites and recents', () => {
  it('keeps a starred node at the top and lets the star take it back off', () => {
    const view = draw(world(12))
    fireEvent.click(starOn('C2'))

    expect(bandLabels(view)).toEqual(['Favourites'])
    // The shortcut and the row it points at are both on screen, and the
    // shortcut is the one that comes first.
    expect(rowNames(view).slice(0, 2)).toEqual(['C2', 'A0'])
    expect(view.container.querySelectorAll('.node-shortcut')).toHaveLength(1)
    expect(useFavourites.getState().byProject['/project']).toEqual(['n-2'])

    fireEvent.click(starOn('C2'))
    expect(bandLabels(view)).toEqual([])
    expect(useFavourites.getState().byProject['/project']).toEqual([])
  })

  it('offers the same switch in the row menu, the way in from the keyboard', () => {
    draw(world(12))
    const row = screen.getByRole('button', { name: 'C2' })
    fireEvent.contextMenu(row, { clientX: 10, clientY: 10 })

    fireEvent.click(screen.getByRole('menuitem', { name: 'Add to favourites' }))
    expect(useFavourites.getState().byProject['/project']).toEqual(['n-2'])

    fireEvent.contextMenu(screen.getAllByRole('button', { name: 'C2' })[0] as HTMLElement, {
      clientX: 10,
      clientY: 10,
    })
    expect(screen.getByRole('menuitem', { name: 'Remove from favourites' })).toBeInTheDocument()
  })

  it('sorts favourites by name rather than by when they were starred', () => {
    // Starring is not a ranking. A list that reshuffles under the reader is one
    // they have to re-read every time they add to it.
    const view = draw(world(12))
    for (const name of ['C2', 'A0', 'B1']) fireEvent.click(starOn(name))
    expect(rowNames(view).slice(0, 3)).toEqual(['A0', 'B1', 'C2'])
  })

  it('remembers where the reader has been, and leaves out where they are', () => {
    const view = draw(world(40))

    // The node you are looking at is not somewhere you might want to get back
    // to, so the section stays empty until the reader moves on.
    act(() => useUI.getState().select('n-3'))
    expect(bandLabels(view)).toEqual([])

    act(() => useUI.getState().select('n-5'))
    expect(bandLabels(view)).toEqual(['Recent'])
    expect(rowNames(view)[0]).toBe('D3')

    act(() => useUI.getState().select('n-7'))
    expect(rowNames(view).slice(0, 2)).toEqual(['F5', 'D3'])
  })

  it('stays out of a project small enough to hold in one screen', () => {
    // Twelve rows, four of them repeated at the top, is not a shortcut — it is
    // the same entity drawn twice in a list nobody was lost in.
    const view = draw(world(12))
    act(() => useUI.getState().select('n-3'))
    act(() => useUI.getState().select('n-5'))
    expect(bandLabels(view)).toEqual([])
    expect(rowNames(view)).toHaveLength(12)
  })
})
