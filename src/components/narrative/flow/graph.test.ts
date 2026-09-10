import { describe, expect, it } from 'vitest'
import { chainScene, councilHearing } from './fixture'
import { buildGraph, defaultClosedGroups, nodeLabel, visibleNodeCount, NODE_BUDGET } from './graph'
import { connectPort } from './model'

describe('the drawn graph', () => {
  it('draws three approaches reconverging on one verdict, and counts the routes in', () => {
    const graph = buildGraph(councilHearing())
    const verdict = graph.nodes.find((node) => node.id === 'beat.7')
    // Evidence (two ways through the condition), challenge, and threat.
    expect(verdict?.inbound).toBe(4)
    // Explicit, not inferred from the edge geometry — this is the mark #186
    // asks for by name.
    expect(nodeLabel(verdict!)).toBe('Beat, The verdict, 4 routes in')
  })

  it('marks the wire back to the start as a back edge and nothing else', () => {
    const graph = buildGraph(councilHearing())
    expect(graph.edges.filter((edge) => edge.back).map((edge) => edge.id)).toEqual([
      'choice.2:option.2',
    ])
  })

  it('carries a port requirement on the edge it gates', () => {
    const graph = buildGraph(councilHearing())
    expect(graph.edges.find((edge) => edge.id === 'choice.1:option.1')?.label).toBe('has_logbook')
  })

  it('names the scene start', () => {
    const graph = buildGraph(councilHearing())
    expect(graph.nodes.find((node) => node.id === 'beat.1')?.entry).toBe(true)
  })

  it('drops a port with no destination from the edges rather than drawing a dangling wire', () => {
    const scene = connectPort(councilHearing(), { elementId: 'choice.1', portId: 'option.2' }, null)
    const graph = buildGraph(scene)
    expect(graph.edges.some((edge) => edge.id === 'choice.1:option.2')).toBe(false)
    // The element is still there. Only the route is missing, and the
    // diagnostics are what say so.
    expect(graph.nodes.some((node) => node.id === 'beat.3')).toBe(true)
  })
})

describe('collapse', () => {
  it('folds a closed group into one node and re-points the edges that crossed it', () => {
    const open = buildGraph(councilHearing())
    const closed = buildGraph(councilHearing(), { closedGroups: ['group.evidence'] })

    expect(open.nodes).toHaveLength(16)
    // Five elements inside the group become one box.
    expect(closed.nodes).toHaveLength(12)
    expect(closed.hidden).toBe(5)

    const box = closed.nodes.find((node) => node.id === 'group.evidence')
    expect(box?.kind).toBe('group')
    expect(box?.group).toEqual({ elements: 5, crossings: 3 })

    // One edge in from the choice, two out to the verdict — the wires inside
    // are gone entirely rather than hidden.
    expect(closed.edges.filter((edge) => edge.target === 'group.evidence')).toHaveLength(1)
    expect(closed.edges.filter((edge) => edge.source === 'group.evidence')).toHaveLength(2)
    expect(closed.edges.some((edge) => edge.source === 'condition.1')).toBe(false)
  })

  it('opens a scene inside the budget fully, and a scene over it with every group closed', () => {
    expect(defaultClosedGroups(councilHearing())).toEqual([])
    const huge = chainScene(NODE_BUDGET + 40, 4)
    expect(defaultClosedGroups(huge)).toHaveLength(4)
  })
})

describe('the node budget', () => {
  it('bounds the array before it reaches React Flow, not the pixels after it', () => {
    // The measurement that matters: React Flow keeps per-node internals for
    // every node it is *given*, so a canvas that hands over a thousand and
    // paints three hundred has already paid for a thousand.
    const scene = chainScene(1000, 50)

    // Nothing gets out of here over budget, by either route.
    const uncollapsed = buildGraph(scene)
    expect(uncollapsed.nodes.length).toBeLessThanOrEqual(NODE_BUDGET)
    expect(uncollapsed.keyhole).toBe(true)

    // Collapse first, and the same thousand-element scene fits comfortably
    // with no keyhole at all — 50 quest boxes instead of 1,000 beats.
    const collapsed = buildGraph(scene, { closedGroups: defaultClosedGroups(scene) })
    expect(collapsed.nodes).toHaveLength(50)
    expect(collapsed.keyhole).toBe(false)
  })

  it('falls back to the neighbourhood of the selection when collapse is not enough', () => {
    // A scene with no groups to close: collapse has nothing to fold, so the
    // keyhole is all that is left. It must say so rather than stall.
    const scene = chainScene(400, 1)
    const graph = buildGraph(scene, { closedGroups: [], focusId: 'beat.200', budget: 20 })
    expect(graph.keyhole).toBe(true)
    expect(graph.nodes.length).toBeLessThanOrEqual(20)
    expect(graph.nodes.some((node) => node.id === 'beat.200')).toBe(true)
    expect(graph.hidden).toBe(400 - graph.nodes.length)
    expect(graph.total).toBe(400)
  })

  it('counts what would be drawn without building the graph', () => {
    const scene = councilHearing()
    expect(visibleNodeCount(scene, [])).toBe(16)
    expect(visibleNodeCount(scene, ['group.evidence'])).toBe(12)
  })
})
