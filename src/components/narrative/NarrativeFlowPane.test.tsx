import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { useUI } from '../../store/ui'
import { NarrativeFlowPane } from './NarrativeFlowPane'
import { councilHearing } from './flow/fixture'
import { resetFlowStore, useFlowStore } from './flow/flowStore'

beforeEach(() => {
  resetFlowStore()
  useUI.setState({
    narrative: { sceneId: null, beatId: null, lineId: null },
    narrativeReveal: null,
    narrativeFilters: { needsText: false, needsReview: false, outOfDate: false },
  })
})

describe('what the pane says about where its scene came from', () => {
  it('says the canvas is a demonstration, because this build has no narrative source', () => {
    // The one thing the pane must never do is draw a convincing fake scene
    // without saying so.
    render(<NarrativeFlowPane />)
    expect(screen.getByText(/Demonstration data/)).toHaveTextContent(
      /held in memory: edits work, and none of them are saved/,
    )
  })

  it('waits, out loud, while a scene is being read', () => {
    render(<NarrativeFlowPane source={{ kind: 'loading' }} />)
    expect(screen.getByText(/Reading the scene/)).toHaveAttribute('aria-busy', 'true')
  })

  it('reports a failed read as a failure rather than as an empty scene', () => {
    render(<NarrativeFlowPane source={{ kind: 'error', message: 'the file is not readable' }} />)
    expect(screen.getByRole('alert')).toHaveTextContent('the file is not readable')
  })

  it('offers to fill a genuinely empty scene, and refuses to in a read-only folder', () => {
    const empty = { ...councilHearing(), elements: [], entryId: null }
    const { rerender } = render(<NarrativeFlowPane source={{ kind: 'ready', scene: empty }} />)
    expect(screen.getByRole('button', { name: /Ashfall example/ })).toBeInTheDocument()

    rerender(<NarrativeFlowPane source={{ kind: 'ready', scene: empty }} readOnly />)
    expect(screen.queryByRole('button', { name: /Ashfall example/ })).toBeNull()
    expect(screen.getByText(/read-only/)).toBeInTheDocument()
  })
})

describe('the outline list, as a full alternative to the canvas', () => {
  const openOutline = () => {
    render(<NarrativeFlowPane />)
    fireEvent.click(screen.getByRole('button', { name: 'Outline list' }))
  }

  it('lists the scene in reading order from its start', () => {
    openOutline()
    // The element rows only: each one nests a list of its own ways out.
    const rows = screen
      .getByLabelText('Council hearing outline')
      .querySelectorAll(':scope > .nrt-outline-row')
    expect(rows[0]).toHaveTextContent('Arrival at the hearing')
    expect(rows[1]).toHaveTextContent('How do you open?')
    // A beat is one row with its counts, exactly as it is one node.
    expect(rows[2]).toHaveTextContent('12 lines · 7 variants')
  })

  it('offers the canvas’s operations as ordinary controls', () => {
    openOutline()
    const destination = screen.getByLabelText('The council reads the logbook — Then leads to')
    expect(destination).toHaveValue('condition.1')

    // Disconnecting is choosing "Nothing yet", and it leaves the hole visible.
    fireEvent.change(destination, { target: { value: '' } })
    expect(screen.getByLabelText('The council reads the logbook — Then leads to')).toHaveValue('')
    expect(screen.getAllByText('no destination').length).toBeGreaterThan(0)
  })

  it('shares one selection with the canvas and with the rest of the workspace', () => {
    openOutline()
    fireEvent.click(screen.getByRole('button', { name: /Present evidence/ }))
    expect(useFlowStore.getState().selectedId).toBe('beat.2')
    expect(useUI.getState().narrative).toEqual({
      sceneId: 'scene.council',
      beatId: 'beat.2',
      lineId: null,
    })
  })

  it('marks an element the start cannot reach', () => {
    openOutline()
    fireEvent.change(screen.getByLabelText('Do you accept the ruling? — Walk out leads to'), {
      target: { value: '' },
    })
    expect(screen.getByText('Not reachable from the start')).toBeInTheDocument()
  })
})

describe('undo', () => {
  it('takes back a structural edit, and puts it back again', () => {
    render(<NarrativeFlowPane />)
    fireEvent.click(screen.getByRole('button', { name: 'Outline list' }))
    const undo = screen.getByRole('button', { name: 'Undo' })
    expect(undo).toBeDisabled()

    fireEvent.change(screen.getByLabelText('The council reads the logbook — Then leads to'), {
      target: { value: 'end.1' },
    })
    expect(undo).toBeEnabled()

    fireEvent.click(undo)
    expect(screen.getByLabelText('The council reads the logbook — Then leads to')).toHaveValue(
      'condition.1',
    )

    fireEvent.click(screen.getByRole('button', { name: 'Redo' }))
    expect(screen.getByLabelText('The council reads the logbook — Then leads to')).toHaveValue(
      'end.1',
    )
  })
})
