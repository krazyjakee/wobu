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
