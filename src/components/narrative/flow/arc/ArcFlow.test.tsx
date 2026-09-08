import { useState } from 'react'
import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useUI } from '../../../../store/ui'
import { resetFlowStore } from '../flowStore'
import { addElement } from '../model'
import { ArcFlow } from './ArcFlow'
import { ARC_STORE } from './arcStore'
import { beaconArc } from './fixture'
import { scenesOf, type FlowArc } from './model'

/**
 * The arc canvas, in jsdom.
 *
 * Read `src/test/setup.ts` before adding to this file. React Flow draws no
 * edges at all unless a synchronously-firing `ResizeObserver` is installed, so
 * a test asserting on edges passes vacuously without it. The first test below
 * counts real edges, including the two dangling ones, and drops to zero if that
 * stub is ever removed.
 *
 * What jsdom cannot answer, and what these tests therefore do not claim:
 * geometry, frame times, dragging, panning, wheel behaviour, or anything about
 * the Tauri webview. Those need a person and a real display.
 */

function Harness({
  initial,
  readOnly = false,
  onEnter = () => {},
  onArc,
}: {
  initial: FlowArc
  readOnly?: boolean
  onEnter?: (sceneId: string, beatId?: string | null) => void
  onArc?: (arc: FlowArc) => void
}) {
  const [arc, setArc] = useState(initial)
  return (
    <ArcFlow
      arc={arc}
      onChange={(next) => {
        onArc?.(next)
        setArc(next)
      }}
      onEnter={onEnter}
      readOnly={readOnly}
    />
  )
}

const nodes = (container: HTMLElement) => container.querySelectorAll('.react-flow__node')
const edges = (container: HTMLElement) => container.querySelectorAll('.react-flow__edge')
const nodeEl = (container: HTMLElement, id: string) =>
  container.querySelector<HTMLElement>(`.react-flow__node[data-id="${id}"]`)!

beforeEach(() => {
  resetFlowStore(ARC_STORE)
  useUI.setState({
    narrative: { sceneId: null, beatId: null, lineId: null },
    narrativeReveal: null,
    narrativeFilters: { needsText: false, needsReview: false, outOfDate: false },
  })
})

describe('what the arc draws', () => {
  it('draws one box per scene, one per dangling destination, and every wire', () => {
    const { container } = render(<Harness initial={beaconArc()} />)
    // Four scenes, two boxes for the two unresolved destinations, and a frame
    // behind each of the two quests.
    expect(nodes(container)).toHaveLength(8)
    /*
     * The edge count is the assertion that guards the suite.
     *
     * Two authored transitions that resolve, and two that do not — and the two
     * that do not are the point: an unresolved cross-scene destination is a
     * wire ending in a named box, not an edge that is quietly absent.
     */
    expect(edges(container)).toHaveLength(4)
  })

  it('summarises a scene as counts, and never as its contents', () => {
    render(<Harness initial={beaconArc()} />)
    const beacon = screen.getByTestId('flow-node-scene.beacon')
    expect(beacon).toHaveTextContent('The beacon chamber')
    expect(beacon).toHaveTextContent('2 beats')
    expect(beacon).toHaveTextContent('1 needs text')
    expect(beacon).toHaveTextContent('1 out of date')
    expect(beacon).toHaveTextContent('Kael · Orren')
    // The beats' own titles stay one level down.
    expect(beacon).not.toHaveTextContent('Orren reads the array')
  })

  it('names the box a wire with no destination ends in', () => {
    render(<Harness initial={beaconArc()} />)
    expect(screen.getByTestId('flow-node-scene.road:link.back:unresolved')).toHaveTextContent(
      'No destination chosen',
    )
    expect(screen.getByTestId('flow-node-scene.beacon:link.harbour:unresolved')).toHaveTextContent(
      'scene.harbour is not in this arc',
    )
  })

  it('gives a scene an accessible name carrying the counts a badge only draws', () => {
    render(<Harness initial={beaconArc()} />)
    expect(
      screen.getByRole('group', {
        name: 'Scene, The beacon chamber, 2 beats, 1 needing text, 1 out of date',
      }),
    ).toBeInTheDocument()
  })

  it('lists each finding with a way to open the scene that owns it', () => {
    const onEnter = vi.fn()
    render(<Harness initial={beaconArc()} onEnter={onEnter} />)
    const findings = screen.getByRole('list', { name: 'Arc diagnostics' })
    expect(findings.querySelectorAll(':scope > li')).toHaveLength(3)

    fireEvent.click(screen.getAllByRole('button', { name: 'Open the scene' })[0]!)
    // Not the scene alone: the beat that authored the destination, which is
    // the field a writer has to change.
    expect(onEnter).toHaveBeenCalledWith('scene.road', 'beat.2')
  })
})

