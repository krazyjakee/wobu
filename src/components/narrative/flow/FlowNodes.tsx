import { isFlowElementMuted } from './flowFilters'
import { memo, type ReactNode } from 'react'
import { Handle, Position, type NodeProps, type Node } from '@xyflow/react'
import { useUI } from '../../../store/ui'
import { Icon } from '../../Icon'
import { NARRATIVE_STATUS } from '../narrativeModel'
import type { FlowGraphNode } from './graph'
import { useFlowLevel } from './flowStore'
import { badgeRows } from './badges'
import {
  FLOW_KIND_ICON,
  FLOW_KIND_LABEL,
  type FlowEffect,
  type FlowElement,
  type FlowPort,
} from './model'

/**
 * The six node kinds and the two container faces, drawn.
 *
 * Three rules run through all of them:
 *
 * - **Nothing here reads the scene.** A node is given its `FlowGraphNode` once,
 *   in `data`, and that object only changes when the *graph* changes. Selection
 *   is read from the store instead, so clicking a node re-renders that node
 *   rather than rewriting all 300 node objects — the direct consequence of the
 *   spike's 83 ms-per-click measurement at 1,000 nodes.
 * - **Fixed size.** Every face fits the box `NODE_SIZE` promises the layout
 *   engine. A beat that grew with its line count would move every other box on
 *   the canvas every time a Build finished.
 * - **Never colour alone.** Every status, cap and badge carries a word and a
 *   glyph. The colours come from tokens, so both themes follow for free.
 */

export type FlowNodeData = { node: FlowGraphNode }
export type FlowRFNode = Node<FlowNodeData>

/**
 * Whether the filters currently hide this node.
 *
 * Computed inside the node from two store subscriptions rather than pushed in
 * through `data`, for the same reason selection is: changing a filter must not
 * rewrite three hundred node objects.
 */
/** The chip a port row wears when something gates it, or when it is broken. */
function PortRow({ port, broken }: { port: FlowPort; broken?: string }) {
  return (
    <li className="nrt-port">
      <span className="nrt-port-label">{port.label}</span>
      {broken && (
        /* The badge on the wire itself. A destination diagnostic is about the
           way *out*, and #189 asks for it on the edge rather than only on the
           box — this is the row that edge leaves from. */
        <span className="nrt-chip is-bad" title={broken}>
          <Icon name="x" size="sm" />
          broken
        </span>
      )}
      {port.requires && (
        // On the port row, not on the wire: a requirement that floats on an
        // edge is a requirement that moves every time the graph is laid out.
        <span className="nrt-chip">
          <Icon name="lock" size="sm" />
          requires {port.requires}
        </span>
      )}
      {port.to === null && (
        <span className="nrt-chip is-bad">
          <Icon name="x" size="sm" />
          no destination
        </span>
      )}
    </li>
  )
}

/**
 * The frame every node shares: caps, header, body, ports.
 *
 * One shell rather than six similar components. The kinds differ in their body
 * and their glyph, and nothing else — and six copies of the handle wiring is
 * six places for a keyboard or an aria bug to hide in exactly one of them.
 */
