import {
  FLOW_KIND_LABEL,
  type FlowElement,
  type FlowGroup,
  type FlowKind,
  type FlowLevel,
  type FlowPort,
} from './model'

/**
 * From a scene to the bounded set of boxes React Flow is allowed to be given.
 *
 * The important word is *given*. The spike measured React Flow's own store, not
 * its DOM: it keeps per-node internals, adjacency and measurements for every
 * node it receives, whether or not it draws it, and the cost of that grows
 * superlinearly — 1.1 s to mount 300 in jsdom, 12.2 s for 1,000, and
 * `onlyRenderVisibleElements` roughly halves both without flattening the curve.
 * So a canvas that hands over 1,000 nodes and hides 700 of them has already
 * paid for 1,000. Everything below therefore bounds the graph *before* it
 * reaches the `nodes` prop, and `onlyRenderVisibleElements` is the backstop for
 * panning inside that bound rather than the mechanism.
 *
 * The two mechanisms, in the order they apply:
 *
 *   1. **Collapse.** A closed group is one node. Its inner elements never
 *      appear, and edges crossing its boundary re-point at the group box.
 *   2. **Keyhole.** A scene still over budget with everything closed draws only
 *      the neighbourhood of the selection, and says so in a banner. A story
 *      that big is being read through a keyhole either way; better one that
 *      admits it.
 *
 * Everything here works on a `FlowLevel`, which is a scene *or* an arc (#187).
 * The two differ in what their nodes mean and in nothing this file cares about,
 * so there is one graph builder rather than two — and the arc's 1,000-scene
 * fixture is therefore held to the same measured budget by the same code.
 */

/**
 * 300 nodes on screen and 300 nodes in the store.
 *
 * Derived from jsdom mounts and Node layout times, not from a browser: the
 * curve bends between 300 and 600 in every measurement the spike took. Treat it
 * as a number to confirm against a real webview, not as an observed frame rate.
 */
export const NODE_BUDGET = 300

/** How far the keyhole reaches from the selection, in hops, ignoring direction. */
const KEYHOLE_HOPS = 2

/** What a node needs to draw itself. No coordinates: see `FlowPositions`. */
export interface FlowGraphNode {
  id: string
  kind: FlowKind | 'group'
  title: string
  /** Present for real elements; absent for a collapsed group box. */
  element?: FlowElement
  /** Present for a collapsed group box; absent otherwise. */
  group?: { elements: number; crossings: number }
  /**
   * The container this node was resolved into, or null.
   *
   * Resolved here rather than read off `element.groupId` by every caller,
   * because the arc level regroups the same scenes by quest or by quest state
   * without touching the model. Layout and the group frames read this.
   */
  groupId: string | null
  /**
   * True when a release-blocking diagnostic names this node.
   *
   * Carried on the node so the *filters* can be forbidden from muting it. A
   * filter that hides the one box a release is blocked on is worse than no
   * filter at all, and the only way to guarantee it cannot is for the node to
   * know.
   */
  blocking: boolean
  /** Ports out, already resolved against what is visible. */
  ports: FlowPort[]
  /**
   * A port that does not exist yet, offered so the next one can be authored.
   *
   * Rendered as one more source handle and one more outline row. Connecting
   * from it is what *creates* a way out, which is the only way a node born with
   * no ports — a scene, at the arc level — can ever gain one by pointer or by
   * keyboard.
   */
  spare?: FlowPort | null
  /**
   * How many routes arrive here.
   *
   * Drawn explicitly — a widened cap and the words "3 routes in" — rather than
   * left to edge geometry. Reconvergence is the thing #186 names, and in a
   * layered drawing the only other signal is several lines ending at the same
   * box, which is exactly the signal a dense scene destroys.
   */
  inbound: number
  /** The scene's start, drawn with an entry cap. */
  entry: boolean
  /** The narrative path selecting this node means, for the shared selection. */
  beatId: string | null
}

export interface FlowGraphEdge {
  id: string
  source: string
  sourceHandle: string
  target: string
  label: string | null
  /**
   * True when this edge climbs back up the graph.
   *
   * Computed here by depth-first search rather than read out of a layout
   * result, so a back edge is dashed and named from the first frame — before
   * elk has answered, and whether or not elk is ever asked.
   */
  back: boolean
}

export interface FlowGraph {
  nodes: FlowGraphNode[]
  edges: FlowGraphEdge[]
  /** The containers in play, after any regrouping. What frames are drawn from. */
  groups: FlowGroup[]
  /** Elements folded into a closed group or left outside the keyhole. */
  hidden: number
  /** True when the budget, not the writer, decided what is on screen. */
  keyhole: boolean
  /** Every element in the scene, bounded or not. For the banner's wording. */
  total: number
}

