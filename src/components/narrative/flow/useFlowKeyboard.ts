import type { KeyboardEvent, RefObject } from 'react'
import type { FlowGraph } from './graph'
import { useFlowLevelApi, type FlowConnection } from './flowStore'
import { unresolvedPorts } from './model'

/**
 * The keyboard layer — the part React Flow does not have and will not grow.
 *
 * What the library gives, asserted against the DOM in the spike: every node is
 * `tabindex="0"` with `role="group"`, every edge likewise, Enter selects, and
 * `deleteKeyCode` deletes. What it does not give:
 *
 * - **Handles are not focusable.** No `tabindex`, no `role`, no key handler.
 *   `connectOnClick` lets a pointer click one handle then another; there is no
 *   keyboard equivalent, and there is no way to add one without forking the
 *   component. So connection here is node-to-node: `c` on the source, `c` on
 *   the target. The port is chosen for the writer — the first one with no
 *   destination — because a port is not a thing the keyboard can reach.
 * - **Arrow keys move a node rather than travel to its neighbours**, and `Tab`
 *   order is node-array order, which has nothing to do with the story. So the
 *   arrows follow *edges* here: down to a successor, up to a predecessor, left
 *   and right between the branches of one parent.
 *
 * Written against the public API only: `data-id` on the rendered wrapper, DOM
 * focus, and the graph the canvas already built. No forked component, no store
 * internals.
 *
 * This layer is ours to keep tested. A refactor that breaks keyboard connection
 * breaks it silently — nothing else in the app will fail.
 */

/** The id in the `data-id` of whichever node or edge the event came from. */
function targetOf(event: KeyboardEvent<HTMLElement>, selector: string): string | null {
  const element = event.target as HTMLElement | null
  return element?.closest?.(selector)?.getAttribute('data-id') ?? null
}

