import { ParticipantFilter, FlowStatusFilters } from './ParticipantFilter'
import { useFlowGroupPresentation } from './useFlowGroupPresentation'
import { useFlowReveal } from './useFlowReveal'
import type { CanonicalFlowActions } from './canonicalFlow'
import type { FlowPresentation } from './useFlowPresentation'
import { PresentationTools, PinnedNotes } from './PresentationTools'
import { layoutKeyOf } from './source'
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import {
  Background,
  Controls,
  MarkerType,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Connection,
  type Edge,
  type NodeChange,
} from '@xyflow/react'
import '@xyflow/react/dist/base.css'
import { Icon } from '../../Icon'
import type { FlowGroup } from './model'
import {
  buildGraph,
  nodeLabel,
  visibleNodeCount,
  NODE_BUDGET,
  type FlowGraph,
  type FlowGraphNode,
} from './graph'
import {
  BeatNode,
  ChoiceNode,
  ConditionNode,
  EndNode,
  GroupFrameNode,
  GroupNode,
  MissingNode,
  OutcomeNode,
  SceneLinkNode,
  SceneNode,
  type FlowRFNode,
} from './FlowNodes'
import { useFlowLevel, useFlowLevelApi } from './flowStore'
import { RouteLegend } from './PreviewOverlay'
import { useRouteCentring, useRouteTranscriptCursor } from './overlay'
import { FlowBadgeFilters } from './FlowBadgeFilters'
import { useSceneEdits, type FlowAuthoring } from './useSceneEdits'
import {
  createLayoutGate,
  elkLayout,
  layoutRequest,
  seedPositions,
  NODE_SIZE,
  type LayoutRunner,
  type XY,
} from './layout'
import { useFlowKeyboard } from './useFlowKeyboard'
import {
  FLOW_KIND_LABEL,
  sceneDiagnostics,
  type FlowDiagnostic,
  type FlowElement,
  type FlowKind,
  type FlowLevel,
  type FlowPort,
  type FlowPositions,
} from './model'

/**
 * The scene Flow canvas.
 *
 * A second editor over one narrative model, never a second copy of it. The
 * scene comes in as a prop and every change goes back out through `onChange`,
 * so whoever owns the model — a store today, Tauri commands later — is the only
 * thing that decides what a scene is.
 *
 * The four decisions worth knowing before changing anything here:
 *
 * 1. **Nodes are bounded before they reach the `nodes` prop.** `buildGraph`
 *    collapses and, if necessary, keyholes. React Flow's own store is what
 *    grows superlinearly, so hiding nodes it has already been given would be no
 *    saving at all.
 * 2. **Selection is not in `nodes`.** It is in `flowStore`, node components
 *    subscribe to it, and `onNodesChange` deliberately drops React Flow's own
 *    `select` changes on the floor. Pushing `selected` through the array was
 *    measured at 83 ms a click on 1,000 nodes.
 * 3. **Two layouts.** A synchronous seed on every structural change so nothing
 *    ever sits at the origin, and elk in a worker on demand for the good
 *    drawing. elk has no cancellation, so a superseded result is discarded by
 *    ticket (`createLayoutGate`).
 * 4. **Positions never touch the scene.** They live in their own state and go
 *    out through `onPositionsChange`. A layout-only change therefore cannot
 *    produce a source, revision, freshness or compiled-output difference,
 *    because it physically cannot reach `FlowScene`.
 *
 * It draws **either level** (#187). A `FlowLevel` is a scene of beats or an arc
 * of scenes, and everything above is true of both — so the arc gets the same
 * bounded node array, the same off-thread layout, the same keyboard layer and
 * the same measured selection cost, rather than a second canvas that would have
 * to be held to all four rules again by hand. What the two levels differ in is
 * handed in as props: which kinds can be created, how the level is grouped,
 * whether an unresolved destination is drawn as a wire, and what happens when a
 * writer goes *into* a node.
 */

const CREATABLE: FlowKind[] = ['beat', 'choice', 'condition', 'outcome', 'end']

/**
 * Module scope, and never rebuilt. A fresh `nodeTypes` object on a render
 * remounts every node on the canvas, which React Flow warns about and which
 * would throw away the measurements and the focus with them.
 */
