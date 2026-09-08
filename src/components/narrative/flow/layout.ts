import type { FlowGraph } from './graph'
import type { FlowKind } from './model'

/**
 * Where the boxes go — and, just as importantly, what happens when the answer
 * arrives after the question stopped mattering.
 *
 * Two layouts live here, and they do different jobs:
 *
 * - **`seedPositions`** is a synchronous longest-path layering, about forty
 *   lines, run on every structural change. It is not a good drawing; it is a
 *   *readable* one, available in the same frame as the edit, with no worker and
 *   no await. Without it a freshly added node would sit at the origin until elk
 *   came back, and a canvas with no stored positions at all would open as a
 *   pile.
 * - **elk `elk.layered`**, in a Web Worker, on demand. This is the good drawing.
 *   The spike measured it at 168 ms for 304 nodes in Node — ten dropped frames
 *   if it ran on the main thread — and 431 KB gzipped if it is imported into
 *   the main chunk rather than a worker. So: worker, always.
 */

/**
 * Fixed node sizes, in the same pixels `narrative.css` gives them.
 *
 * Fixed, per the spike's rule: a beat node that grew with its line count would
 * make layout a function of generation output, so re-running Build would move
 * every box on the canvas. Layout is told these numbers rather than measuring
 * the DOM, which also means a layout can be computed before anything is
 * rendered — including in a test, and including for a node that is off screen.
 */
export const NODE_SIZE: Record<FlowKind | 'group', { width: number; height: number }> = {
  // Tall enough for the two badge rows #189 adds — the lifecycle counts and the
  // diagnostics — without growing when they appear. A box whose height depended
  // on how much work was outstanding would move every other box on the canvas
  // every time a Build finished, which is the same rule that keeps a beat's
  // line count off its face.
  beat: { width: 224, height: 136 },
  choice: { width: 224, height: 128 },
  condition: { width: 224, height: 88 },
  outcome: { width: 224, height: 128 },
  end: { width: 160, height: 64 },
  sceneLink: { width: 224, height: 80 },
  // Wider and taller than a beat: a scene node carries a name, its
  // participants and four rolled-up work counts, and the counts are the reason
  // the arc view exists.
  scene: { width: 248, height: 208 },
  questStage: { width: 248, height: 132 },
  missing: { width: 200, height: 76 },
  group: { width: 224, height: 88 },
}

const GAP_X = 48
const GAP_Y = 64

export interface XY {
  x: number
  y: number
}

export interface LayoutRequest {
  nodes: { id: string; width: number; height: number; groupId: string | null }[]
  edges: { id: string; source: string; target: string }[]
  groups: string[]
}

export interface LayoutResult {
  positions: Record<string, XY>
}

/** Anything that can lay a graph out. The canvas is handed one; see below. */
export type LayoutRunner = (request: LayoutRequest) => Promise<LayoutResult>

/**
 * The request elk is given, derived from the graph the canvas is drawing.
 *
 * Group membership is sent as hierarchy so elk keeps a group's elements
 * together; the reply is flattened back to absolute coordinates, because the
 * canvas draws a group as a frame *behind* its members rather than as a React
 * Flow parent. Parenting would buy containment and cost relative coordinates,
 * drag-to-reparent and extent clamping on every node.
 */
export function layoutRequest(graph: FlowGraph): LayoutRequest {
  const groups = new Set<string>()
  for (const node of graph.nodes) {
    if (node.groupId) groups.add(node.groupId)
  }
  return {
    nodes: graph.nodes.map((node) => ({
      id: node.id,
      ...NODE_SIZE[node.kind],
      groupId: node.groupId,
    })),
    edges: graph.edges.map((edge) => ({ id: edge.id, source: edge.source, target: edge.target })),
    groups: [...groups],
  }
}

/**
 * A synchronous, deterministic layering. Good enough to read, cheap enough to
 * run on every keystroke.
 *
 * Longest path from the roots, ignoring back edges, so a node sits one row
 * below its deepest predecessor. Within a row, members of a group are kept
 * adjacent and authored order is preserved — the group frames are drawn from
 * child bounds, so clustering is what stops them overlapping before elk runs.
 */
export function seedPositions(graph: FlowGraph): Record<string, XY> {
  const rank = new Map<string, number>()
  for (const node of graph.nodes) rank.set(node.id, 0)

  const forward = graph.edges.filter((edge) => !edge.back)
  // Relaxation rather than a topological sort: the graph is small, bounded by
  // the node budget, and this needs no ordering pass of its own.
  for (let pass = 0; pass < graph.nodes.length; pass++) {
    let moved = false
    for (const edge of forward) {
      const from = rank.get(edge.source)
      const to = rank.get(edge.target)
      if (from === undefined || to === undefined) continue
      if (to < from + 1) {
        rank.set(edge.target, from + 1)
        moved = true
      }
    }
    if (!moved) break
  }

  const rows = new Map<number, string[]>()
  // A stable sort by group id, so authored order survives inside each group.
  const order = [...graph.nodes].sort((a, b) => (a.groupId ?? '').localeCompare(b.groupId ?? ''))
  for (const node of order) {
    const row = rank.get(node.id) ?? 0
    const list = rows.get(row)
    if (list) list.push(node.id)
    else rows.set(row, [node.id])
  }

  const size = new Map(graph.nodes.map((node) => [node.id, NODE_SIZE[node.kind]]))
  const positions: Record<string, XY> = {}
  let y = 0
  for (const row of [...rows.keys()].sort((a, b) => a - b)) {
    const ids = rows.get(row) ?? []
    const width = ids.reduce((sum, id) => sum + (size.get(id)?.width ?? 0) + GAP_X, -GAP_X)
    let x = -width / 2
    let tallest = 0
    for (const id of ids) {
      const box = size.get(id) ?? { width: 0, height: 0 }
      positions[id] = { x, y }
      x += box.width + GAP_X
      tallest = Math.max(tallest, box.height)
    }
    y += tallest + GAP_Y
  }
  return positions
}

/**
 * A ticket window for layout results.
 *
 * elkjs has **no cancellation**: once a layout is running it runs to
 * completion, and a graph edited twice in quick succession produces two answers
 * in an order nobody promised. Terminating the worker to cancel would cost the
 * ~187 ms GWT cold start again, on every edit. So the caller discards instead —
 * take a ticket before asking, and only apply a result whose ticket is still
 * the newest one issued.
 *
 * Tiny on purpose. It is the whole cancellation story, it is pure, and it is
 * testable without a worker, a canvas or a clock.
 */
export function createLayoutGate() {
  let latest = 0
  return {
    /** Take a ticket for a layout about to be requested. */
    begin: () => ++latest,
    /** True only for the newest ticket. A superseded result is dropped here. */
    accept: (ticket: number) => ticket === latest,
    /** Drop every result in flight — used when the canvas changes scene. */
    abandon: () => {
      latest += 1
    },
  }
}

// The upstream worker protocol and failure handling live in one lazy runner.
export { elkLayout } from './elkRunner'