/**
 * Which groups a scene should open with.
 *
 * A scene inside budget opens fully: closing boxes a writer could have seen is
 * a worse first impression than a slightly busy canvas. A scene over budget
 * opens with every group closed, which is the cheapest honest answer and the
 * one the spike measured — the 1,000-node arc fixture collapses to 240 nodes
 * with 80% of its quests closed, and elk lays that out in 131 ms.
 */
export function defaultClosedGroups(
  level: FlowLevel,
  budget = NODE_BUDGET,
  groups: readonly FlowGroup[] = level.groups,
): string[] {
  if (level.elements.length <= budget) return []
  return groups.map((group) => group.id)
}

/**
 * How many boxes a scene would put on the canvas with these groups closed.
 *
 * Cheap, and separate from `buildGraph`, because the canvas needs the answer
 * *before* it decides whether the keyhole is in play — and feeding the
 * selection into `buildGraph` when it is not would rebuild every node object on
 * every click.
 */
export function visibleNodeCount(
  level: FlowLevel,
  closedGroups: readonly string[],
  groupOf: (element: FlowElement) => string | null = ownGroup,
): number {
  const closed = new Set(closedGroups)
  let count = 0
  const seen = new Set<string>()
  for (const element of level.elements) {
    const owner = groupOf(element)
    if (owner && closed.has(owner)) {
      if (!seen.has(owner)) {
        seen.add(owner)
        count += 1
      }
      continue
    }
    count += 1
  }
  return count
}

/** A level's own containers, and the default when nothing regroups it. */
function ownGroup(element: FlowElement): string | null {
  return element.groupId ?? null
}

/** The node that stands for an element: itself, or the group that swallowed it. */
function representative(id: string, group: string | null, closed: ReadonlySet<string>): string {
  return group !== null && closed.has(group) ? group : id
}

/**
 * Depth-first search marking every edge that returns to a node still on the
 * stack. Iterative, because a long scene is a long path and a recursive walk
 * would put the canvas one deep story away from a stack overflow.
 */
function findBackEdges(nodeIds: string[], edges: FlowGraphEdge[]): Set<string> {
  const out = new Map<string, FlowGraphEdge[]>()
  for (const edge of edges) {
    const list = out.get(edge.source)
    if (list) list.push(edge)
    else out.set(edge.source, [edge])
  }

  const back = new Set<string>()
  const done = new Set<string>()
  const onStack = new Set<string>()

  for (const root of nodeIds) {
    if (done.has(root)) continue
    const stack: { id: string; at: number }[] = [{ id: root, at: 0 }]
    onStack.add(root)

    while (stack.length > 0) {
      const frame = stack[stack.length - 1]!
      const outgoing = out.get(frame.id) ?? []
      if (frame.at >= outgoing.length) {
        onStack.delete(frame.id)
        done.add(frame.id)
        stack.pop()
        continue
      }
      const edge = outgoing[frame.at++]!
      if (onStack.has(edge.target)) back.add(edge.id)
      else if (!done.has(edge.target)) {
        stack.push({ id: edge.target, at: 0 })
        onStack.add(edge.target)
      }
    }
  }

  return back
}

/** Node ids within `hops` of `from`, ignoring edge direction, capped at `budget`. */
function neighbourhood(
  from: string,
  edges: FlowGraphEdge[],
  hops: number,
  budget: number,
): Set<string> {
  const near = new Map<string, string[]>()
  const link = (a: string, b: string) => {
    const list = near.get(a)
    if (list) list.push(b)
    else near.set(a, [b])
  }
  for (const edge of edges) {
    link(edge.source, edge.target)
    link(edge.target, edge.source)
  }

  const seen = new Set<string>([from])
  let frontier = [from]
  for (let hop = 0; hop < hops && seen.size < budget; hop++) {
    const next: string[] = []
    for (const id of frontier) {
      for (const other of near.get(id) ?? []) {
        if (seen.size >= budget) break
        if (seen.has(other)) continue
        seen.add(other)
        next.push(other)
      }
    }
    frontier = next
  }
  return seen
}

/**
 * The graph to draw.
 *
 * Pure, and knowing nothing about React Flow — which is what lets the budget be
 * tested by counting the array it returns rather than by counting DOM nodes in
 * a jsdom viewport that turned out to be one node tall.
 */