const NODE_TYPES = {
  beat: BeatNode,
  choice: ChoiceNode,
  condition: ConditionNode,
  outcome: OutcomeNode,
  end: EndNode,
  sceneLink: SceneLinkNode,
  scene: SceneNode,
  missing: MissingNode,
  group: GroupNode,
  groupFrame: GroupFrameNode,
}

export interface FlowCanvasProps {
  scene: FlowLevel
  onChange: (scene: FlowLevel) => void
  readOnly?: boolean
  /** Stored node coordinates (#185). Absent ids fall back to the seed layout. */
  positions?: FlowPositions
  presentation?: FlowPresentation
  /** Where moved and laid-out coordinates go. Nothing here persists them. */
  onPositionsChange?: (positions: FlowPositions) => void
  /** Swapped for a synchronous fake in tests; the real one starts a worker. */
  layout?: LayoutRunner

  // ── what differs between the two levels ─────────────────────────────────

  /** Which kinds the toolbar offers to create. Defaults to the scene's six. */
  creatable?: readonly FlowKind[]
  /** Extra controls, rendered in the toolbar before the count. */
  toolbar?: ReactNode
  /** Regroup without editing: see `buildGraph`'s `grouping`. */
  grouping?: { groups: readonly FlowGroup[]; of: (element: FlowElement) => string | null }
  /** Draw an unresolved destination as a wire into a named box. */
  dangling?: boolean
  /** The unauthored way out this level offers. See `FlowGraphNode.spare`. */
  spare?: (element: FlowElement) => FlowPort | null
  /** Recomputed by the caller when a level has diagnostics of its own. */
  diagnostics?: FlowDiagnostic[]
  /**
   * What this level allows, and the sentence for each refusal.
   *
   * Absent means "anything goes", which is what a fixture wants. A canvas
   * drawing a real scene document hands in the four rules the source model
   * actually imposes — see `flowAuthoring` in `source.ts`.
   */
  authoring?: FlowAuthoring
  actions?: CanonicalFlowActions
  /** Go into a node: double-click, or Enter. The arc's drill-down. */
  onActivate?: (id: string) => void
  /** What one node is, in the keyhole banner and the minimap's name. */
  noun?: string
  /** The shared narrative path a selected element means. See `useSceneEdits`. */
  targetOf?: FlowCanvasTargetOf
}

type FlowCanvasTargetOf = Parameters<typeof useSceneEdits>[0]['targetOf']

export function FlowCanvas(props: FlowCanvasProps) {
  // React Flow's imperative API is only available under its provider, and
  // centring on a reveal needs it.
  return (
    <ReactFlowProvider>
      <Canvas {...props} />
    </ReactFlowProvider>
  )
}

