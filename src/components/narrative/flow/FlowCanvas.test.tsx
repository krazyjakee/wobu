import { useState } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useUI } from '../../../store/ui'
import { FlowCanvas } from './FlowCanvas'
import { resetFlowStore, useFlowStore } from './flowStore'
import { councilHearing } from './fixture'
import { connectPort, sceneDiagnostics, type FlowScene } from './model'
import type { LayoutRunner } from './layout'

/**
 * The canvas, in jsdom.
 *
 * Read `src/test/setup.ts` before adding to this file. React Flow draws no
 * edges at all unless a synchronously-firing `ResizeObserver` is installed, so
 * a test asserting on edges passes vacuously without it. The first test below
 * exists to fail loudly if that stub is ever removed.
 *
 * What jsdom cannot answer, and what these tests therefore do not claim:
 * geometry, frame times, dragging, wheel and trackpad behaviour, or anything
 * about the Tauri webview. Those need a person and a real display.
 */

function Harness({
  initial,
  readOnly = false,
  layout,
  onScene,
}: {
  initial: FlowScene
  readOnly?: boolean
  layout?: LayoutRunner
  onScene?: (scene: FlowScene) => void
}) {
  const [scene, setScene] = useState(initial)
  return (
    <FlowCanvas
      scene={scene}
      onChange={(next) => {
        onScene?.(next)
        setScene(next)
      }}
      readOnly={readOnly}
      layout={layout}
    />
  )
}

const nodes = (container: HTMLElement) => container.querySelectorAll('.react-flow__node')
const edges = (container: HTMLElement) => container.querySelectorAll('.react-flow__edge')
const nodeEl = (container: HTMLElement, id: string) =>
  container.querySelector<HTMLElement>(`.react-flow__node[data-id="${id}"]`)!
const announcements = () => screen.getByRole('status', { name: 'Flow canvas announcements' })

beforeEach(() => {
  resetFlowStore()
  useUI.setState({
    narrative: { sceneId: null, beatId: null, lineId: null },
    narrativeReveal: null,
    narrativeFilters: { needsText: false, needsReview: false, outOfDate: false },
  })
})

describe('what the canvas draws', () => {
  it('draws every element and every route between them', () => {
    const { container } = render(<Harness initial={councilHearing()} />)
    // Sixteen boxes and one group frame behind the evidence branch.
    expect(nodes(container)).toHaveLength(17)
    /*
     * The edge count is the assertion that guards the whole suite.
     *
     * Remove the `ResizeObserver` stub from `src/test/setup.ts` and React Flow
     * considers every node unmeasured, draws no edges at all, and this drops to
     * zero — while every other test in the file keeps passing. That is the trap
     * this line exists to spring.
     */
    expect(edges(container)).toHaveLength(19)
  })

  it('shows a beat as one node with its counts, whatever its variants', () => {
    render(<Harness initial={councilHearing()} />)
    // Twelve lines, seven variants, one box, and no line of dialogue anywhere.
    const beat = screen.getByTestId('flow-node-beat.2')
    expect(beat).toHaveTextContent('Present evidence')
    expect(beat).toHaveTextContent('12 lines · 7 variants')
    expect(beat).toHaveTextContent('Kael · Mira')
  })

  it('marks a reconvergence with a count rather than leaving it to the wires', () => {
    render(<Harness initial={councilHearing()} />)
    expect(screen.getByTestId('flow-node-beat.7')).toHaveTextContent('4 routes in')
    expect(screen.getByTestId('flow-node-beat.1')).toHaveTextContent('Scene start')
  })

  it('puts a requirement on the port row that carries it, not on the wire', () => {
    const { container } = render(<Harness initial={councilHearing()} />)
    const choice = screen.getByTestId('flow-node-choice.1')
    expect(choice).toHaveTextContent('Show the logbook')
    expect(choice).toHaveTextContent('requires has_logbook')
    // And not on the wire, where a layout pass would carry it away from the
    // option it gates.
    expect(container.querySelector('.react-flow__edge-textwrapper')?.textContent).not.toMatch(
      /has_logbook/,
    )
  })

  it('lists an outcome’s effects and counts the ones that do not fit', () => {
    render(<Harness initial={councilHearing()} />)
    const outcome = screen.getByTestId('flow-node-outcome.3')
    expect(outcome).toHaveTextContent('support −8')
    expect(outcome).toHaveTextContent('and 1 more')
  })

  it('gives every node an accessible name that says what it is', () => {
    render(<Harness initial={councilHearing()} />)
    expect(
      screen.getByRole('group', { name: 'Beat, The verdict, 4 routes in' }),
    ).toBeInTheDocument()
  })

  it('says how many of the scene’s elements are on the canvas', () => {
    const { container } = render(<Harness initial={councilHearing()} />)
    expect(container).toHaveTextContent('16 of 16 on the canvas')
  })
})