function NodeShell({
  node,
  children,
  tone,
}: {
  node: FlowGraphNode
  children?: ReactNode
  /** A modifier class, so a kind can be told apart by shape as well as tint. */
  tone: string
}) {
  const selected = useFlowLevel((s) => s.selectedId === node.id)
  const connecting = useFlowLevel((s) => s.connectFrom?.elementId === node.id)
  const participant = useFlowLevel((s) => s.participant)
  const statusFilters = useUI((s) => s.narrativeFilters)
  const status = node.element?.status ? NARRATIVE_STATUS[node.element.status] : null
  const ports = node.ports
  // The spare is a handle like any other, so it is laid out with the rest: a
  // separate absolute position would drift the moment a port was added.
  const handles = node.spare ? [...ports, node.spare] : ports
  const muted = isFlowElementMuted(node.element, node.blocking, participant, statusFilters)
  const brokenPorts = new Map<string, string>()
  for (const found of node.element?.diagnostics ?? []) {
    // Errors only, and errors are never filtered: a wire that cannot be taken
    // has to stay marked whatever chips are on.
    if (found.portId && found.severity === 'error') brokenPorts.set(found.portId, found.message)
  }

  const classes = ['nrt-node', `is-${tone}`]
  if (selected) classes.push('is-selected')
  if (connecting) classes.push('is-connecting')
  if (muted) classes.push('is-muted')

  return (
    <div className={classes.join(' ')} data-testid={`flow-node-${node.id}`}>
      {/* The caps sit above the box because they are facts about the box's
          place in the graph rather than about its content. Reconvergence gets
          one of its own: in a layered drawing the only other signal is several
          lines ending at one box, which is the first signal a dense scene
          destroys, so it is said in words and a count. */}
      <div className="nrt-node-caps">
        {node.entry && (
          <span className="nrt-node-cap is-entry">
            <Icon name="chev" size="sm" />
            Scene start
          </span>
        )}
        {node.inbound >= 2 && (
          <span className="nrt-node-cap is-join">
            <Icon name="layers" size="sm" />
            {node.inbound} routes in
          </span>
        )}
        {muted && (
          /* Said, not only dimmed. A filter that answers with nothing but
             opacity is indistinguishable from a rendering fault. */
          <span className="nrt-node-cap is-muted">
            <Icon name="minus" size="sm" />
            Filtered out
          </span>
        )}
      </div>

      {/* One target handle, whatever the kind: everything has exactly one way in. */}
      <Handle type="target" position={Position.Top} className="nrt-handle" />

      <header className="nrt-node-head">
        <Icon name={node.kind === 'group' ? 'folder' : FLOW_KIND_ICON[node.kind]} size="sm" />
        <span className="nrt-node-kind">
          {node.kind === 'group' ? 'Group' : FLOW_KIND_LABEL[node.kind]}
        </span>
        {status && (
          <span className="nrt-badge">
            <Icon name={status.icon} size="sm" />
            {status.label}
          </span>
        )}
      </header>
      <h4 className="nrt-node-title">{node.title}</h4>
      <div className="nrt-node-body">{children}</div>

      <WorkCounts element={node.element} />
      <DiagnosticBadges element={node.element} />

      {handles.length > 0 && (
        <ul className="nrt-ports">
          {ports.map((port) => (
            <PortRow key={port.id} port={port} broken={brokenPorts.get(port.id)} />
          ))}
          {node.spare && (
            /* Not a hole: nothing is missing here. It is the affordance that
               turns "drag from this box to that one" into an authored exit,
               and without it a node born with no ports could never gain one. */
            <li className="nrt-port is-spare">
              <span className="nrt-port-label">{node.spare.label}</span>
            </li>
          )}
        </ul>
      )}

      {handles.map((port, index) => (
        <Handle
          key={port.id}
          id={port.id}
          type="source"
          position={Position.Bottom}
          className={port.fixed ? 'nrt-handle is-fixed' : 'nrt-handle'}
          // A structural port draws its edge and refuses to start a new one: a
          // beat leads to its own choices because they belong to it, so there
          // is no wire here for a pointer to pick up and move.
          isConnectable={!port.fixed}
          // Spread across the bottom edge in authored order, so a choice's
          // options leave the box left to right in the order they are listed.
          style={{ left: `${((index + 1) / (handles.length + 1)) * 100}%` }}
        />
      ))}
    </div>
  )
}

/**
 * The three lifecycle dimensions, side by side and never summed.
 *
 * Four independent counts from `Text.lifecycle`, each with its own word and
 * glyph. There is deliberately no single "state of this beat": a locked line
 * whose context moved is locked *and* out of date, and a badge that picked one
 * would hide the mismatch US-06 requires stays visible. Zeroes are left out —
 * a row of four of them reads as noise and buries the beat that has a four.
 */
function WorkCounts({ element }: { element?: FlowElement }) {
  const counts = element?.counts
  if (!counts) return null
  const rows = (
    [
      ['needsText', counts.needsText],
      ['needsReview', counts.needsReview],
      ['outOfDate', counts.outOfDate],
      ['locked', counts.locked],
    ] as const
  ).filter(([, value]) => value > 0)
  if (rows.length === 0) return null
  return (
    <ul className="nrt-scene-counts">
      {rows.map(([status, value]) => (
        <li key={status} className="nrt-badge">
          <Icon name={NARRATIVE_STATUS[status].icon} size="sm" />
          {value} {NARRATIVE_STATUS[status].label.toLocaleLowerCase()}
        </li>
      ))}
    </ul>
  )
}

