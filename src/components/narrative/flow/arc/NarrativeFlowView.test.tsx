import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { useUI } from '../../../../store/ui'
import { resetFlowStore, useFlowStore } from '../flowStore'
import { ARC_STORE } from './arcStore'
import { NarrativeFlowView } from './NarrativeFlowView'

/**
 * Going into a scene and coming back out.
 *
 * The tests that matter here are the ones about what *survives* the round trip,
 * because that is the only part a person cannot check by looking: a canvas that
 * loses the writer's place, their collapsed quests or their unsaved edit looks
 * exactly like one that does not until it happens to you.
 *
 * jsdom cannot pan, so the viewport half of the promise is asserted in
 * `ArcFlow.test.tsx` against the value the canvas itself stores. Everything
 * else is driven end to end here.
 */

const level = () => document.querySelector<HTMLElement>('.nrt-levels, .nrt-pane.nrt-flow')!
const nodeEl = (id: string) =>
  document.querySelector<HTMLElement>(`.react-flow__node[data-id="${id}"]`)!
const enterCouncil = () => fireEvent.doubleClick(nodeEl('scene.council'))
const escape = () => fireEvent.keyDown(level(), { key: 'Escape' })

beforeEach(() => {
  resetFlowStore()
  resetFlowStore(ARC_STORE)
  useUI.setState({
    narrative: { sceneId: null, beatId: null, lineId: null },
    narrativeReveal: null,
    narrativeFilters: { needsText: false, needsReview: false, outOfDate: false },
  })
})

describe('what the view says about where its arc came from', () => {
  it('opens on the arc, and says the scenes are a demonstration', () => {
    render(<NarrativeFlowView />)
    expect(screen.getByRole('navigation', { name: 'Flow level' })).toHaveTextContent(
      'The beacon inquiry',
    )
    expect(screen.getByText(/Demonstration data/)).toHaveTextContent(
      /held in memory: edits work, and none of them are saved/,
    )
  })

  it('says separately that the quests are not read from the project', () => {
    // The demonstration arc remains explicit even though a project's own arc
    // now groups by the quests in its World document.
    render(<NarrativeFlowView />)
    expect(document.getElementById('nrt-quests-unavailable')).toHaveTextContent(
      /Grouping uses demonstration quests, not the ones in this project’s World document/,
    )
    // And the limits of the grouping itself, which hold either way.
    expect(document.getElementById('nrt-quests-unavailable')).toHaveTextContent(
      /the stage each quest starts in/,
    )
  })

  it('waits, out loud, while an arc is being read', () => {
    render(<NarrativeFlowView source={{ kind: 'loading' }} />)
    expect(screen.getByText(/Reading the arc/)).toHaveAttribute('aria-busy', 'true')
  })

  it('reports a failed read as a failure rather than as an empty arc', () => {
    render(<NarrativeFlowView source={{ kind: 'error', message: 'the folder is gone' }} />)
    expect(screen.getByRole('alert')).toHaveTextContent('the folder is gone')
  })
})

