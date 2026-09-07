import { describe, expect, it } from 'vitest'
import { councilHearing } from './fixture'
import { buildGraph } from './graph'
import { createLayoutGate, layoutRequest, seedPositions, NODE_SIZE } from './layout'

describe('the layout gate', () => {
  it('accepts the newest result and discards a superseded one', async () => {
    // elkjs has no cancellation. A graph edited twice in quick succession
    // produces two answers in an order nobody promised, and the older one must
    // not be allowed to move boxes back.
    const gate = createLayoutGate()
    const first = gate.begin()
    const second = gate.begin()

    expect(gate.accept(second)).toBe(true)
    expect(gate.accept(first)).toBe(false)
  })

  it('applies a result that is still current even if it arrives slowly', async () => {
    const gate = createLayoutGate()
    const ticket = gate.begin()
    await Promise.resolve()
    expect(gate.accept(ticket)).toBe(true)
  })

  it('abandons everything in flight when the canvas changes scene', () => {
    const gate = createLayoutGate()
    const ticket = gate.begin()
    gate.abandon()
    expect(gate.accept(ticket)).toBe(false)
  })
})

describe('the seed layout', () => {
  it('puts every node somewhere, in the same place every time', () => {
    const graph = buildGraph(councilHearing())
    const once = seedPositions(graph)
    const twice = seedPositions(graph)
    expect(Object.keys(once).sort()).toEqual(graph.nodes.map((node) => node.id).sort())
    expect(once).toEqual(twice)
  })

  it('puts a node below its deepest predecessor and ignores the back edge', () => {
    const graph = buildGraph(councilHearing())
    const at = seedPositions(graph)
    // Were the back edge counted, the arrival beat would be pushed below the
    // verdict choice that loops up to it and the whole drawing would invert.
    expect(at['beat.1']!.y).toBe(0)
    expect(at['choice.1']!.y).toBeGreaterThan(at['beat.1']!.y)
    expect(at['beat.7']!.y).toBeGreaterThan(at['outcome.2']!.y)
    expect(at['beat.7']!.y).toBeGreaterThan(at['beat.5']!.y)
  })
})

describe('the request elk is given', () => {
  it('carries fixed node sizes and group membership, and no text', () => {
    const request = layoutRequest(buildGraph(councilHearing()))
    expect(request.groups).toEqual(['group.evidence'])
    const beat = request.nodes.find((node) => node.id === 'beat.2')
    // Fixed, so a Build that adds five variants cannot reflow the canvas.
    expect(beat).toEqual({ id: 'beat.2', ...NODE_SIZE.beat, groupId: 'group.evidence' })
    expect(request.edges).toHaveLength(buildGraph(councilHearing()).edges.length)
  })
})