/**
 * What the backend says is wrong with this element, as words and a count.
 *
 * Subscribed to the filter here rather than fed through `data`, for the same
 * reason selection is: changing a chip must not rewrite three hundred node
 * objects. `badgeShown` is what keeps a release-blocking error visible whatever
 * the chips say, and it does so before it looks at them.
 */
function DiagnosticBadges({ element }: { element?: FlowElement }) {
  const filter = useFlowLevel((s) => s.badges)
  const rows = badgeRows(element?.diagnostics, filter)
  if (rows.length === 0) return null
  return (
    <ul className="nrt-node-badges" aria-label="Diagnostics">
      {rows.map((row) => (
        <li
          key={row.id}
          className={row.severity === 'error' ? 'nrt-badge is-bad' : 'nrt-badge is-warn'}
          title={row.detail}
        >
          <Icon name={row.icon} size="sm" />
          {row.label}
        </li>
      ))}
    </ul>
  )
}

/** A beat: one node, forever. Counts of lines and variants, never the text. */
export const BeatNode = memo(function BeatNode({ data }: NodeProps<FlowRFNode>) {
  const beat = data.node.element
  const detail = beat?.kind === 'beat' ? beat : null
  return (
    <NodeShell node={data.node} tone="beat">
      {detail && detail.participants.length > 0 && (
        <p className="nrt-node-line">{detail.participants.join(' · ')}</p>
      )}
      {detail && (
        /* The whole of what this canvas knows about a beat's text. Twelve
           lines and seven variants is one box; the lines live in Script. */
        <p className="nrt-node-line is-count">
          {detail.lines} line{detail.lines === 1 ? '' : 's'} · {detail.variants} variant
          {detail.variants === 1 ? '' : 's'}
        </p>
      )}
    </NodeShell>
  )
})

/**
 * A choice. Its way out is its port, so the shell already drew that; what it
 * adds is what taking it *does*, which the source model records on the choice
 * itself and a canvas that hid it would be hiding half the decision.
 */
export const ChoiceNode = memo(function ChoiceNode({ data }: NodeProps<FlowRFNode>) {
  const element = data.node.element
  return (
    <NodeShell node={data.node} tone="choice">
      <EffectRows effects={element?.kind === 'choice' ? element.effects : []} />
    </NodeShell>
  )
})

/** A branch the player never sees. Exactly two ways out. */
export const ConditionNode = memo(function ConditionNode({ data }: NodeProps<FlowRFNode>) {
  const element = data.node.element
  return (
    <NodeShell node={data.node} tone="condition">
      {element?.kind === 'condition' && <p className="nrt-node-line is-code">{element.test}</p>}
    </NodeShell>
  )
})

/** How many effects fit on an outcome before it starts counting instead. */
const EFFECTS_SHOWN = 3

/**
 * The rows an outcome or a choice shows.
 *
 * Keyed by position rather than by variable name: two effects on one variable
 * are legal — `support +10` then `support = 0` — and a duplicate React key
 * would silently drop the second.
 */
function EffectRows({ effects }: { effects: readonly FlowEffect[] }) {
  if (effects.length === 0) return null
  return (
    <ul className="nrt-effects">
      {effects.slice(0, EFFECTS_SHOWN).map((effect, index) => (
        <li key={index} className="nrt-node-line is-code">
          {effect.variable} {effect.operator}
          {effect.operator === '=' ? ' ' : ''}
          {effect.value}
        </li>
      ))}
      {effects.length > EFFECTS_SHOWN && (
        <li className="nrt-node-line is-count">and {effects.length - EFFECTS_SHOWN} more</li>
      )}
    </ul>
  )
}

export const OutcomeNode = memo(function OutcomeNode({ data }: NodeProps<FlowRFNode>) {
  const element = data.node.element
  return (
    <NodeShell node={data.node} tone="outcome">
      <EffectRows effects={element?.kind === 'outcome' ? element.effects : []} />
    </NodeShell>
  )
})

/** Terminal. */
export const EndNode = memo(function EndNode({ data }: NodeProps<FlowRFNode>) {
  return <NodeShell node={data.node} tone="end" />
})