export function buildGraph(
  level: FlowLevel,
  options: {
    closedGroups?: readonly string[]
    /** Where the keyhole is centred when one is needed. */
    focusId?: string | null
    budget?: number
    /**
     * Regroup the level without touching it.
     *
     * The arc view groups the same scenes by quest or by quest state, and both
     * are presentation: neither may write a `groupId` into the model, because a
     * writer switching the grouping control has not edited their story. So the
     * containers are handed in here instead, and the model stays the one thing
     * a save would write.
     */
    grouping?: { groups: readonly FlowGroup[]; of: (element: FlowElement) => string | null }
    /**
     * Draw a port that leads nowhere as a wire into a named box.
     *
     * Off inside a scene, where the element with the hole in it is right there
     * on screen and an extra box per unset port would double a busy canvas. On
     * across an arc, where a scene with no way into it is invisible otherwise —
     * it looks exactly like a scene the quest deliberately ends before.
     */
    dangling?: boolean
    /** Nodes a release-blocking diagnostic names. Filters may not mute these. */
    blockingIds?: readonly string[]
    /**
     * One more way out than the element has, for authoring the next one.
     *
     * A scene with no exits yet has no source handle at all, so there is
     * nothing for a pointer to drag from and nothing for the keyboard
     * connector to pick — "connect two nodes to set a scene exit" would be
     * impossible on a fresh scene. The arc hands in a spare port; the scene
     * level does not, because every element there is born with the ports its
     * kind requires.
     */
    spare?: (element: FlowElement) => FlowPort | null
  } = {},
): FlowGraph {
  const budget = options.budget ?? NODE_BUDGET
  const closed = new Set(options.closedGroups ?? [])
  const groups = [...(options.grouping?.groups ?? level.groups)]
  const groupOf = options.grouping?.of ?? ownGroup
  const blocking = new Set(options.blockingIds ?? [])
  const byId = new Map(level.elements.map((element) => [element.id, element]))

  const visible: FlowGraphNode[] = []
  const groupCounts = new Map<string, { elements: number; crossings: number }>()
  for (const group of groups) {
    if (closed.has(group.id)) groupCounts.set(group.id, { elements: 0, crossings: 0 })
  }

  const owners = new Map<string, string | null>()
  for (const element of level.elements) {
    const owner = groupOf(element)
    owners.set(element.id, owner)
    if (owner && closed.has(owner)) {
      const counts = groupCounts.get(owner)
      if (counts) counts.elements += 1
      continue
    }
    visible.push({
      id: element.id,
      kind: element.kind,
      title: element.title,
      element,
      groupId: owner,
      blocking: blocking.has(element.id),
      ports: element.out,
      spare: options.spare?.(element) ?? null,
      inbound: 0,
      entry: level.entryId === element.id,
      beatId: element.beatId ?? null,
    })
  }

  for (const group of groups) {
    const counts = groupCounts.get(group.id)
    if (!counts) continue
    visible.push({
      id: group.id,
      kind: 'group',
      title: group.name,
      group: counts,
      groupId: null,
      blocking: false,
      ports: [],
      inbound: 0,
      entry: false,
      beatId: null,
    })
  }

  // Edges, with both ends mapped through whatever swallowed them. An edge whose
  // ends collapse to the same box is not drawn — it is counted on the box.
  let edges: FlowGraphEdge[] = []
  for (const element of level.elements) {
    const owner = owners.get(element.id) ?? null
    const source = representative(element.id, owner, closed)
    for (const port of element.out) {
      const destination = port.to === null ? undefined : byId.get(port.to)
      if (!destination) {
        // An unresolved destination. Inside a closed group it is folded away
        // with everything else — collapse is the writer's own choice, and the
        // group box's crossing count is the signal there. Outside one, and only
        // when this level asks for it, the hole gets a box of its own so the
        // wire into it is visible.
        if (!options.dangling || source !== element.id) continue
        const hole = danglingNode(element, port)
        visible.push(hole)
        edges.push({
          id: `${element.id}:${port.id}`,
          source,
          sourceHandle: port.id,
          target: hole.id,
          label: port.requires ?? null,
          back: false,
        })
        continue
      }
      const target = representative(destination.id, owners.get(destination.id) ?? null, closed)
      // An edge with both ends inside one closed group is internal to it: not
      // drawn, and not a boundary crossing either.
      if (source === target && groupCounts.has(source)) continue
      for (const end of new Set([source, target])) {
        const crossing = groupCounts.get(end)
        if (crossing) crossing.crossings += 1
      }
      edges.push({
        id: `${element.id}:${port.id}`,
        source,
        sourceHandle: port.id,
        target,
        label: port.requires ?? null,
        back: false,
      })
    }
  }

  let nodes = visible
  let keyhole = false

  /*
   * The backstop. Collapse has already run and the scene is still too big, so
   * the canvas draws the neighbourhood of whatever is selected and says how
   * much it left out. Note the order: this cuts the *node array*, so the
   * budget binds on what React Flow is handed, not on what it paints.
   */
  if (nodes.length > budget) {
    const centre = options.focusId ?? nodes[0]?.id ?? null
    if (centre !== null) {
      const keep = neighbourhood(centre, edges, KEYHOLE_HOPS, budget)
      nodes = nodes.filter((node) => keep.has(node.id))
      edges = edges.filter((edge) => keep.has(edge.source) && keep.has(edge.target))
      keyhole = true
    }
  }

  const drawn = new Set(nodes.map((node) => node.id))
  edges = edges.filter((edge) => drawn.has(edge.source) && drawn.has(edge.target))

  const inbound = new Map<string, number>()
  for (const edge of edges) inbound.set(edge.target, (inbound.get(edge.target) ?? 0) + 1)
  for (const node of nodes) node.inbound = inbound.get(node.id) ?? 0

  const back = findBackEdges(
    nodes.map((node) => node.id),
    edges,
  )
  for (const edge of edges) edge.back = back.has(edge.id)

  return {
    nodes,
    edges,
    groups,
    hidden: level.elements.length - nodes.filter((node) => byId.has(node.id)).length,
    keyhole,
    total: level.elements.length,
  }
}

