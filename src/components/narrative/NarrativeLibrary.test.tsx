import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { LibraryQuery } from '../../lib/api/narrativeLibrary'
import { NarrativeLibrary } from './NarrativeLibrary'
import { libraryPage, libraryRow } from './sceneLibrary.fixture'
import { emptyPreferences, readPreferences, writePreferences } from './sceneLibraryPreferences'
const query = vi.hoisted(() => vi.fn())
vi.mock('../../lib/queries/narrativeLibrary', () => ({ useNarrativeLibraryQuery: query }))
function mount(over: Partial<Parameters<typeof NarrativeLibrary>[0]> = {}) {
  const onOpen = vi.fn()
  return {
    ...render(
      <NarrativeLibrary
        projectKey="/world"
        readOnly={false}
        navCollapsed={false}
        nameOf={(id) => (id === 'mira' ? 'Mira' : undefined)}
        onOpen={onOpen}
        onCreateScene={vi.fn()}
        {...over}
      />,
    ),
    onOpen,
  }
}
beforeEach(() => {
  localStorage.clear()
  query.mockReset()
  query.mockReturnValue({ data: libraryPage, isFetching: false, refetch: vi.fn() })
})
describe('Bounded scene discovery', () => {
  it('sends combined canonical filters and reveals the selected stable line identity', () => {
    const { onOpen } = mount()
    const table = screen.getByRole('table')
    for (const label of ['Act', 'Arc', 'Tags'])
      expect(within(table).getByRole('columnheader', { name: label })).toBeInTheDocument()
    const cells = within(within(table).getAllByRole('row')[1]!).getAllByRole('cell')
    expect(cells.slice(0, 3).map((cell) => cell.textContent)).toEqual([
      'Arrival',
      'Inquiry',
      'Politics',
    ])
    for (const [label, value] of [
      ['Quest', 'trust'],
      ['Act', 'arrival'],
      ['Arc', 'inquiry'],
      ['Tag', 'politics'],
      ['Participant', 'mira'],
      ['Policy', 'locked'],
      ['Freshness', 'out_of_date'],
    ])
      fireEvent.change(screen.getByLabelText(label!), { target: { value } })
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'attack' } })
    expect(query).toHaveBeenCalledWith(
      '/world',
      expect.objectContaining({
        query: 'attack',
        quest: 'trust',
        act: 'arrival',
        arc: 'inquiry',
        tag: 'politics',
        participant: 'mira',
        policy: 'locked',
        freshness: 'out_of_date',
        offset: 0,
        limit: 25,
        revision: null,
      }),
    )
    fireEvent.change(screen.getByLabelText('Matching passage in Council hearing'), {
      target: { value: '1' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Open Council hearing in Script' }))
    expect(onOpen).toHaveBeenCalledWith(
      expect.objectContaining({
        sceneId: 'council',
        beatId: 'evidence',
        lineId: 'line',
        variantId: 'low',
      }),
      'script',
      'low',
    )
    expect(
      within(screen.getByRole('list', { name: 'Quests for Council hearing' })).getAllByRole(
        'listitem',
      ),
    ).toHaveLength(2)
  })
  it('persists combined saved views, pins, recent and selection across reopen', () => {
    const { unmount } = mount()
    fireEvent.change(screen.getByLabelText('Quest'), { target: { value: 'trust' } })
    fireEvent.change(screen.getByLabelText('Act'), { target: { value: 'arrival' } })
    fireEvent.change(screen.getByLabelText('View name'), { target: { value: 'Political arrival' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save view' }))
    fireEvent.click(screen.getByRole('button', { name: 'Pin Council hearing' }))
    fireEvent.click(screen.getByRole('button', { name: 'Open Council hearing in Flow' }))
    unmount()
    mount()
    expect(readPreferences('/world')).toMatchObject({
      selected: 'council',
      pins: ['council'],
      recent: ['council'],
    })
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }))
    fireEvent.click(screen.getByRole('button', { name: 'Political arrival' }))
    expect(screen.getByLabelText('Act')).toHaveValue('arrival')
    expect(screen.getByLabelText('Quest')).toHaveValue('trust')
    expect(readPreferences('/other').saved).toEqual([])
  })
  it('restores saved scroll after asynchronous rows arrive and preserves unavailable filters', () => {
    writePreferences('/world', {
      ...emptyPreferences(),
      scroll: 240,
      view: { ...emptyPreferences().view, participant: 'removed-character' },
    })
    query.mockReturnValue({ data: undefined, isFetching: true })
    const { container, rerender } = mount()
    const scroller = container.querySelector('.nsl-table-scroll')!
    expect(scroller.scrollTop).toBe(0)
    expect(screen.getByLabelText('Participant')).toHaveValue('removed-character')
    query.mockReturnValue({ data: libraryPage, isFetching: false })
    rerender(
      <NarrativeLibrary
        projectKey="/world"
        readOnly={false}
        navCollapsed={false}
        nameOf={() => undefined}
        onOpen={vi.fn()}
        onCreateScene={vi.fn()}
      />,
    )
    expect(scroller.scrollTop).toBe(240)
    scroller.scrollTop = 320
    fireEvent.scroll(scroller)
    expect(readPreferences('/world').scroll).toBe(320)
    expect(screen.getByRole('button', { name: 'Open Council hearing in Script' })).toHaveAttribute(
      'data-library-scene',
      'council',
    )
  })
  it('clears pending restored scroll when restarting an initially failed query', () => {
    writePreferences('/world', { ...emptyPreferences(), scroll: 240 })
    const refetch = vi.fn()
    query.mockReturnValue({
      data: undefined,
      error: new Error('Scene library changed'),
      isFetching: false,
      refetch,
    })
    const { container, rerender } = mount()
    const scroller = container.querySelector('.nsl-table-scroll')!
    fireEvent.click(screen.getByRole('button', { name: 'Restart results' }))
    expect(refetch).toHaveBeenCalledOnce()
    expect(readPreferences('/world').scroll).toBe(0)
    query.mockReturnValue({ data: libraryPage, isFetching: false, refetch })
    rerender(
      <NarrativeLibrary
        projectKey="/world"
        readOnly={false}
        navCollapsed={false}
        nameOf={() => undefined}
        onOpen={vi.fn()}
        onCreateScene={vi.fn()}
      />,
    )
    expect(scroller.scrollTop).toBe(0)
    expect(screen.getByRole('button', { name: 'Open Council hearing in Script' })).toBeEnabled()
  })
  it('retains page revision, bounds mounted rows, and never silently retries a stale page', () => {
    let stale = false
    query.mockImplementation((_project: string, q: LibraryQuery) => ({
      data: stale
        ? undefined
        : {
            ...libraryPage,
            total: 1000,
            sceneCount: 1000,
            rows: Array.from({ length: 25 }, (_, i) => ({
              ...libraryRow,
              summary: {
                ...libraryRow.summary,
                id: String(q.offset + i),
                name: `Scene ${q.offset + i}`,
              },
            })),
          },
      error: stale ? new Error('Scene library changed') : undefined,
      isFetching: false,
      refetch: vi.fn(),
    }))
    const { unmount } = mount()
    expect(within(screen.getByRole('table')).getAllByRole('row')).toHaveLength(26)
    fireEvent.click(screen.getByRole('button', { name: 'Next page' }))
    expect(query).toHaveBeenCalledWith(
      '/world',
      expect.objectContaining({ offset: 25, revision: libraryPage.revision }),
    )
    unmount()
    stale = true
    mount()
    expect(screen.getByRole('alert')).toHaveTextContent('Scene library changed')
    expect(screen.getByRole('button', { name: 'Next page' })).toBeDisabled()
    expect(readPreferences('/world').page).toBe(1)
    fireEvent.click(screen.getByRole('button', { name: 'Restart results' }))
    expect(query).toHaveBeenCalledWith(
      '/world',
      expect.objectContaining({ offset: 0, revision: null }),
    )
  })
  it('surfaces unreadable source with repair and explicitly pages errors', () => {
    const onRepair = vi.fn()
    query.mockReturnValue({
      data: {
        ...libraryPage,
        unreadable: [{ rel: 'narrative/scenes/broken.yaml', reason: 'ambiguous ID' }],
        unreadableTotal: 30,
        unreadableNextOffset: 25,
      },
      isFetching: false,
    })
    mount({ onRepair })
    fireEvent.click(screen.getByRole('button', { name: 'Repair source' }))
    expect(onRepair).toHaveBeenCalledWith('narrative/scenes/broken.yaml')
    fireEvent.click(screen.getByRole('button', { name: 'More source errors' }))
    expect(query).toHaveBeenCalledWith(
      '/world',
      expect.objectContaining({ unreadableOffset: 25, revision: libraryPage.revision }),
    )
  })
  it('discloses truncated passages and read-only discovery', () => {
    query.mockReturnValue({
      data: { ...libraryPage, rows: [{ ...libraryRow, matchCount: 22 }] },
      isFetching: false,
    })
    mount({ readOnly: true })
    expect(screen.getByRole('button', { name: 'New scene' })).toBeDisabled()
    expect(screen.getByText(/Showing 2 of 22 passages/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Open Council hearing in Flow' })).toBeEnabled()
  })
})