/** Leaves the scene. Terminal here; the arc view is where it joins up. */
export const SceneLinkNode = memo(function SceneLinkNode({ data }: NodeProps<FlowRFNode>) {
  const element = data.node.element
  return (
    <NodeShell node={data.node} tone="scenelink">
      {element?.kind === 'sceneLink' && (
        <p className="nrt-node-line is-code">
          <span aria-hidden>→ </span>
          {element.targetSceneId ?? 'no scene chosen'}
        </p>
      )}
    </NodeShell>
  )
})

/**
 * A closed group.
 *
 * The primary way this canvas stays inside its budget: everything inside is
 * gone from the node array, not merely hidden, and the edges that crossed the
 * boundary now end here. The counts are the whole of what a writer gets to
 * judge whether to open it.
 */
/**
 * A whole scene, at the arc level.
 *
 * Counts and nothing else — no beat titles, no lines. The rule that keeps a
 * beat one node keeps a scene one node here, one zoom level up: a scene that
 * grew a row per beat would put the 1,000-scene arc back over the budget the
 * collapse exists to hold it under, and the beats are one double-click away.
 *
 * The three work counts are the reason a designer opens this view at all, so
 * each is a badge with a word and a glyph, and only shown when it is not zero —
 * a row of three zeroes reads as noise and hides the one scene that has a four.
 */
export const SceneNode = memo(function SceneNode({ data }: NodeProps<FlowRFNode>) {
  const element = data.node.element
  const scene = element?.kind === 'scene' ? element : null
  return (
    <NodeShell node={data.node} tone="scene">
      {scene && scene.participants.length > 0 && (
        <p className="nrt-node-line">{scene.participants.join(' · ')}</p>
      )}
      {scene && (
        <p className="nrt-node-line is-count">
          {scene.counts.beats} beat{scene.counts.beats === 1 ? '' : 's'}
        </p>
      )}
      {/* The rolled-up counts are drawn by the shell, from the same `counts`
          field a beat carries — so a scene's three numbers and a beat's are one
          component rather than two that drift. */}
    </NodeShell>
  )
})

/**
 * Where an unresolved destination goes.
 *
 * Drawn, rather than left as an edge that simply is not there. Inside a scene
 * the unset port is enough — the box with the hole in it is on screen. Across
 * an arc it is not: "this quest ends here" and "nothing links to the scene that
 * finishes this quest" look identical unless the second one is given a wire and
 * a name.
 */
export const MissingNode = memo(function MissingNode({ data }: NodeProps<FlowRFNode>) {
  const element = data.node.element
  return (
    <NodeShell node={data.node} tone="missing">
      {element?.kind === 'missing' && (
        <p className="nrt-node-line is-code">
          {element.targetId === null
            ? 'No destination chosen'
            : `${element.targetId} is not in this arc`}
        </p>
      )}
    </NodeShell>
  )
})

export const GroupNode = memo(function GroupNode({ data }: NodeProps<FlowRFNode>) {
  const toggle = useFlowLevel((s) => s.toggleGroup)
  const counts = data.node.group
  return (
    <NodeShell node={data.node} tone="group">
      <p className="nrt-node-line is-count">
        {counts?.elements ?? 0} elements · {counts?.crossings ?? 0} edges cross
      </p>
      <button type="button" className="btn btn-sm" onClick={() => toggle(data.node.id)}>
        Open group
      </button>
    </NodeShell>
  )
})

/**
 * The frame drawn behind an open group.
 *
 * Not a React Flow parent node. Parenting would buy containment and cost
 * parent-relative coordinates, extent clamping and drag-to-reparent on every
 * node inside it; the frame is derived from the bounds of its members instead,
 * so it is presentation and nothing depends on it.
 */
export const GroupFrameNode = memo(function GroupFrameNode({ data }: NodeProps<FlowRFNode>) {
  const toggle = useFlowLevel((s) => s.toggleGroup)
  return (
    <div className="nrt-frame" style={{ width: '100%', height: '100%' }}>
      <div className="nrt-frame-head">
        <Icon name="folder" size="sm" />
        <span>{data.node.title}</span>
        <button type="button" className="btn btn-sm" onClick={() => toggle(data.node.id)}>
          Close group
        </button>
      </div>
    </div>
  )
})