describe('going in and coming back', () => {
  it('enters a scene on a double-click and shows both levels in the breadcrumb', () => {
    render(<NarrativeFlowView />)
    enterCouncil()

    const crumbs = screen.getByRole('navigation', { name: 'Flow level' })
    expect(crumbs).toHaveTextContent('The beacon inquiry')
    expect(crumbs).toHaveTextContent('Council hearing')
    // The scene's own canvas, not the arc's: a beat, which the arc never draws.
    expect(screen.getByTestId('flow-node-beat.2')).toHaveTextContent('Present evidence')
    expect(useUI.getState().narrative.sceneId).toBe('scene.council')
  })

  it('comes back on Escape, and on the breadcrumb', () => {
    render(<NarrativeFlowView />)
    enterCouncil()
    escape()
    expect(screen.getByTestId('flow-node-scene.council')).toBeInTheDocument()

    enterCouncil()
    fireEvent.click(screen.getByRole('button', { name: /The beacon inquiry/ }))
    expect(screen.getByTestId('flow-node-scene.council')).toBeInTheDocument()
  })

  it('leaves Escape to the canvas while a connection is half made', () => {
    // The more local meaning wins: a writer mid-connect is asking to stop
    // connecting, not to leave the scene they are working in.
    render(<NarrativeFlowView />)
    enterCouncil()
    fireEvent.keyDown(nodeEl('beat.4'), { key: 'c' })
    expect(useFlowStore.getState().connectFrom).not.toBeNull()

    fireEvent.keyDown(nodeEl('beat.4'), { key: 'Escape' })
    expect(useFlowStore.getState().connectFrom).toBeNull()
    // Still in the scene.
    expect(screen.getByTestId('flow-node-beat.4')).toBeInTheDocument()
  })

  it('will not enter a dangling destination, because there is no scene behind it', () => {
    render(<NarrativeFlowView />)
    fireEvent.doubleClick(nodeEl('scene.beacon:link.harbour:unresolved'))
    expect(screen.getByTestId('flow-node-scene.beacon')).toBeInTheDocument()
  })
})

describe('what survives the round trip', () => {
  it('keeps the arc’s selection', () => {
    render(<NarrativeFlowView />)
    fireEvent.click(nodeEl('scene.ruins'))
    expect(ARC_STORE.getState().selectedId).toBe('scene.ruins')

    enterCouncil()
    escape()
    expect(ARC_STORE.getState().selectedId).toBe('scene.ruins')
  })

  it('keeps the quests the writer opened and closed', () => {
    render(<NarrativeFlowView />)
    fireEvent.click(screen.getByRole('button', { name: 'Close all groups' }))
    expect(ARC_STORE.getState().closedGroups).toEqual(['quest.beacon', 'quest.aftermath'])

    // In through a diagnostic rather than a box, because with the quests closed
    // there is no scene box left to double-click — which is the point.
    fireEvent.click(screen.getAllByRole('button', { name: 'Open the scene' })[0]!)
    escape()
    // Redrawn from the same closed set, not re-defaulted to open.
    expect(ARC_STORE.getState().closedGroups).toEqual(['quest.beacon', 'quest.aftermath'])
    expect(screen.getByTestId('flow-node-quest.beacon')).toBeInTheDocument()
  })

  it('keeps an unsaved edit to the arc', () => {
    render(<NarrativeFlowView />)
    fireEvent.click(nodeEl('scene.ruins'))
    fireEvent.click(screen.getByRole('button', { name: 'Add scene' }))
    expect(screen.getByTestId('flow-node-scene.1')).toBeInTheDocument()

    enterCouncil()
    escape()
    expect(screen.getByTestId('flow-node-scene.1')).toBeInTheDocument()
  })

  it('keeps an unsaved edit to a scene, so re-entering is not a fresh copy', () => {
    render(<NarrativeFlowView />)
    enterCouncil()
    fireEvent.click(screen.getByRole('button', { name: 'Outline list' }))
    fireEvent.change(screen.getByLabelText('The council reads the logbook — Then leads to'), {
      target: { value: '' },
    })

    escape()
    enterCouncil()
    fireEvent.click(screen.getByRole('button', { name: 'Outline list' }))
    expect(screen.getByLabelText('The council reads the logbook — Then leads to')).toHaveValue('')
  })
})

describe('following a diagnostic down a level', () => {
  it('opens the scene at the beat that authored the destination', () => {
    render(<NarrativeFlowView />)
    // The first finding is the long road's unfinished exit, authored on beat.2.
    fireEvent.click(screen.getAllByRole('button', { name: 'Open the scene' })[0]!)

    expect(screen.getByRole('navigation', { name: 'Flow level' })).toHaveTextContent(
      'The long road out',
    )
    expect(useUI.getState().narrative).toEqual({
      sceneId: 'scene.road',
      beatId: 'beat.2',
      lineId: null,
    })
    // And the scene canvas honoured the reveal, so the writer lands on it.
    expect(useFlowStore.getState().selectedId).toBe('beat.2')
  })
})
