import type { FlowPresentation } from '../useFlowPresentation'
import { useEffect, useMemo, useState } from 'react'
import { Icon } from '../../../Icon'
import { FlowCanvas } from '../FlowCanvas'
import { FlowModeTabs, type FlowMode } from '../FlowModeTabs'
import { FlowOutline } from '../FlowOutline'
import { defaultClosedGroups } from '../graph'
import { FlowStoreContext, useFlowLevel } from '../flowStore'
import type { LayoutRunner } from '../layout'
import type { FlowKind, FlowLevel, FlowPositions } from '../model'
import type { FlowAuthoring } from '../useSceneEdits'
import { ARC_STORE } from './arcStore'
import {
  ARC_GROUPINGS,
  arcDiagnostics,
  arcGrouping,
  arcSpare,
  arcTarget,
  type ArcGrouping,
  type FlowArc,
} from './model'

/**
 * The quest and arc canvas: scenes as boxes, authored transitions as wires.
 *
 * It is the same `FlowCanvas` and the same `FlowOutline` the scene level uses,
 * handed the four things that differ — what can be created, how the level is
 * grouped, that an unresolved destination gets a wire of its own, and what
 * happens when a writer goes *into* a box. Everything the spike's budget
 * depends on is therefore shared rather than reimplemented: the node array is
 * bounded before React Flow sees it, selection stays out of that array, layout
 * runs off the main thread, and positions cannot reach the model.
 *
 * The level store it mounts is `ARC_STORE`, which outlives it on purpose: see
 * `arcStore.ts` for why that is the whole of how a round trip through a scene
 * comes back where it started.
 */

export function ArcFlow({
  arc,
  onChange,
  onEnter,
  readOnly = false,
  layout,
  positions,
  onPositionsChange,
  presentation,
  authoring,
  creatable = CREATABLE,
}: {
  arc: FlowArc
  onChange: (arc: FlowArc) => void
  /** Drill down: double-click, Enter, the outline's Open button, a diagnostic. */
  onEnter: (sceneId: string, beatId?: string | null) => void
  readOnly?: boolean
  /** Swapped for a synchronous fake in tests; the real one starts a worker. */
  layout?: LayoutRunner
  /** Stored node coordinates for this arc (#185). Persisted by nobody here. */
  positions?: FlowPositions
  onPositionsChange?: (positions: FlowPositions) => void
  presentation?: FlowPresentation
  /**
   * What this level allows, and the sentence for each refusal.
   *
   * A real project arc hands in a blanket refusal: a scene's exits are authored
   * on a beat *inside* it, so there is no field at this level for a wire to
   * write to, and a spare exit handle here would be a gesture with nothing
   * behind it. The demonstration arc hands in nothing and edits freely, because
   * nothing it does is written anywhere.
   */
  authoring?: FlowAuthoring
  /** Which kinds this level offers to create. Nothing, on a real project arc. */
  creatable?: readonly FlowKind[]
}) {
  return (
    <FlowStoreContext value={ARC_STORE}>
      <Arc
        arc={arc}
        onChange={onChange}
        onEnter={onEnter}
        readOnly={readOnly}
        layout={layout}
        positions={positions}
        onPositionsChange={onPositionsChange}
        presentation={presentation}
        authoring={authoring}
        creatable={creatable}
      />
    </FlowStoreContext>
  )
}