/**
 * The box an unresolved destination points at.
 *
 * Always blocking. A wire ending here is a route the story cannot take, which
 * is a release problem by definition — so no filter is allowed to mute it, and
 * `blocking` is set here rather than left to the caller's diagnostic list to
 * remember.
 */
function danglingNode(from: FlowElement, port: FlowPort): FlowGraphNode {
  const id = `${from.id}:${port.id}:unresolved`
  return {
    id,
    kind: 'missing',
    title: port.label,
    element: {
      kind: 'missing',
      id,
      title: port.label,
      targetId: port.to,
      beatId: port.via ?? null,
      out: [],
    },
    groupId: null,
    blocking: true,
    ports: [],
    inbound: 0,
    entry: false,
    beatId: port.via ?? null,
  }
}

/**
 * The accessible name of a node: kind, title, and the two facts that are only
 * otherwise legible as shapes.
 *
 * React Flow gives every node `role="group"` and an `aria-label`, and the label
 * it generates by default is the node id. A screen-reader user navigating this
 * canvas needs "Beat, Present evidence, 3 routes in" instead.
 */
export function nodeLabel(node: FlowGraphNode): string {
  const kind = node.kind === 'group' ? 'Group' : FLOW_KIND_LABEL[node.kind]
  const parts = [`${kind}, ${node.title}`]
  if (node.entry) parts.push('scene start')
  if (node.inbound >= 2) parts.push(`${node.inbound} routes in`)
  if (node.group) parts.push(`${node.group.elements} elements, closed`)
  const element = node.element
  if (element?.kind === 'scene') {
    const counts = element.counts
    parts.push(`${counts.beats} beat${counts.beats === 1 ? '' : 's'}`)
  }
  if (element?.kind === 'missing') {
    parts.push(
      element.targetId === null ? 'no destination chosen' : `${element.targetId} not found`,
    )
  }
  /*
   * The badges, said rather than only drawn.
   *
   * A corner badge is not reachable by a reader arrowing between nodes, so the
   * accessible name carries the same counts. Unfiltered on purpose: this is the
   * name of the node, not a rendering of the current chips, and a name that
   * changed when a filter was toggled would be a name that could not be relied
   * on.
   */
  if (element?.counts) {
    for (const [label, value] of [
      ['needing text', element.counts.needsText],
      ['needing review', element.counts.needsReview],
      ['out of date', element.counts.outOfDate],
      ['locked', element.counts.locked],
    ] as const) {
      if (value > 0) parts.push(`${value} ${label}`)
    }
  }
  const errors = (element?.diagnostics ?? []).filter((d) => d.severity === 'error').length
  const warnings = (element?.diagnostics ?? []).length - errors
  if (errors > 0) parts.push(`${errors} error${errors === 1 ? '' : 's'}`)
  if (warnings > 0) parts.push(`${warnings} warning${warnings === 1 ? '' : 's'}`)
  return parts.join(', ')
}
