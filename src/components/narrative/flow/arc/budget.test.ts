import { describe, expect, it } from 'vitest'
import { buildGraph, defaultClosedGroups, visibleNodeCount, NODE_BUDGET } from '../graph'
import { layoutRequest } from '../layout'
import { chainArc } from './fixture'
import { arcGrouping } from './model'

/**
 * The 1,000-scene arc, held to the spike's budget.
 *
 * The measurement that matters is the length of the array `buildGraph` returns,
 * not the number of boxes painted: React Flow keeps per-node internals,
 * adjacency and measurements for every node it is *given*, and that cost grows
 * superlinearly — 1.1 s to mount 300 in jsdom against 12.2 s for 1,000. A view
 * that hands over a thousand and paints two hundred has already paid for a
 * thousand. So everything below counts what would reach the `nodes` prop.
 *
 * The 80%-closed row exists because it is the shape the spike measured and
 * recorded a layout time against; the all-closed row is what the arc view
 * actually opens with at this size.
 */

const ARC = chainArc(1000, 50)
const QUESTS = arcGrouping(ARC, 'quest')

describe('the 1,000-scene arc', () => {
  it('opens with every quest closed, at 50 boxes and no keyhole', () => {
    const closed = defaultClosedGroups(ARC.level, undefined, QUESTS.groups)
    expect(closed).toHaveLength(50)

    const graph = buildGraph(ARC.level, {
      grouping: QUESTS,
      closedGroups: closed,
      dangling: true,
    })
    expect(graph.nodes).toHaveLength(50)
    expect(graph.nodes.length).toBeLessThanOrEqual(NODE_BUDGET)
    // Comfortably inside budget, so nobody is reading it through a keyhole.
    expect(graph.keyhole).toBe(false)
    expect(graph.hidden).toBe(1000)
  })

  it('stays inside budget with 80% of its quests closed — the spike’s fixture', () => {
    const closed = QUESTS.groups.slice(0, 40).map((group) => group.id)
    const graph = buildGraph(ARC.level, {
      grouping: QUESTS,
      closedGroups: closed,
      dangling: true,
    })

    // 800 scenes folded into 40 quest boxes, plus the 200 in the ten open ones.
    expect(graph.nodes).toHaveLength(240)
    expect(graph.edges).toHaveLength(239)
    expect(graph.keyhole).toBe(false)
    // The cheap count agrees with the expensive one, which is what lets the
    // canvas decide whether it needs a keyhole without building the graph.
    expect(visibleNodeCount(ARC.level, closed, QUESTS.of)).toBe(240)
  })

  it('falls back to the keyhole when nothing is closed, rather than handing over 1,000', () => {
    const graph = buildGraph(ARC.level, { grouping: QUESTS, dangling: true })
    expect(graph.nodes.length).toBeLessThanOrEqual(NODE_BUDGET)
    expect(graph.keyhole).toBe(true)
    expect(graph.total).toBe(1000)
  })

  it('sends elk the collapsed graph with its group membership, and no text', () => {
    const closed = QUESTS.groups.slice(0, 40).map((group) => group.id)
    const request = layoutRequest(
      buildGraph(ARC.level, { grouping: QUESTS, closedGroups: closed, dangling: true }),
    )
    expect(request.nodes).toHaveLength(240)
    // The ten open quests are the only containers left; the forty closed ones
    // are boxes, not groups, so elk is never asked to nest an empty one.
    expect(request.groups).toHaveLength(10)
    expect(JSON.stringify(request)).not.toMatch(/Scene \d/)
  })
})
