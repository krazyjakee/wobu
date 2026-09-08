import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { NarrativeLibrary } from './NarrativeLibrary'
import { libraryRows } from './sceneLibrary.fixture'
import { readPreferences } from './sceneLibraryPreferences'

function renderLibrary(over: Partial<Parameters<typeof NarrativeLibrary>[0]> = {}) {
  const onOpen = vi.fn()
  const result = render(
    <NarrativeLibrary
      projectKey="/world"
      rows={libraryRows}
      loading={false}
      readOnly={false}
      navCollapsed={false}
      nameOf={(id) => (id === 'mira' ? 'Mira' : undefined)}
      onOpen={onOpen}
      onCreateScene={vi.fn()}
      {...over}
    />,
  )
  return { ...result, onOpen }
}
beforeEach(() => localStorage.clear())

describe('Scene library', () => {
  it('opens a remembered line at its selected variant and combines filters', () => {
    const { onOpen } = renderLibrary()
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'attack' } })
    fireEvent.change(screen.getByLabelText('Participant'), { target: { value: 'mira' } })
    fireEvent.change(screen.getByLabelText('Policy'), { target: { value: 'locked' } })
    fireEvent.change(screen.getByLabelText('Freshness'), { target: { value: 'out_of_date' } })
    expect(screen.getByRole('status')).toHaveTextContent('1 matching of 1 scenes')
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
  })
  it('persists named views, pins, recent scenes and selected result locally per project', () => {
    const { unmount } = renderLibrary()
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'attack' } })
    fireEvent.change(screen.getByLabelText('View name'), { target: { value: 'Mira lines' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save view' }))
    fireEvent.click(screen.getByRole('button', { name: 'Pin Council hearing' }))
    fireEvent.click(screen.getByRole('button', { name: 'Open Council hearing in Flow' }))
    expect(readPreferences('/world')).toMatchObject({
      selected: 'council',
      pins: ['council'],
      recent: ['council'],
    })
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }))
    fireEvent.click(screen.getByRole('button', { name: 'Mira lines' }))
    expect(screen.getByRole('searchbox')).toHaveValue('attack')
    unmount()
    renderLibrary()
    expect(screen.getByRole('searchbox')).toHaveValue('attack')
    expect(screen.getByRole('button', { name: 'Pin Council hearing' })).toHaveAttribute(
      'aria-pressed',
      'true',
    )
    expect(readPreferences('/other').saved).toEqual([])
  })
  it('bounds mounted rows with pages and preserves paging across remount', () => {
    const rows = Array.from({ length: 1000 }, (_, n) => ({
      summary: {
        id: String(n),
        name: `Scene ${String(n).padStart(4, '0')}`,
        slug: String(n),
        rel: `${n}.yaml`,
      },
    }))
    const { unmount } = renderLibrary({ rows })
    expect(within(screen.getByRole('table')).getAllByRole('row')).toHaveLength(26)
    fireEvent.click(screen.getByRole('button', { name: 'Next page' }))
    expect(screen.getByRole('button', { name: 'Open Scene 0025 in Flow' })).toBeInTheDocument()
    unmount()
    renderLibrary({ rows })
    expect(screen.getByRole('button', { name: 'Open Scene 0025 in Flow' })).toBeInTheDocument()
  })
  it('distinguishes incomplete search, read failures and a filtered empty result', () => {
    renderLibrary({ rows: [{ summary: libraryRows[0]!.summary, error: 'Changed on disk' }] })
    expect(screen.getByRole('status')).toHaveTextContent('results are incomplete')
    expect(screen.getByRole('alert')).toHaveTextContent('Changed on disk')
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'not present' } })
    expect(screen.getByText('No matching scenes. Change or clear the filters.')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Create first scene' })).toBeNull()
  })
  it('allows read-only discovery while refusing scene creation', () => {
    renderLibrary({ readOnly: true })
    expect(screen.getByRole('button', { name: 'New scene' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Open Council hearing in Flow' })).toBeEnabled()
  })
  it('recovers malformed saved preferences', () => {
    localStorage.setItem('wobu:narrative-library:v1:/world', '{broken')
    renderLibrary()
    expect(screen.getByRole('searchbox')).toHaveValue('')
  })
})

describe('Quest discovery', () => {
  const quests = [
    { id: 'attack', name: 'Investigate the attack', scene_ids: ['council'] },
    { id: 'trust', name: 'Earn council trust', scene_ids: ['council'] },
  ]
  it('shows both memberships, finds under either, and restores a named quest view', () => {
    const { unmount } = renderLibrary({ quests })
    const memberships = screen.getByRole('list', { name: 'Quests for Council hearing' })
    expect(within(memberships).getAllByRole('listitem')).toHaveLength(2)
    for (const quest of ['attack', 'trust']) {
      fireEvent.change(screen.getByLabelText('Quest'), { target: { value: quest } })
      expect(screen.getByRole('status')).toHaveTextContent('1 matching of 1 scenes')
      expect(within(screen.getByRole('table')).getAllByRole('row')).toHaveLength(2)
    }
    fireEvent.change(screen.getByLabelText('View name'), { target: { value: 'Trust scenes' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save view' }))
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }))
    unmount()
    renderLibrary({ quests })
    fireEvent.click(screen.getByRole('button', { name: 'Trust scenes' }))
    expect(screen.getByLabelText('Quest')).toHaveValue('trust')
    expect(readPreferences('/world').saved[0]?.view.quest).toBe('trust')
  })
  it('migrates old saved views without discarding their search and selection', () => {
    localStorage.setItem(
      'wobu:narrative-library:v1:/world',
      JSON.stringify({
        view: {
          query: 'attack',
          participant: 'mira',
          policy: '',
          review: '',
          freshness: '',
          missing: false,
          includeDrafts: false,
          sort: 'name',
        },
        saved: [
          {
            name: 'Older view',
            view: {
              query: 'convince',
              participant: '',
              policy: '',
              review: '',
              freshness: '',
              missing: false,
              includeDrafts: false,
              sort: 'name',
            },
          },
        ],
        selected: 'council',
        pins: ['council'],
        page: 0,
        scroll: 0,
      }),
    )
    renderLibrary({ quests })
    expect(screen.getByRole('searchbox')).toHaveValue('attack')
    expect(screen.getByLabelText('Quest')).toHaveValue('')
    expect(readPreferences('/world').selected).toBe('council')
    fireEvent.click(screen.getByRole('button', { name: 'Older view' }))
    expect(screen.getByRole('searchbox')).toHaveValue('convince')
    expect(screen.getByLabelText('Quest')).toHaveValue('')
  })
  it('retains a deleted quest filter and explains why its view is empty', () => {
    const { unmount } = renderLibrary({ quests })
    fireEvent.change(screen.getByLabelText('Quest'), { target: { value: 'trust' } })
    unmount()
    renderLibrary({ quests: [quests[0]!] })
    expect(screen.getByLabelText('Quest')).toHaveValue('trust')
    expect(screen.getByText(/quest selected by this view no longer exists/)).toBeInTheDocument()
    expect(screen.getByRole('status')).toHaveTextContent('0 matching of 1 scenes')
  })
  it('distinguishes unavailable quest source from an unassigned scene', () => {
    renderLibrary({ questsError: 'Unsupported world version' })
    expect(screen.getByLabelText('Quest')).toBeDisabled()
    expect(screen.getByRole('alert')).toHaveTextContent('Quest results are incomplete')
    expect(screen.queryByText('No quests')).not.toBeInTheDocument()
  })
})