function Arc({
  arc,
  onChange,
  onEnter,
  readOnly,
  layout,
  positions,
  onPositionsChange,
  presentation,
  authoring,
  creatable,
}: {
  arc: FlowArc
  onChange: (arc: FlowArc) => void
  onEnter: (sceneId: string, beatId?: string | null) => void
  readOnly: boolean
  layout?: LayoutRunner
  positions?: FlowPositions
  onPositionsChange?: (positions: FlowPositions) => void
  presentation?: FlowPresentation
  authoring?: FlowAuthoring
  creatable: readonly FlowKind[]
}) {
  const [mode, setMode] = useState<FlowMode>('canvas')
  const [grouping, setGrouping] = useState<ArcGrouping>(arc.quests === null ? 'none' : 'quest')
  const closedGroups = useFlowLevel((s) => s.closedGroups)
  const setClosedGroups = useFlowLevel((s) => s.setClosedGroups)

  const diagnostics = useMemo(() => arcDiagnostics(arc), [arc])
  const groups = useMemo(
    () =>
      presentation
        ? {
            groups: arc.level.groups,
            of: (element: FlowLevel['elements'][number]) => element.groupId ?? null,
          }
        : arcGrouping(arc, grouping),
    [arc, grouping, presentation],
  )

  /*
   * Which quests open closed.
   *
   * The spike's rule for the arc view is "every quest closed by default", and
   * that is what `defaultClosedGroups` produces at the size the rule is about:
   * it closes everything once the level is over the 300-node budget. Below it,
   * closing boxes a designer could have seen costs them the view they opened
   * and buys nothing measurable, which is the same judgement the scene canvas
   * makes.
   *
   * The guard matters more than the default. This component is **remounted**
   * every time somebody comes back from a scene, so an unguarded effect would
   * throw away the quests they had opened on the way in — the exact state #187
   * requires the round trip to preserve. A closed set holding any id of the
   * current grouping is a set that belongs to it, and is left alone; anything
   * else is a first look at this grouping and gets the default.
   */
  useEffect(() => {
    if (presentation) return
    const mine = new Set(groups.groups.map((group) => group.id))
    if (closedGroups.some((id) => mine.has(id))) return
    setClosedGroups(defaultClosedGroups(arc.level, undefined, groups.groups))
    // `closedGroups` is deliberately not a dependency: this decides what the
    // *default* is, and re-running it every time the writer opens a box is how
    // it would start fighting them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [groups, arc.level, setClosedGroups])

  const change = (level: FlowLevel) => onChange({ ...arc, level })

  return (
    <>
      <div className="nrt-flow-head">
        <FlowModeTabs mode={mode} onMode={setMode} label="Arc view mode" />
        {readOnly && (
          <span className="nrt-badge">
            <Icon name="lock" size="sm" />
            Read-only
          </span>
        )}
      </div>

      {diagnostics.length > 0 && (
        /* Not a count, and not only the first one. Each row names the scene and
           offers to open it at the beat that authored the destination — the
           "links to the responsible outcome" #187 asks for, which a message in
           a banner cannot be. */
        <ul className="nrt-arc-diagnostics" aria-label="Arc diagnostics">
          {diagnostics.map((diagnostic) => (
            <li key={diagnostic.id} className={diagnostic.severity === 'error' ? 'is-bad' : ''}>
              <Icon name={diagnostic.severity === 'error' ? 'x' : 'clock'} size="sm" />
              <span>{diagnostic.message}</span>
              {diagnostic.at && (
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => onEnter(diagnostic.at!.sceneId, diagnostic.at!.beatId)}
                >
                  Open the scene
                </button>
              )}
            </li>
          ))}
        </ul>
      )}

      {mode === 'canvas' ? (
        <FlowCanvas
          scene={arc.level}
          onChange={change}
          readOnly={readOnly}
          layout={layout}
          positions={positions}
          onPositionsChange={onPositionsChange}
          presentation={presentation}
          creatable={creatable}
          grouping={groups}
          // No spare exit where an exit cannot be authored: a handle that
          // refuses every drag is worse than no handle.
          spare={authoring?.refuse ? undefined : arcSpare}
          authoring={authoring}
          dangling
          diagnostics={diagnostics}
          onActivate={onEnter}
          noun="scene"
          targetOf={arcTarget}
          toolbar={<GroupingField arc={arc} grouping={grouping} onGrouping={setGrouping} />}
        />
      ) : (
        <FlowOutline
          scene={arc.level}
          onChange={change}
          readOnly={readOnly}
          creatable={creatable}
          spare={authoring?.refuse ? undefined : arcSpare}
          authoring={authoring}
          onActivate={onEnter}
          activateLabel="Open scene"
          targetOf={arcTarget}
        />
      )}
    </>
  )
}

/** The arc creates scenes, and nothing else. Beats belong one level down. */
const CREATABLE: readonly FlowKind[] = ['scene']

/**
 * Group by quest, or say why not.
 *
 * `readOnly` and `aria-disabled` rather than `disabled`, the same choice the
 * rest of this workspace makes: a disabled control cannot be focused, so it
 * cannot explain itself, and "why can't I group by quest" is exactly the
 * question this build has to answer.
 */
function GroupingField({
  arc,
  grouping,
  onGrouping,
}: {
  arc: FlowArc
  grouping: ArcGrouping
  onGrouping: (grouping: ArcGrouping) => void
}) {
  const available = arc.quests !== null
  return (
    <label className="nrt-bar-field">
      <span>Group</span>
      <select
        value={grouping}
        aria-disabled={available ? undefined : 'true'}
        aria-describedby={available ? undefined : 'nrt-quests-unavailable'}
        onChange={(event) => available && onGrouping(event.target.value as ArcGrouping)}
      >
        {ARC_GROUPINGS.filter((option) => available || option.id === 'none').map((option) => (
          <option key={option.id} value={option.id}>
            {option.label}
          </option>
        ))}
      </select>
    </label>
  )
}
