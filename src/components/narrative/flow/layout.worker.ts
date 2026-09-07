/// <reference lib="webworker" />
import ELK from 'elkjs/lib/elk.bundled.js'
import type { ElkNode } from 'elkjs/lib/elk-api'
import type { LayoutRequest, XY } from './layout'

/**
 * elk, off the main thread.
 *
 * This file is the only place elkjs is imported, and that is the whole point.
 * The spike measured the alternative: imported on the main thread, elkjs costs
 * **+431 KB gzipped** — twenty-six times dagre — and the app parses all of it
 * at start-up. Imported here, Vite emits it as a separate worker asset and the
 * main chunk grows by about 2 KB gzipped, none of which is parsed until the
 * first layout is asked for. The ~187 ms GWT cold start is then paid once, off
 * the main thread, behind a "laying out" state.
 *
 * The instance is kept between messages. Rebuilding it per layout would pay
 * that cold start on every structural edit.
 *
 * Note for anyone fixing a layout bug in here: **never patch a vendored
 * elkjs.** It is EPL-2.0 OR GPL-3.0-or-later, and shipping an unmodified
 * upstream build is fine where shipping a private patch is not. A fix goes
 * upstream, or it is worked around in Wobu's own code.
 */

const elk = new ELK()

/**
 * Model order is set here rather than left to elk's default.
 *
 * A choice's options are authored in an order and a writer expects to see them
 * in it. The spike's table is blunt about the default: three branches come out
 * `b2 b1 b0`, and adding a fourth reshuffles the existing three to
 * `b3 b0 b1 b2`. These two options make the input order the drawn order, which
 * is one setting rather than a constraint array to rebuild on every edit.
 */
const ROOT_OPTIONS = {
  'elk.algorithm': 'layered',
  'elk.direction': 'DOWN',
  'elk.hierarchyHandling': 'INCLUDE_CHILDREN',
  'elk.layered.considerModelOrder.strategy': 'NODES_AND_EDGES',
  'elk.layered.crossingMinimization.forceNodeModelOrder': 'true',
  'elk.layered.spacing.nodeNodeBetweenLayers': '64',
  'elk.spacing.nodeNode': '48',
  'elk.padding': '[top=44,left=24,bottom=24,right=24]',
}

/** Build the elk graph, nesting grouped nodes so a group stays together. */
function toElk(request: LayoutRequest): ElkNode {
  const children = new Map<string, ElkNode[]>()
  for (const group of request.groups) children.set(group, [])

  const roots: ElkNode[] = []
  for (const node of request.nodes) {
    const box: ElkNode = { id: node.id, width: node.width, height: node.height }
    const siblings = node.groupId === null ? null : children.get(node.groupId)
    if (siblings) siblings.push(box)
    else roots.push(box)
  }

  for (const group of request.groups) {
    roots.push({ id: group, children: children.get(group) ?? [] })
  }

  return {
    id: 'root',
    layoutOptions: ROOT_OPTIONS,
    children: roots,
    edges: request.edges.map((edge) => ({
      id: edge.id,
      sources: [edge.source],
      targets: [edge.target],
    })),
  }
}

/** Flatten elk's parent-relative coordinates back to one absolute space. */
function flatten(node: ElkNode, offsetX: number, offsetY: number, into: Record<string, XY>) {
  for (const child of node.children ?? []) {
    const x = offsetX + (child.x ?? 0)
    const y = offsetY + (child.y ?? 0)
    into[child.id] = { x, y }
    if (child.children && child.children.length > 0) flatten(child, x, y, into)
  }
}

self.addEventListener('message', (event: MessageEvent<{ id: number; request: LayoutRequest }>) => {
  const { id, request } = event.data
  elk
    .layout(toElk(request))
    .then((laid) => {
      const positions: Record<string, XY> = {}
      flatten(laid, 0, 0, positions)
      // Group boxes are laid out too, but the canvas draws its frames from the
      // bounds of their members, so only element positions are sent back.
      self.postMessage({ id, positions })
    })
    .catch((error: unknown) => {
      self.postMessage({ id, error: error instanceof Error ? error.message : String(error) })
    })
})