describe('the shared selection', () => {
  it('writes the whole path, and says the click came from Flow', () => {
    const { container } = render(<Harness initial={councilHearing()} />)
    fireEvent.click(nodeEl(container, 'beat.2'))

    // The scene as well as the beat: a beat id on its own would leave the
    // previous scene's id above it and open the wrong scene in Script.
    expect(useUI.getState().narrative).toEqual({
      sceneId: 'scene.council',
      beatId: 'beat.2',
      lineId: null,
    })
    expect(useUI.getState().narrativeReveal?.origin).toBe('flow')
    expect(useFlowStore.getState().selectedId).toBe('beat.2')
  })

  it('selects a choice’s owning beat, because a choice is not a beat', () => {
    const { container } = render(<Harness initial={councilHearing()} />)
    fireEvent.click(nodeEl(container, 'choice.1'))
    expect(useUI.getState().narrative.beatId).toBe('beat.1')
    expect(useFlowStore.getState().selectedId).toBe('choice.1')
  })

  it('honours a reveal raised somewhere else, once', () => {
    render(<Harness initial={councilHearing()} />)
    act(() => {
      useUI.getState().selectNarrative({ sceneId: 'scene.council', beatId: 'beat.5' }, 'diagnostic')
    })
    expect(useFlowStore.getState().selectedId).toBe('beat.5')

    // Honoured once per `seq`: re-rendering must not re-centre.
    act(() => useFlowStore.getState().select('beat.3'))
    act(() => {
      useUI.setState({
        narrativeFilters: { needsText: true, needsReview: false, outOfDate: false },
      })
    })
    expect(useFlowStore.getState().selectedId).toBe('beat.3')
  })

  it('ignores a reveal it raised itself, so a click never bounces the viewport', () => {
    const { container } = render(<Harness initial={councilHearing()} />)
    fireEvent.click(nodeEl(container, 'beat.3'))
    act(() => {
      // A reveal with origin `flow` naming a different node: if the canvas
      // honoured its own echoes it would jump to beat.5 here.
      useUI.getState().selectNarrative({ sceneId: 'scene.council', beatId: 'beat.5' }, 'flow')
    })
    expect(useFlowStore.getState().selectedId).toBe('beat.3')
  })
})