describe('grouping, collapse and filters', () => {
  it('groups by quest, and folds a quest into one box that counts its crossings', () => {
    const { container } = render(<Harness initial={beaconArc()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Close all groups' }))

    expect(container.querySelector('.react-flow__node[data-id="scene.road"]')).toBeNull()
    expect(screen.getByTestId('flow-node-quest.beacon')).toHaveTextContent('3 elements')
    // The dangling boxes go with the scenes that owned them: collapse is the
    // writer's own choice, and the group box's counts are the signal there.
    expect(container.querySelector('[data-id="scene.road:link.back:unresolved"]')).toBeNull()
  })

  it('regroups by quest state without touching the model', () => {
    let latest: FlowArc | null = null
    render(<Harness initial={beaconArc()} onArc={(arc) => (latest = arc)} />)
    fireEvent.change(screen.getByLabelText('Group'), { target: { value: 'questState' } })
    fireEvent.click(screen.getByRole('button', { name: 'Close all groups' }))

    expect(screen.getByTestId('flow-node-state:Investigating')).toHaveTextContent('3 elements')
    // Changing a grouping control is not an edit, so nothing was reported.
    expect(latest).toBeNull()
  })

  it('folds the same quest in the outline, which is the keyboard alternative', () => {
    // #151 asks for a full alternative rather than a reduced one, and a reader
    // who cannot use a plane is the reader who most needs an arc folded into
    // its quests. Same control, same containers, same closed set.
    render(<Harness initial={beaconArc()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Outline list' }))
    const rows = () =>
      screen
        .getByLabelText('The beacon inquiry outline')
        .querySelectorAll(':scope > .nrt-outline-row')
    expect(rows()).toHaveLength(4)

    fireEvent.click(screen.getByRole('button', { name: 'Close group The beacon inquiry' }))
    expect(rows()).toHaveLength(1)
    expect(screen.getByLabelText('The beacon inquiry outline')).toHaveTextContent('3 elements')
    expect(
      screen.getByRole('button', { name: 'Open group The beacon inquiry' }),
    ).toBeInTheDocument()
  })

  it('regroups the outline by quest state from the same control', () => {
    render(<Harness initial={beaconArc()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Outline list' }))
    fireEvent.change(screen.getByLabelText('Group'), { target: { value: 'questState' } })
    expect(screen.getByRole('button', { name: 'Close group Investigating' })).toBeInTheDocument()
  })

  it('refuses to group by quest when the quest model is not available', () => {
    render(<Harness initial={{ ...beaconArc(), quests: null }} />)
    const field = screen.getByLabelText('Group')
    expect(field).toHaveAttribute('aria-disabled', 'true')
    // And offers nothing it cannot honour.
    expect(field.querySelectorAll('option')).toHaveLength(1)
  })

  it('never mutes a scene a release-blocking diagnostic names', () => {
    render(<Harness initial={beaconArc()} />)
    act(() => useUI.getState().toggleNarrativeFilter('outOfDate'))

    // The drowned ruins has no out-of-date work, so the filter dims it.
    expect(screen.getByTestId('flow-node-scene.ruins')).toHaveClass('is-muted')
    // The long road has none either — and it is the scene with the unfinished
    // exit, so the filter is not allowed to hide it.
    expect(screen.getByTestId('flow-node-scene.road')).not.toHaveClass('is-muted')
  })

  it('mutes by participant, and still not a blocking scene', () => {
    render(<Harness initial={beaconArc()} />)
    fireEvent.change(screen.getByLabelText('Participant'), { target: { value: 'Orren' } })
    expect(screen.getByTestId('flow-node-scene.ruins')).toHaveClass('is-muted')
    expect(screen.getByTestId('flow-node-scene.road')).not.toHaveClass('is-muted')
  })
})

describe('authoring the arc', () => {
  it('creates a scene with exactly the change the form path would make', () => {
    let latest: FlowArc | null = null
    const { container } = render(<Harness initial={beaconArc()} onArc={(arc) => (latest = arc)} />)
    fireEvent.click(nodeEl(container, 'scene.ruins'))
    fireEvent.click(screen.getByRole('button', { name: 'Add scene' }))

    // Byte for byte the result of the shared model function, because the canvas
    // and the outline both call it and there is no second implementation.
    expect(latest!.level).toEqual(addElement(beaconArc().level, 'scene', 'scene.ruins').scene)
  })

  it('sets a scene exit by connecting two nodes, on a scene that had none', () => {
    let latest: FlowArc | null = null
    const { container } = render(<Harness initial={beaconArc()} onArc={(arc) => (latest = arc)} />)

    // Handles are not focusable and React Flow will not make them so, so the
    // connection is node to node — and the port here does not exist yet.
    fireEvent.keyDown(nodeEl(container, 'scene.ruins'), { key: 'c' })
    fireEvent.keyDown(nodeEl(container, 'scene.council'), { key: 'c' })

    expect(scenesOf(latest!).find((scene) => scene.id === 'scene.ruins')?.out).toEqual([
      { id: 'exit.1', label: 'Exit 1', to: 'scene.council' },
    ])
  })

  it('selects the scene itself in the shared selection, not the arc', () => {
    const { container } = render(<Harness initial={beaconArc()} />)
    fireEvent.click(nodeEl(container, 'scene.road'))
    expect(useUI.getState().narrative).toEqual({
      sceneId: 'scene.road',
      beatId: null,
      lineId: null,
    })
    expect(ARC_STORE.getState().selectedId).toBe('scene.road')
  })

  it('enters a scene on Enter and on a double-click, and only then', () => {
    const onEnter = vi.fn()
    const { container } = render(<Harness initial={beaconArc()} onEnter={onEnter} />)

    fireEvent.keyDown(nodeEl(container, 'scene.road'), { key: ' ' })
    expect(onEnter).not.toHaveBeenCalled()

    fireEvent.keyDown(nodeEl(container, 'scene.road'), { key: 'Enter' })
    expect(onEnter).toHaveBeenCalledWith('scene.road')

    fireEvent.doubleClick(nodeEl(container, 'scene.beacon'))
    expect(onEnter).toHaveBeenLastCalledWith('scene.beacon')
  })

  it('refuses every structural edit in a read-only project', () => {
    const onArc = vi.fn()
    const { container } = render(<Harness initial={beaconArc()} readOnly onArc={onArc} />)
    fireEvent.keyDown(nodeEl(container, 'scene.ruins'), { key: 'Delete' })
    expect(onArc).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: 'Add scene' })).toBeDisabled()
  })
})

describe('the outline list, as a full alternative to the arc canvas', () => {
  const openOutline = (arc = beaconArc()) => {
    const rendered = render(<Harness initial={arc} onEnter={() => {}} />)
    fireEvent.click(screen.getByRole('button', { name: 'Outline list' }))
    return rendered
  }

  it('reads the arc in story order and marks the scene with no way in', () => {
    openOutline()
    const rows = screen
      .getByLabelText('The beacon inquiry outline')
      .querySelectorAll(':scope > .nrt-outline-row')
    expect(rows[0]).toHaveTextContent('Council hearing')
    expect(rows[1]).toHaveTextContent('The long road out')
    expect(rows[3]).toHaveTextContent('Not reachable from the start')
  })

  it('sets an exit with a select rather than a gesture', () => {
    let latest: FlowArc | null = null
    render(<Harness initial={beaconArc()} onArc={(arc) => (latest = arc)} />)
    fireEvent.click(screen.getByRole('button', { name: 'Outline list' }))

    fireEvent.change(screen.getByLabelText('The long road out — Turn back leads to'), {
      target: { value: 'scene.ruins' },
    })
    expect(
      scenesOf(latest!)
        .find((scene) => scene.id === 'scene.road')
        ?.out.find((port) => port.id === 'link.back')?.to,
    ).toBe('scene.ruins')
  })

  it('offers the unauthored exit as a control too, so the keyboard can author one', () => {
    let latest: FlowArc | null = null
    render(<Harness initial={beaconArc()} onArc={(arc) => (latest = arc)} />)
    fireEvent.click(screen.getByRole('button', { name: 'Outline list' }))

    fireEvent.change(screen.getByLabelText('The drowned ruins — Exit 1 leads to'), {
      target: { value: 'scene.beacon' },
    })
    expect(scenesOf(latest!).find((scene) => scene.id === 'scene.ruins')?.out).toEqual([
      { id: 'exit.1', label: 'Exit 1', to: 'scene.beacon' },
    ])
  })

  it('offers the drill-down as an ordinary button', () => {
    const onEnter = vi.fn()
    render(<Harness initial={beaconArc()} onEnter={onEnter} />)
    fireEvent.click(screen.getByRole('button', { name: 'Outline list' }))
    fireEvent.click(screen.getAllByRole('button', { name: 'Open scene' })[0]!)
    expect(onEnter).toHaveBeenCalledWith('scene.council')
  })
})

describe('where the plane is left', () => {
  it('opens at the viewport this level was last on, rather than fitting again', () => {
    // jsdom cannot pan, so the viewport is set the way the canvas itself sets
    // it — through the level store — and what is asserted is that the plane
    // opens there. The panning that writes it needs a person and a display.
    act(() => ARC_STORE.getState().setViewport({ x: 120, y: 40, zoom: 0.5 }))
    const { container } = render(<Harness initial={beaconArc()} />)
    expect(container.querySelector('.react-flow__viewport')?.getAttribute('style')).toContain(
      'translate(120px,40px) scale(0.5)',
    )
  })
})
