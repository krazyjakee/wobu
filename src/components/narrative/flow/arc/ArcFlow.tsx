import { useEffect, useMemo, useState } from 'react'
import type { NarrativeTarget } from '../../../../store/ui'
import type { CanonicalFlowActions } from '../canonicalFlow'
import { FlowCanvas } from '../FlowCanvas'
import { FlowModeTabs, type FlowMode } from '../FlowModeTabs'
import { FlowOutline } from '../FlowOutline'
import { defaultClosedGroups } from '../graph'
import { FlowStoreContext, useFlowLevel, type FlowStoreApi } from '../flowStore'
import type { LayoutRunner } from '../layout'
import type { FlowKind, FlowPositions } from '../model'
import type { FlowPresentation } from '../useFlowPresentation'
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

export interface ArcView {
  mode: FlowMode
  grouping: ArcGrouping
}
interface Props {
  arc: FlowArc
  onChange: (arc: FlowArc) => void
  onEnter: (sceneId: string, beatId?: string | null, target?: Partial<NarrativeTarget>) => void
  onQuest?: (questId: string) => void
  readOnly?: boolean
  layout?: LayoutRunner
  positions?: FlowPositions
  onPositionsChange?: (positions: FlowPositions) => void
  presentation?: FlowPresentation
  authoring?: FlowAuthoring
  actions?: CanonicalFlowActions
  creatable?: readonly FlowKind[]
  store?: FlowStoreApi
  view?: ArcView
  onView?: (view: ArcView) => void
  /** Project groupings each have their own persistent arrangement. */
  persistentGrouping?: boolean
}
const CREATABLE: readonly FlowKind[] = ['scene']
export function ArcFlow({ store = ARC_STORE, ...props }: Props) {
  return (
    <FlowStoreContext value={store}>
      <Arc {...props} />
    </FlowStoreContext>
  )
}
function Arc({
  arc,
  onChange,
  onEnter,
  onQuest,
  readOnly = false,
  layout,
  positions,
  onPositionsChange,
  presentation,
  authoring,
  actions,
  creatable = CREATABLE,
  view,
  onView,
  persistentGrouping = false,
}: Props) {
  const [localView, setLocalView] = useState<ArcView>({
    mode: 'canvas',
    grouping: presentation ? 'arrangement' : arc.quests === null ? 'none' : 'quest',
  })
  const current = view ?? localView
  const changeView = onView ?? setLocalView
  const closedGroups = useFlowLevel((s) => s.closedGroups)
  const setClosedGroups = useFlowLevel((s) => s.setClosedGroups)
  const diagnostics = useMemo(
    () => arcDiagnostics(arc).sort((a, b) => a.severity.localeCompare(b.severity)),
    [arc],
  )
  const groups = useMemo(() => arcGrouping(arc, current.grouping), [arc, current.grouping])
  const derived = !persistentGrouping && current.grouping !== 'arrangement'
  useEffect(() => {
    if (!derived || closedGroups.some((id) => groups.groups.some((group) => group.id === id)))
      return
    setClosedGroups(defaultClosedGroups(arc.level, undefined, groups.groups))
    // Initialise this grouping once; opening its last group must remain possible.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [groups, derived, arc.level, setClosedGroups])
  const [page, setPage] = useState(0)
  const last = Math.max(0, Math.ceil(diagnostics.length / 25) - 1)
  const currentPage = Math.min(page, last)
  const errors = diagnostics.filter((d) => d.severity === 'error').length
  const level = useMemo(
    () => ({
      ...arc.level,
      elements: arc.level.elements.map((element) => ({
        ...element,
        diagnostics: diagnostics
          .filter((d) => d.elementId === element.id)
          .map((d) => ({
            id: d.id,
            code: 'arc.destination',
            message: d.message,
            severity: d.severity,
            category: 'destination' as const,
          })),
      })),
    }),
    [arc.level, diagnostics],
  )
  const activate = (id: string) => {
    const element = arc.level.elements.find((one) => one.id === id)
    if (element?.kind === 'questStage') onQuest?.(element.questId)
    else if (element?.kind === 'scene') onEnter(id)
  }
  const groupingControl = (
    <label className="nrt-bar-field">
      <span>Group</span>
      <select
        aria-label="Group"
        aria-disabled={arc.quests === null && !presentation ? true : undefined}
        value={current.grouping}
        onChange={(e) => changeView({ ...current, grouping: e.target.value as ArcGrouping })}
      >
        {ARC_GROUPINGS.filter((option) =>
          option.id === 'arrangement'
            ? !!presentation
            : option.id === 'quest' || option.id === 'questState'
              ? arc.quests !== null
              : true,
        ).map((option) => (
          <option key={option.id} value={option.id}>
            {option.label}
          </option>
        ))}
      </select>
    </label>
  )
  return (
    <>
      <div className="nrt-flow-head">
        <FlowModeTabs
          mode={current.mode}
          onMode={(mode) => changeView({ ...current, mode })}
          label="Arc view mode"
        />
        {readOnly && <span className="nrt-badge">Read-only</span>}
      </div>
      {arc.quests === null && (
        <p role="status">Quest grouping is unavailable until World can be read.</p>
      )}
      {diagnostics.length > 0 && (
        <details className="nrt-arc-checks" open={diagnostics.length <= 25}>
          <summary>
            {errors} blocking destination problems · {diagnostics.length - errors} scenes without an
            incoming exit. Includes filtered and collapsed nodes.
          </summary>
          <ul className="nrt-arc-diagnostics" aria-label="Arc diagnostics">
            {diagnostics.slice(currentPage * 25, (currentPage + 1) * 25).map((diagnostic) => (
              <li key={diagnostic.id} className={diagnostic.severity === 'error' ? 'is-bad' : ''}>
                <span>{diagnostic.message}</span>
                {diagnostic.at && (
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() =>
                      onEnter(diagnostic.at!.sceneId, diagnostic.at!.beatId, {
                        ...diagnostic.at,
                        field:
                          diagnostic.at!.choiceId || diagnostic.at!.outcomeId
                            ? 'destination'
                            : undefined,
                      })
                    }
                  >
                    Open the scene
                  </button>
                )}
                {diagnostic.questId && (
                  <button className="btn btn-sm" onClick={() => onQuest?.(diagnostic.questId!)}>
                    Open quest transition
                  </button>
                )}
              </li>
            ))}
          </ul>
          {last > 0 && (
            <nav aria-label="Arc diagnostic pages">
              <button
                className="btn"
                disabled={!currentPage}
                onClick={() => setPage(currentPage - 1)}
              >
                Previous diagnostics
              </button>
              <span>
                {currentPage + 1} / {last + 1}
              </span>
              <button
                className="btn"
                disabled={currentPage === last}
                onClick={() => setPage(currentPage + 1)}
              >
                Next diagnostics
              </button>
            </nav>
          )}
        </details>
      )}
      {current.mode === 'canvas' ? (
        <FlowCanvas
          scene={level}
          onChange={(next) =>
            onChange({
              ...arc,
              level: {
                ...next,
                elements: next.elements.map(({ diagnostics: _diagnostics, ...element }) => element),
              },
            })
          }
          readOnly={readOnly}
          layout={layout}
          positions={positions}
          onPositionsChange={onPositionsChange}
          presentation={presentation}
          creatable={creatable}
          grouping={derived ? groups : undefined}
          spare={authoring?.refuse ? undefined : arcSpare}
          authoring={authoring}
          actions={actions}
          dangling
          diagnostics={diagnostics}
          onActivate={activate}
          noun="scene"
          targetOf={arcTarget}
          toolbar={groupingControl}
        />
      ) : (
        <>
          <div className="nrt-flow-head">{groupingControl}</div>
          <FlowOutline
            pageSize={persistentGrouping ? 25 : undefined}
            scene={level}
            onChange={(next) =>
              onChange({
                ...arc,
                level: {
                  ...next,
                  elements: next.elements.map(
                    ({ diagnostics: _diagnostics, ...element }) => element,
                  ),
                },
              })
            }
            readOnly={readOnly}
            presentation={presentation}
            creatable={creatable}
            grouping={derived ? groups : undefined}
            spare={authoring?.refuse ? undefined : arcSpare}
            authoring={authoring}
            actions={actions}
            onActivate={activate}
            activateLabel={persistentGrouping ? 'Open scene or quest' : 'Open scene'}
            targetOf={arcTarget}
          />
        </>
      )}
    </>
  )
}