function Canvas({
  scene,
  onChange,
  readOnly = false,
  positions: stored,
  presentation,
  onPositionsChange,
  layout = elkLayout,
  creatable = CREATABLE,
  toolbar,
  grouping,
  dangling = false,
  spare,
  diagnostics: given,
  onActivate,
  noun = 'element',
  targetOf,
  authoring,
  actions,
}: FlowCanvasProps) {
  const container = useRef<HTMLDivElement>(null)
  const flow = useReactFlow()
  const gate = useRef(createLayoutGate())
  const [positions, setPositions] = useState<Record<string, XY>>(() => ({ ...stored }))
  const [autoPositions, setAutoPositions] = useState<Record<string, XY>>({})
  const [seenPositions, setSeenPositions] = useState(stored)
  const [dragging, setDragging] = useState(false)
  const presentationLevel = presentation?.layout.graph.kind === 'scene' ? 'scene' : 'arc'
  const automatic = presentation?.layout.mode === 'automatic'
  const persistent = !!presentation
  if (stored !== seenPositions && !dragging) {
    setSeenPositions(stored)
    setPositions({ ...stored })
  }
  const [laying, setLaying] = useState(false)
  const [layoutError, setLayoutError] = useState<string | null>(null)
  const [settledLayout, setSettledLayout] = useState<string | null>(null)

  const store = useFlowLevelApi()
  /*
   * Where this level was when it was last on screen.
   *
   * Read once, on mount, and deliberately not subscribed to: React Flow owns
   * the viewport while the canvas is up, and feeding a changing value back into
   * `defaultViewport` would fight the writer's own panning. Leaving a scene and
   * returning to the arc remounts this component against the same level store,
   * which is what makes the arc come back where it was left.
   */
  const [openedAt] = useState(() => store.getState().viewport)
  const selectedId = useFlowLevel((s) => s.selectedId)
  const closedGroups = useFlowLevel((s) => s.closedGroups)
  const announcement = useFlowLevel((s) => s.announcement)
  const setClosedGroups = useFlowLevel((s) => s.setClosedGroups)
  // A regrouped level's closed set names its own containers rather than the
  // sidecar's, so the sidecar is left alone while one is on. See the hook.
  useFlowGroupPresentation(presentation, readOnly, !!grouping)
  const sourceDisabled = readOnly || !!actions?.disabled
  const { select, connect, remove, add } = useSceneEdits({
    scene,
    onChange,
    readOnly: sourceDisabled,
    targetOf,
    authoring,
    actions,
  })

  /*
   * The keyhole is centred on the selection — but only when there *is* a
   * keyhole. Feeding `selectedId` into `buildGraph` unconditionally would
   * rebuild every node object on every click, which is the one thing this
   * canvas is not allowed to do.
   */
  const groupOf = grouping?.of
  const over = visibleNodeCount(scene, closedGroups, groupOf) > NODE_BUDGET
  const focusId = over ? selectedId : null
  const ownDiagnostics = useMemo(() => sceneDiagnostics(scene), [scene])
  const diagnostics = given ?? ownDiagnostics
  /*
   * Blocking node ids, recomputed with the diagnostics rather than stored.
   *
   * They travel into the graph so a *filter* can be forbidden from muting them:
   * #187 requires that filtering never hides a release-blocking diagnostic, and
   * the only place that can be guaranteed is on the node itself.
   */
  const blockingIds = useMemo(
    () => diagnostics.filter((d) => d.severity === 'error').map((d) => d.elementId),
    [diagnostics],
  )
  const graph = useMemo(
    () => buildGraph(scene, { closedGroups, focusId, grouping, dangling, spare, blockingIds }),
    [scene, closedGroups, focusId, grouping, dangling, spare, blockingIds],
  )
  const layoutInput = useMemo(() => layoutRequest(graph), [graph])
  // Text and diagnostic refreshes replace graph objects without moving boxes.
  // Only a geometry change should launch ELK or postpone a pending reveal.
  const layoutIdentity = JSON.stringify(layoutInput)

  const seed = useMemo(() => seedPositions(graph), [graph])
  const placed = useMemo(() => {
    const merged: Record<string, XY> = {}
    for (const node of graph.nodes)
      merged[node.id] =
        (automatic && !dragging ? autoPositions[node.id] : positions[node.id]) ??
        seed[node.id] ??
        ZERO
    return merged
  }, [graph, positions, seed, automatic, autoPositions, dragging])

  const center = useCallback(
    (id: string) => {
      const node = graph.nodes.find((node) => node.id === id)
      const at = placed[id]
      if (node && at) {
        const size = NODE_SIZE[node.kind]
        void flow.setCenter(at.x + size.width / 2, at.y + size.height / 2, {
          duration: 0,
          zoom: flow.getZoom(),
        })
      }
    },
    [graph, placed, flow],
  )
  useFlowReveal({
    scene,
    container,
    projectKey: actions?.projectKey,
    graph,
    center,
    enabled: !automatic || settledLayout === layoutIdentity,
  })
  // The other half of the Preview cursor: a trace step asking for its node.
  useRouteCentring(center)
  const pointTranscript = useRouteTranscriptCursor()

  const runLayout = useCallback(() => {
    const ticket = gate.current.begin()
    setLaying(true)
    setLayoutError(null)
    layout(layoutInput)
      .then((result) => {
        // elkjs cannot be cancelled, so a superseded answer is simply dropped
        // here. Applying it would move the boxes of a graph that no longer
        // exists, on top of a newer layout that was right.
        if (!gate.current.accept(ticket)) return
        setLaying(false)
        setSettledLayout(layoutIdentity)
        if (automatic) setAutoPositions(result.positions)
        else {
          setPositions((previous) => ({ ...previous, ...result.positions }))
          if (!readOnly) onPositionsChange?.(result.positions)
        }
        if (!automatic) window.setTimeout(() => flow.fitView({ duration: 200 }), 0)
      })
      .catch((error: unknown) => {
        if (!gate.current.accept(ticket)) return
        setLaying(false)
        setSettledLayout(layoutIdentity)
        setLayoutError(error instanceof Error ? error.message : String(error))
      })
  }, [layoutInput, layoutIdentity, layout, flow, onPositionsChange, automatic, readOnly])

  const automaticRunner = useRef(runLayout)
  useEffect(() => {
    automaticRunner.current = runLayout
  }, [runLayout])
  useEffect(() => {
    if (automatic) automaticRunner.current()
  }, [automatic, layoutIdentity])

  // A scene swap abandons whatever elk is still chewing on.
  const sceneId = scene.id
  useEffect(() => {
    const current = gate.current
    return () => current.abandon()
  }, [sceneId])

  // ── React Flow's own arrays ─────────────────────────────────────────────

  const rfNodes = useMemo<FlowRFNode[]>(() => {
    const nodes: FlowRFNode[] = graph.nodes.map((node) => ({
      id: node.id,
      type: node.kind,
      position: placed[node.id] ?? ZERO,
      data: { node },
      draggable: !readOnly && (!persistent || layoutKeyOf(node.id, presentationLevel) !== null),
      ariaLabel: nodeLabel(node),
      style: NODE_SIZE[node.kind],
      width: NODE_SIZE[node.kind].width,
      height: NODE_SIZE[node.kind].height,
    }))
    return [...frames(graph, placed, closedGroups, NODE_BUDGET - nodes.length), ...nodes]
  }, [graph, placed, readOnly, closedGroups, persistent, presentationLevel])

  const rfEdges = useMemo<Edge[]>(
    () =>
      graph.edges.map((edge) => ({
        id: edge.id,
        source: edge.source,
        sourceHandle: edge.sourceHandle,
        target: edge.target,
        // A long wire climbing sixty ranks is unreadable however it is routed,
        // so a back edge is dashed and says where it goes, near where it leaves.
        className: edge.back ? 'nrt-edge is-back' : 'nrt-edge',
        label: edge.back ? `back to ${title(graph.nodes, edge.target)}` : undefined,
        markerEnd: { type: MarkerType.ArrowClosed },
      })),
    [graph],
  )

  const onNodesChange = useCallback((changes: NodeChange<FlowRFNode>[]) => {
    // Position only. React Flow's own `select` and `remove` changes are dropped
    // on purpose: selection lives in `flowStore` and deletion goes through the
    // scene, and letting either through here would rewrite the node array.
    setPositions((previous) => {
      let next = previous
      for (const change of changes) {
        if (change.type !== 'position' || !change.position) continue
        if (next === previous) next = { ...previous }
        next[change.id] = change.position
      }
      return next
    })
  }, [])

  const onDragStop = useCallback(
    (_event: unknown, node: FlowRFNode) => {
      setDragging(false)
      if (presentation) {
        const key = layoutKeyOf(node.id, presentationLevel)
        if (!key) return
        const now = new Date().toISOString()
        const base = presentation.layout
        presentation.onChange({
          ...base,
          mode: 'manual',
          modeUpdatedAt: now,
          nodes: { ...base.nodes, [key]: { ...base.nodes[key], ...node.position, updatedAt: now } },
        })
      } else onPositionsChange?.({ [node.id]: node.position })
    },
    [onPositionsChange, presentation, presentationLevel],
  )

  const onKeyDown = useFlowKeyboard({
    graph,
    container,
    readOnly: sourceDisabled,
    onConnect: (from, to) => connect(from, to),
    onDisconnect: (from) => connect(from, null),
    onDelete: remove,
    canonicalDeletion: !!actions,
    onSelect: select,
    onActivate,
  })

  const onPointerConnect = useCallback(
    (connection: Connection) => {
      if (!connection.sourceHandle) return
      // A drag from the *spare* handle is an exit being authored, so the label
      // has to travel with it — `connectPort` has nothing else to name the new
      // port after.
      const from = graph.nodes.find((node) => node.id === connection.source)
      const label = from?.spare?.id === connection.sourceHandle ? from.spare.label : undefined
      connect(
        { elementId: connection.source, portId: connection.sourceHandle, label },
        connection.target,
      )
    },
    [connect, graph],
  )

  const unresolved = diagnostics.filter((d) => d.severity === 'error').length

  return (
    <div className="nrt-flow-wrap">
      <div className="nrt-flow-bar" role="toolbar" aria-label="Flow canvas actions">
        {/* Not disabled while it runs. A writer who moves a box and asks
            again is asking for a newer answer, and the gate is what makes the
            older one harmless — see `createLayoutGate`. */}
        {presentation && (
          <PresentationTools
            presentation={presentation}
            level={presentationLevel}
            readOnly={readOnly}
            nodePositions={placed}
            notePosition={() =>
              flow.screenToFlowPosition({ x: window.innerWidth / 2, y: window.innerHeight / 2 })
            }
          />
        )}
        <button type="button" className="btn btn-sm" onClick={runLayout}>
          {laying ? 'Laying out…' : 'Auto layout'}
        </button>
        <button
          type="button"
          className="btn btn-sm"
          onClick={() => flow.fitView({ duration: 200 })}
        >
          Fit
        </button>
        <span className="nrt-bar-sep" aria-hidden />
        {creatable.map((kind) => (
          <button
            key={kind}
            type="button"
            className="btn btn-sm"
            disabled={sourceDisabled}
            onClick={() => add(kind, selectedId)}
          >
            Add {FLOW_KIND_LABEL[kind].toLocaleLowerCase()}
          </button>
        ))}
        <span className="nrt-bar-sep" aria-hidden />
        <button
          type="button"
          className="btn btn-sm"
          onClick={() =>
            setClosedGroups(closedGroups.length > 0 ? [] : graph.groups.map((group) => group.id))
          }
          disabled={graph.groups.length === 0}
        >
          {closedGroups.length > 0 ? 'Open all groups' : 'Close all groups'}
        </button>
        <ParticipantFilter scene={scene} />
        <FlowStatusFilters />
        {/* Only when there is something to explain. A legend for a canvas with
            no badges on it is a control that answers a question nobody asked. */}
        {scene.elements.some((element) => element.diagnostics || element.counts) && (
          <FlowBadgeFilters />
        )}
        {/* Only when a run has been played: see `RouteLegend`. */}
        <RouteLegend />
        {toolbar}
        <span className="nrt-bar-count">
          {graph.nodes.length} of {graph.total} on the canvas
          {graph.hidden > 0 ? ` · ${graph.hidden} folded away` : ''}
        </span>
      </div>

      {graph.keyhole && (
        <p className="nrt-note" role="status">
          <Icon name="layers" size="sm" />
          This has {graph.total} {noun}s, past the {NODE_BUDGET}-node budget even with every group
          closed. Only the neighbourhood of the selection is drawn. Use the outline list to read all
          of it.
        </p>
      )}
      {layoutError && (
        <p className="nrt-note inline-error" role="alert">
          Automatic layout failed: {layoutError}. The boxes are where you left them.
        </p>
      )}
      {unresolved > 0 && (
        <p className="nrt-note" role="status">
          <Icon name="x" size="sm" />
          {unresolved} problem{unresolved === 1 ? '' : 's'} to answer: {diagnostics[0]?.message}
        </p>
      )}

      {/* The key handler sits on the container rather than on each node: React
          Flow owns the focusable wrapper, so a listener inside our own node
          component would never see the event. */}
      <div className="nrt-flow-canvas" ref={container} onKeyDown={onKeyDown}>
        <ReactFlow<FlowRFNode>
          nodes={rfNodes}
          edges={rfEdges}
          nodeTypes={NODE_TYPES}
          onNodesChange={onNodesChange}
          onNodeDragStart={() => {
            setPositions(placed)
            setDragging(true)
            gate.current.abandon()
          }}
          onNodeDragStop={onDragStop}
          onNodeClick={(_event, node) => {
            select(node.id)
            // Leaves the tab alone; see `useRouteTranscriptCursor`.
            pointTranscript(node.id)
          }}
          // Double-click is the nested-flow gesture articy uses and #187 names.
          // The keyboard equivalent is Enter, in `useFlowKeyboard`.
          onNodeDoubleClick={(_event, node) => onActivate?.(node.id)}
          onConnect={onPointerConnect}
          onPaneClick={() => select(null)}
          nodesConnectable={!sourceDisabled}
          nodesDraggable={!readOnly}
          // Ours, not React Flow's: an edge is deleted by disconnecting the port
          // that made it, so the diagnostic can name the field.
          deleteKeyCode={null}
          // The backstop for panning inside the budget. Collapse is what keeps
          // the array small; this only keeps the DOM small.
          onlyRenderVisibleElements
          // Where this level was last left, so entering a scene and coming
          // back does not dump the writer at the origin. `fitView` only runs
          // when there is nothing to go back to.
          defaultViewport={openedAt ?? undefined}
          fitView={openedAt === null}
          onMoveEnd={(_event, viewport) => store.getState().setViewport(viewport)}
          minZoom={0.1}
          proOptions={{ hideAttribution: false }}
        >
          {presentation && (
            <PinnedNotes
              presentation={presentation}
              readOnly={readOnly}
              positions={Object.fromEntries(
                Object.entries(placed).flatMap(([id, at]) => {
                  const key = layoutKeyOf(id, presentationLevel)
                  return key ? [[key, at]] : []
                }),
              )}
            />
          )}
          <Background gap={24} />
          <Controls showInteractive={false} />
          <MiniMap pannable zoomable ariaLabel={`Map of every ${noun}`} />
        </ReactFlow>
      </div>

      {/* Everything the keyboard layer does is invisible without this. */}
      {/* Named, because the canvas has more than one polite region: the
          diagnostics line above is the other, and a test — or a reader
          arrowing through landmarks — needs to tell them apart. */}
      <div
        className="nrt-sr-only"
        role="status"
        aria-live="polite"
        aria-label="Flow canvas announcements"
      >
        <span key={announcement.seq}>{announcement.text}</span>
      </div>
    </div>
  )
}