export function useFlowKeyboard({
  graph,
  container,
  readOnly,
  onConnect,
  onDisconnect,
  onDelete,
  canonicalDeletion = false,
  onSelect,
  onActivate,
}: {
  graph: FlowGraph
  container: RefObject<HTMLDivElement | null>
  readOnly: boolean
  /** Returns whether the connection was made: a refusal must not be announced as one. */
  onConnect: (from: FlowConnection, to: string) => boolean
  onDisconnect: (from: FlowConnection) => boolean
  onDelete: (id: string) => boolean
  /** Canonical source edits announce and reveal their own surviving target. */
  canonicalDeletion?: boolean
  onSelect: (id: string) => void
  /**
   * Go *into* this node — the arc level's drill-down (#187).
   *
   * Bound to Enter, and Enter only: Space still selects. React Flow gives both
   * keys the same meaning, and a canvas where the two most obvious keys both
   * navigate away would make a screen-reader user unable to simply *look* at a
   * scene node without leaving the arc.
   */
  onActivate?: (id: string) => void
}) {
  const store = useFlowLevelApi()
  /**
   * Move DOM focus as well as selection, so a screen reader follows along.
   *
   * Not memoised, and neither is the handler below. There is exactly one of
   * each per canvas — the listener sits on the container, not on every node —
   * so memoising them would buy nothing and would mean reading `container.current`
   * through a dependency array, which is a lie about when the value changes.
   */
  const focus = (id: string) => {
    onSelect(id)
    // Matched rather than selected: a narrative id may contain anything, and
    // building a selector out of user data is how a quote mark becomes a bug.
    const nodes = container.current?.querySelectorAll<HTMLElement>('.react-flow__node') ?? []
    for (const node of nodes) {
      if (node.getAttribute('data-id') === id) {
        node.focus()
        return
      }
    }
  }

  return (event: KeyboardEvent<HTMLDivElement>) => {
    const { announce, beginConnect } = store.getState()
    const connectFrom = store.getState().connectFrom
    const edgeId = targetOf(event, '.react-flow__edge')
    const nodeId = targetOf(event, '.react-flow__node')

    // An edge is focusable and carries `element:port` as its id, so deleting
    // one is how a destination is disconnected without a pointer.
    if (edgeId !== null && (event.key === 'Delete' || event.key === 'Backspace')) {
      event.preventDefault()
      if (readOnly) return announce('This project folder is read-only.')
      const edge = graph.edges.find((candidate) => candidate.id === edgeId)
      if (!edge) return
      // Only when it happened. On a source-backed scene it does not: a
      // destination cannot be cleared, and `useSceneEdits` has already said so
      // in this same live region.
      if (onDisconnect({ elementId: edge.source, portId: edge.sourceHandle })) {
        announce('Destination removed. The route now has no destination.')
      }
      return
    }

    if (nodeId === null) return
    const here = graph.nodes.find((node) => node.id === nodeId)
    if (!here) return

    const successors = graph.edges.filter((edge) => edge.source === nodeId).map((e) => e.target)
    const predecessors = graph.edges.filter((edge) => edge.target === nodeId).map((e) => e.source)

    switch (event.key) {
      case 'ArrowDown': {
        event.preventDefault()
        if (successors[0]) focus(successors[0])
        return
      }
      case 'ArrowUp': {
        event.preventDefault()
        if (predecessors[0]) focus(predecessors[0])
        return
      }
      case 'ArrowLeft':
      case 'ArrowRight': {
        event.preventDefault()
        // Siblings are the other targets of whichever parent leads here, in
        // the parent's authored port order — so Left and Right walk a
        // choice's options in the order the writer typed them.
        const parent = predecessors[0]
        if (parent === undefined) return
        const siblings = graph.edges.filter((edge) => edge.source === parent).map((e) => e.target)
        const at = siblings.indexOf(nodeId)
        const to = siblings[at + (event.key === 'ArrowRight' ? 1 : -1)]
        if (to) focus(to)
        return
      }
      case ' ': {
        event.preventDefault()
        onSelect(nodeId)
        return
      }
      case 'Enter': {
        event.preventDefault()
        onSelect(nodeId)
        onActivate?.(nodeId)
        return
      }
      case 'Escape': {
        if (connectFrom === null) return
        event.preventDefault()
        beginConnect(null)
        announce('Connection cancelled.')
        return
      }
      case 'c':
      case 'C': {
        event.preventDefault()
        if (readOnly) return announce('This project folder is read-only.')
        if (connectFrom === null) {
          // The spare is the last resort rather than the first: a writer
          // pressing C on a scene that already has an unfinished exit means
          // that one, not a second one beside it.
          // A structural port is skipped rather than offered: a beat leads to
          // its own choices because they belong to it, so "connect from here"
          // would be a gesture with nothing behind it.
          const authored =
            (here.element ? unresolvedPorts(here.element)[0] : undefined) ??
            here.ports.find((port) => !port.fixed)
          const port = authored ?? here.spare
          if (!port) return announce(`${here.title} has no way out to connect.`)
          // A label travels only with a port that does not exist yet: it is the
          // only name `connectPort` will have to author it under. An existing
          // port already has one, and sending it again would say nothing.
          beginConnect(
            authored
              ? { elementId: nodeId, portId: port.id }
              : { elementId: nodeId, portId: port.id, label: port.label },
          )
          announce(`Connecting “${port.label}” from ${here.title}. Move to a node and press C.`)
          return
        }
        if (connectFrom.elementId === nodeId) {
          beginConnect(null)
          return announce('Connection cancelled.')
        }
        const made = onConnect(connectFrom, nodeId)
        beginConnect(null)
        if (made) announce(`Connected to ${here.title}.`)
        return
      }
      case 'Delete':
      case 'Backspace': {
        event.preventDefault()
        if (readOnly) return announce('This project folder is read-only.')
        // Focus has to land somewhere that still exists, or the reader is
        // dropped back at the top of the document with no idea what happened.
        const survivor = predecessors[0] ?? successors[0] ?? null
        if (!onDelete(nodeId) || canonicalDeletion) return
        announce(`Deleted ${here.title}.`)
        if (survivor !== null) requestAnimationFrame(() => focus(survivor))
        return
      }
      default:
    }
  }
}