describe('authoring with the keyboard alone', () => {
  it('travels along edges rather than in node-array order', () => {
    const { container } = render(<Harness initial={councilHearing()} />)
    nodeEl(container, 'beat.1').focus()
    fireEvent.keyDown(nodeEl(container, 'beat.1'), { key: 'ArrowDown' })
    expect(useFlowStore.getState().selectedId).toBe('choice.1')
    expect(document.activeElement).toBe(nodeEl(container, 'choice.1'))

    // And back up the way it came.
    fireEvent.keyDown(nodeEl(container, 'choice.1'), { key: 'ArrowUp' })
    expect(useFlowStore.getState().selectedId).toBe('beat.1')
  })

  it('walks a choice’s branches left and right in the order they were authored', () => {
    const { container } = render(<Harness initial={councilHearing()} />)
    fireEvent.keyDown(nodeEl(container, 'beat.2'), { key: 'ArrowRight' })
    expect(useFlowStore.getState().selectedId).toBe('beat.3')
    fireEvent.keyDown(nodeEl(container, 'beat.3'), { key: 'ArrowRight' })
    expect(useFlowStore.getState().selectedId).toBe('beat.4')
  })

  it('connects two nodes with no pointer at all', () => {
    // Handles are not focusable and React Flow will not make them so, so the
    // connection is node to node and the port is chosen for the writer.
    const scene = connectPort(councilHearing(), { elementId: 'beat.4', portId: 'then' }, null)
    let latest: FlowScene = scene
    const { container } = render(<Harness initial={scene} onScene={(next) => (latest = next)} />)

    fireEvent.keyDown(nodeEl(container, 'beat.4'), { key: 'c' })
    expect(useFlowStore.getState().connectFrom).toEqual({ elementId: 'beat.4', portId: 'then' })
    expect(announcements()).toHaveTextContent('Connecting “Then” from Threaten')

    fireEvent.keyDown(nodeEl(container, 'end.1'), { key: 'c' })
    expect(latest.elements.find((e) => e.id === 'beat.4')?.out[0]?.to).toBe('end.1')
    expect(useFlowStore.getState().connectFrom).toBeNull()
  })

  it('cancels a half-made connection on Escape', () => {
    const { container } = render(<Harness initial={councilHearing()} />)
    fireEvent.keyDown(nodeEl(container, 'beat.4'), { key: 'c' })
    fireEvent.keyDown(nodeEl(container, 'beat.4'), { key: 'Escape' })
    expect(useFlowStore.getState().connectFrom).toBeNull()
  })

  it('deletes a node, takes its wires with it, and leaves the selection somewhere real', () => {
    let latest: FlowScene = councilHearing()
    const { container } = render(
      <Harness initial={councilHearing()} onScene={(next) => (latest = next)} />,
    )
    fireEvent.click(nodeEl(container, 'beat.3'))
    fireEvent.keyDown(nodeEl(container, 'beat.3'), { key: 'Delete' })

    expect(latest.elements.some((element) => element.id === 'beat.3')).toBe(false)
    // The route into it is now an explicit question rather than a silent gap.
    expect(sceneDiagnostics(latest).map((d) => d.field)).toContain('choice.1.option.2')
    expect(useUI.getState().narrative.beatId).toBeNull()
  })

  it('disconnects a destination and returns focus from the removed edge to its route', async () => {
    let latest: FlowScene = councilHearing()
    const { container } = render(
      <Harness initial={councilHearing()} onScene={(next) => (latest = next)} />,
    )
    const edge = container.querySelector<HTMLElement>(
      '.react-flow__edge[data-id="outcome.2:then"]',
    )!
    fireEvent.keyDown(edge, { key: 'Delete' })

    expect(latest.elements.find((e) => e.id === 'outcome.2')?.out[0]?.to).toBeNull()
    expect(sceneDiagnostics(latest).map((d) => d.field)).toEqual(['outcome.2.then'])
    await waitFor(() => expect(nodeEl(container, 'outcome.2')).toHaveFocus())
  })
})