const ZERO: XY = { x: 0, y: 0 }

/** How far a group frame stands off the boxes inside it. */
const FRAME_PAD = 20
const FRAME_HEAD = 28

function title(nodes: FlowGraphNode[], id: string): string {
  return nodes.find((node) => node.id === id)?.title ?? id
}

/**
 * A background rectangle per open group, sized from the boxes inside it.
 *
 * Derived from positions rather than parented, so nothing about a group's
 * members depends on the frame existing. It is drawn behind everything and
 * takes no pointer or keyboard focus of its own beyond its close button.
 */
function frames(
  graph: FlowGraph,
  placed: Record<string, XY>,
  closed: readonly string[],
  budget: number,
): FlowRFNode[] {
  const out: FlowRFNode[] = []
  for (const group of graph.groups) {
    if (out.length >= budget) break
    if (closed.includes(group.id)) continue
    const members = graph.nodes.filter((node) => node.groupId === group.id)
    if (members.length === 0) continue
    let left = Infinity
    let top = Infinity
    let right = -Infinity
    let bottom = -Infinity
    for (const member of members) {
      const at = placed[member.id] ?? ZERO
      const size = NODE_SIZE[member.kind]
      left = Math.min(left, at.x)
      top = Math.min(top, at.y)
      right = Math.max(right, at.x + size.width)
      bottom = Math.max(bottom, at.y + size.height)
    }
    out.push({
      id: `frame:${group.id}`,
      type: 'groupFrame',
      position: { x: left - FRAME_PAD, y: top - FRAME_PAD - FRAME_HEAD },
      data: {
        node: {
          id: group.id,
          kind: 'group',
          title: group.name,
          groupId: null,
          blocking: false,
          ports: [],
          inbound: 0,
          entry: false,
          beatId: null,
        },
      },
      draggable: false,
      selectable: false,
      focusable: false,
      zIndex: -1,
      style: {
        width: right - left + FRAME_PAD * 2,
        height: bottom - top + FRAME_PAD * 2 + FRAME_HEAD,
      },
    })
  }
  return out
}

/** Filter by who is in a beat. Muting, not removing: a hidden node breaks a route. */