describe('creating, collapsing and laying out', () => {
  it('adds an element through the selected node’s spare port', () => {
    const scene = connectPort(councilHearing(), { elementId: 'beat.4', portId: 'then' }, null)
    let latest: FlowScene = scene
    const { container } = render(<Harness initial={scene} onScene={(next) => (latest = next)} />)
    fireEvent.click(nodeEl(container, 'beat.4'))
    fireEvent.click(screen.getByRole('button', { name: 'Add outcome' }))

    expect(latest.elements.find((e) => e.id === 'beat.4')?.out[0]?.to).toBe('outcome.5')
    expect(latest.elements.find((e) => e.id === 'outcome.5')?.kind).toBe('outcome')
  })

  it('collapses a group to one box and takes its contents out of the node array', () => {
    const { container } = render(<Harness initial={councilHearing()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Close all groups' }))

    expect(container.querySelector('.react-flow__node[data-id="beat.5"]')).toBeNull()
    expect(screen.getByTestId('flow-node-group.evidence')).toHaveTextContent(
      '5 elements · 3 edges cross',
    )
    expect(container).toHaveTextContent('12 of 16 on the canvas')
  })

  it('applies a layout result, and drops one that a later request superseded', async () => {
    // elkjs cannot be cancelled, so the only defence is to discard. Two
    // requests are made; the first resolves last and must be ignored.
    const settle: ((value: { positions: Record<string, { x: number; y: number }> }) => void)[] = []
    const layout: LayoutRunner = () => new Promise((resolve) => settle.push(resolve))
    const onPositions = vi.fn()

    render(
      <FlowCanvas
        scene={councilHearing()}
        onChange={() => {}}
        layout={layout}
        onPositionsChange={onPositions}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Auto layout' }))
    // The button stays live while a layout runs, precisely so a second request
    // can supersede the first.
    fireEvent.click(screen.getByRole('button', { name: 'Laying out…' }))
    expect(settle).toHaveLength(2)

    await act(async () => {
      settle[1]!({ positions: { 'beat.1': { x: 10, y: 20 } } })
      settle[0]!({ positions: { 'beat.1': { x: 999, y: 999 } } })
    })

    await waitFor(() => expect(onPositions).toHaveBeenCalledTimes(1))
    expect(onPositions.mock.calls[0]?.[0]['beat.1']).toEqual({ x: 10, y: 20 })
  })

  it('says so, out loud, when a layout fails', async () => {
    const layout: LayoutRunner = () => Promise.reject(new Error('the worker died'))
    render(<FlowCanvas scene={councilHearing()} onChange={() => {}} layout={layout} />)
    fireEvent.click(screen.getByRole('button', { name: 'Auto layout' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('the worker died')
  })
})

describe('filters and read-only', () => {
  it('allows a read-only local layout without attempting a sidecar or source write', async () => {
    const onPositionsChange = vi.fn()
    const onChange = vi.fn()
    render(
      <FlowCanvas
        scene={councilHearing()}
        readOnly
        onChange={onChange}
        onPositionsChange={onPositionsChange}
        layout={() => Promise.resolve({ positions: { 'beat.1': { x: 100, y: 200 } } })}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Auto layout' }))
    await screen.findByRole('button', { name: 'Auto layout' })
    expect(onPositionsChange).not.toHaveBeenCalled()
    expect(onChange).not.toHaveBeenCalled()
  })

  it('mutes the beats a participant is not in, and says they are filtered', () => {
    render(<Harness initial={councilHearing()} />)
    fireEvent.change(screen.getByLabelText('Participant'), { target: { value: 'Orren' } })
    expect(screen.getByTestId('flow-node-beat.6')).not.toHaveClass('is-muted')
    expect(screen.getByTestId('flow-node-beat.5')).toHaveClass('is-muted')
    // Said, not only dimmed.
    expect(screen.getByTestId('flow-node-beat.5')).toHaveTextContent('Filtered out')
  })

  it('honours the workspace’s own status filters rather than inventing a second set', () => {
    render(<Harness initial={councilHearing()} />)
    act(() => useUI.getState().toggleNarrativeFilter('needsText'))
    expect(screen.getByTestId('flow-node-beat.4')).not.toHaveClass('is-muted')
    expect(screen.getByTestId('flow-node-beat.1')).toHaveClass('is-muted')
  })

  it('refuses every structural edit in a read-only project, and says why', () => {
    const onScene = vi.fn()
    const { container } = render(<Harness initial={councilHearing()} readOnly onScene={onScene} />)
    fireEvent.keyDown(nodeEl(container, 'beat.3'), { key: 'Delete' })
    expect(onScene).not.toHaveBeenCalled()
    expect(announcements()).toHaveTextContent('read-only')
    expect(screen.getByRole('button', { name: 'Add beat' })).toBeDisabled()
  })
})
