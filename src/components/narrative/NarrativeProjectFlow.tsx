import { useSceneEditSession } from './useSceneEditSession'
import { SceneEditControls } from './SceneEditControls'
import { applySceneEdit, type SceneEditOperation } from './sceneEdits'
import { canonicalFlowActions, flowNodeForTarget } from './flow/canonicalFlow'
import { FlowElementEditor } from './flow/FlowElementEditor'
import { useCallback, useEffect, useMemo, useState, type KeyboardEvent } from 'react'
import type { LayoutNotice, NarrativeDiagnostic, SceneFile } from '../../lib/api'
import { useDiagnoseScene, useScene, useSceneDiagnostics, useScenes } from '../../lib/queries'
import { useUI, type NarrativeTarget } from '../../store/ui'
import { Icon } from '../Icon'
import { NarrativeFlowPane } from './NarrativeFlowPane'
import { NARRATIVE_UNAVAILABLE } from './narrativeModel'
import { ProjectArc } from './flow/arc/ProjectArc'
import { attachDiagnostics } from './flow/badges'
import { useFlowStore } from './flow/flowStore'
import { PreviewOverlayProvider, RouteBanner, RouteDetail } from './flow/PreviewOverlay'
import { usePlayedScenes, usePreviewRoute } from './flow/overlay'
import { useFlowPresentation } from './flow/useFlowPresentation'
import { useNarrativeNames } from './flow/useNarrativeNames'
import type { LayoutRunner } from './flow/layout'
import type { FlowElement, FlowKind, FlowPort, FlowPositions } from './flow/model'
import {
  flowAuthoring,
  layoutWithPositions,
  positionsFromLayout,
  sceneToFlow,
  NEW_OUTCOME_PORT,
} from './flow/source'

/** One array, so "no diagnostics" is not a new object on every render. */
const EMPTY: NarrativeDiagnostic[] = []

/**
 * Project Flow renders the same complete scene draft as Script. Gestures become
 * canonical reducer operations; only explicit Save commits source and records
 * canonical undo. Arrangement autosave remains a separate cosmetic sidecar.
 */

/** A beat's offer of one more way out. Connecting from it authors an outcome. */
function beatSpare(element: FlowElement): FlowPort | null {
  if (element.kind !== 'beat') return null
  return { id: NEW_OUTCOME_PORT, label: 'Add a way out', to: null }
}

export function NarrativeProjectFlow({
  projectKey = '',
  active = true,
  readOnly,
  layout,
}: {
  readOnly: boolean
  projectKey?: string
  active?: boolean
  /** Swapped for a synchronous fake in tests; the real one starts a worker. */
  layout?: LayoutRunner
}) {
  const catalog = useScenes()
  const ids = useMemo(() => (catalog.data?.scenes ?? []).map((one) => one.id), [catalog.data])
  const selectNarrative = useUI((s) => s.selectNarrative)
  const reveal = useUI((s) => s.narrativeReveal)

  /*
   * Which level is on screen.
   *
   * Local, because leaving a scene is not the same as deselecting it: coming
   * back up to the arc must leave Script and the inspector still pointing at
   * the scene the writer was in. So the selection cannot be the only record of
   * which level is drawn.
   *
   * The Library is the other way in, and it writes only the selection — so this
   * follows the *reveal* channel rather than the selection itself. That is what
   * makes choosing the same scene twice work: a reveal carries a rising `seq`,
   * so the second click is a new request rather than a value that did not
   * change. Adjusted during render rather than in an effect, which is React's
   * own answer for state that follows a prop, and which avoids the extra pass
   * an effect would spend drawing the arc over a selection that had moved.
   *
   * A scene the catalog does not have is not entered: an id can outlive the
   * file it named, and opening an empty editor for one would suggest there was
   * something there to fix.
   */
  const [entered, setEntered] = useState<string | null>(null)
  const [honoured, setHonoured] = useState(0)
  const played = usePlayedScenes(projectKey)
  // Not until the catalog has answered: a reveal marked as handled against an
  // empty `ids` would be a request quietly thrown away, and the writer would be
  // left on the arc having clicked a scene.
  if (reveal !== null && reveal.seq !== honoured && catalog.isSuccess) {
    setHonoured(reveal.seq)
    if (
      (reveal.projectKey === null || reveal.projectKey === projectKey) &&
      reveal.focus &&
      reveal.sceneId !== null &&
      ids.includes(reveal.sceneId)
    ) {
      setEntered(reveal.sceneId)
    }
  }

  const enter = useCallback(
    (sceneId: string, beatId?: string | null, target?: Partial<NarrativeTarget>) => {
      setEntered(sceneId)
      // The whole path, and the origin that raised it. `diagnostic` is what
      // carries a writer following a broken destination to the beat that
      // authored it rather than to the scene's start; the reveal is latched, so
      // the canvas honours it when it mounts a moment later.
      selectNarrative(
        { sceneId, beatId: beatId ?? null, ...target },
        beatId ? 'diagnostic' : 'flow',
        { projectKey },
      )
    },
    [selectNarrative, projectKey],
  )

  /** Back to the arc. The selection is left alone: leaving is not deselecting. */
  const leave = useCallback(() => setEntered(null), [])

  if (catalog.isPending) {
    return (
      <div className="nrt-pane nrt-flow">
        <p className="nrt-note" aria-busy="true">
          <Icon name="clock" size="sm" />
          Reading the scenes…
        </p>
      </div>
    )
  }

  if (catalog.isError) {
    return (
      <div className="nrt-pane nrt-flow">
        <p className="nrt-note inline-error" role="alert">
          Could not read this project’s scenes: {String(catalog.error)}
        </p>
      </div>
    )
  }

  if (entered !== null) {
    return (
      <SceneLevel
        key={entered}
        sceneId={entered}
        projectKey={projectKey}
        readOnly={readOnly}
        layout={layout}
        onLeave={leave}
      />
    )
  }

  return (
    /*
     * The arc gets one badge per scene and is told why (#188).
     *
     * Preview plays a single scene, so there is no cross-scene route to draw
     * and drawing one would be inventing connections nothing observed. The
     * provider carries only the set of scenes played, and `RouteBanner` states
     * that limitation in the pane rather than in a comment.
     */
    <PreviewOverlayProvider value={played}>
      <RouteBanner />
      <ProjectArc
        projectKey={projectKey}
        active={active}
        readOnly={readOnly}
        layout={layout}
        onEnter={enter}
      />
    </PreviewOverlayProvider>
  )
}

/* ── one scene ────────────────────────────────────────────────────────────── */

interface SceneLevelProps {
  sceneId: string
  projectKey: string
  readOnly: boolean
  layout?: LayoutRunner
  onLeave: () => void
}
function SceneLevel(props: SceneLevelProps) {
  const file = useScene(props.sceneId)
  if (file.isPending) return <NarrativeFlowPane source={{ kind: 'loading' }} />
  if (file.isError)
    return <NarrativeFlowPane source={{ kind: 'error', message: String(file.error) }} />
  return <SceneSourceEditor {...props} file={file.data} />
}
function SceneSourceEditor({
  file,
  sceneId,
  projectKey,
  readOnly,
  layout,
  onLeave,
}: SceneLevelProps & { file: SceneFile }) {
  const session = useSceneEditSession(file, projectKey, readOnly)
  const scene = session.scene
  const graph = useMemo(() => ({ kind: 'scene' as const, scene: sceneId }), [sceneId])
  const { stored, outcome, presentation } = useFlowPresentation(graph)
  const saved = useSceneDiagnostics(sceneId)
  const diagnose = useDiagnoseScene()
  const diagnoseScene = diagnose.mutate
  useEffect(() => {
    if (session.dirty) diagnoseScene({ sceneId, scene })
  }, [scene, sceneId, session.dirty, diagnoseScene])
  const diagnostics = session.dirty
    ? diagnose.variables?.scene === scene
      ? (diagnose.data ?? EMPTY)
      : EMPTY
    : (saved.data ?? EMPTY)
  const { nameOf, sceneName } = useNarrativeNames()
  const attached = useMemo(
    () =>
      attachDiagnostics(
        sceneToFlow(scene, {
          layout: presentation?.layout ?? null,
          nameOf,
          sceneName,
        }),
        diagnostics,
      ),
    [scene, presentation?.layout, nameOf, sceneName, diagnostics],
  )
  const positions = useMemo(
    () => positionsFromLayout(presentation?.layout, 'scene'),
    [presentation?.layout],
  )
  const onPositions = useCallback(
    (next: FlowPositions) => {
      if (presentation)
        presentation.onChange(layoutWithPositions(presentation.layout, next, 'scene'))
    },
    [presentation],
  )
  const dispatch = (operation: SceneEditOperation, focus = false) => {
    if (session.disabled) {
      useFlowStore.getState().announce('Scene editing is currently unavailable.')
      return false
    }
    const result = applySceneEdit(scene, operation)
    if ('refused' in result) {
      useFlowStore.getState().announce(result.refused)
      return false
    }
    if (!result.changed) return true
    if (!session.edit(result.scene)) return false
    const removed = result.removed
      .map(
        (target) =>
          target.variantId ?? target.lineId ?? target.choiceId ?? target.outcomeId ?? target.beatId,
      )
      .filter((id): id is string => !!id)
    if (removed.length) useUI.getState().forgetNarrative(removed)
    if (focus || result.created.length || result.removed.length) {
      useUI.getState().selectNarrative(result.target, 'flow', { projectKey, focus: true })
      useFlowStore.getState().select(flowNodeForTarget(result.target))
    }
    if (operation.kind === 'removeBeat')
      useFlowStore
        .getState()
        .announce(
          'Beat deleted. Incoming routes retain the deleted destination and its diagnostic.',
        )
    else if (operation.kind === 'removeRoute')
      useFlowStore.getState().announce('Route deleted. Its beat and dialogue are kept.')
    else if (result.created.length)
      useFlowStore.getState().announce('Added to the shared scene draft. Save scene to write it.')
    return true
  }
  const actions = canonicalFlowActions(
    scene,
    (operation) => dispatch(operation, true),
    (message) => useFlowStore.getState().announce(message),
    { projectKey, disabled: session.disabled },
  )
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Escape' || event.defaultPrevented) return
    event.preventDefault()
    onLeave()
  }
  /*
   * The Preview route, read from the session Preview already holds (#188).
   *
   * Derived on render from the frames the runtime returned, so a restart, a
   * checkpoint restore and a step back all update it without a code path each —
   * every one of them is simply a different list of frames. `session.dirty` is
   * what lets it say that the draft on screen is not the source the build
   * compiled; the overlay is drawn anyway, because a route over an edited scene
   * is still the route that ran.
   */
  const route = usePreviewRoute(projectKey, sceneId, attached.level, session.dirty)
  // Hidden, not discarded: clearing the overlay is a view decision and must
  // not throw away a run the writer may still be reading in the Preview tab.
  const [routeHidden, setRouteHidden] = useState(false)
  return (
    <PreviewOverlayProvider value={routeHidden ? null : route}>
      <div className="nrt-levels" onKeyDown={onKeyDown}>
        <nav className="nrt-crumbs" aria-label="Flow level">
          <button type="button" className="chip" onClick={onLeave}>
            <Icon name="layers" size="sm" />
            Every scene
          </button>
          <span aria-hidden>/</span>
          <span className="chip is-on" aria-current="true">
            {scene.name}
          </span>
        </nav>
        <SceneEditControls session={session} />
        <div className="nrt-source-flow-body">
          <NarrativeFlowPane
            source={
              stored.isPending ? { kind: 'loading' } : { kind: 'ready', scene: attached.level }
            }
            readOnly={readOnly}
            layout={layout}
            onEdit={() => {
              /* Canonical actions own source edits; display models never write. */
            }}
            actions={actions}
            authoring={{ fixedPort: flowAuthoring.fixedPort }}
            positions={positions}
            presentation={presentation}
            onPositionsChange={onPositions}
            creatable={SCENE_CREATABLE}
            spare={beatSpare}
            notes={
              <>
                <LayoutNotices notices={stored.data?.notices} outcome={outcome} />
                <SceneWideProblems found={attached.sceneWide} />
                <RouteBanner onClose={() => setRouteHidden(true)} />
                <RouteDetail />
                {route && routeHidden && (
                  <p className="nrt-note" role="status">
                    <Icon name="spark" size="sm" />
                    The Preview route is hidden.{' '}
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() => setRouteHidden(false)}
                    >
                      Show it again
                    </button>
                  </p>
                )}
                <details className="nrt-note nrt-flow-limitations">
                  <summary>About validation and affected text</summary>
                  <p>
                    {NARRATIVE_UNAVAILABLE.witness} {NARRATIVE_UNAVAILABLE.affectedScope}
                  </p>
                </details>
              </>
            }
          />
          <FlowElementEditor
            diagnostics={diagnostics}
            scene={scene}
            projectKey={projectKey}
            disabled={session.disabled}
            onEditOperation={(operation) => dispatch(operation)}
          />
        </div>
      </div>
    </PreviewOverlayProvider>
  )
}
const SCENE_CREATABLE: readonly FlowKind[] = ['beat', 'choice', 'condition', 'outcome', 'end']

/** Problems that belong to the scene rather than to any box on the canvas. */
function SceneWideProblems({ found }: { found: { id: string; message: string }[] }) {
  if (found.length === 0) return null
  return (
    <ul className="nrt-arc-diagnostics" aria-label="Problems with this scene">
      {found.map((one) => (
        <li key={one.id} className="is-bad">
          <Icon name="x" size="sm" />
          <span>{one.message}</span>
        </li>
      ))}
    </ul>
  )
}

/**
 * What happened to the arrangement, as information.
 *
 * `role="status"` and never `role="alert"`, and no `inline-error` class: #185's
 * whole point is that layout cannot stop anybody. A sidecar written by a newer
 * Wobu, a read-only share and a disk that refused the write are all things to
 * know about the *boxes*, and none of them puts a writer's words at risk.
 */
function LayoutNotices({
  notices,
  outcome,
}: {
  notices?: LayoutNotice[]
  outcome?: { outcome: string; rel?: string; reason?: string }
}) {
  const said: string[] = []
  for (const notice of notices ?? []) {
    if (notice.kind === 'missing') continue // A project with no arrangement yet is ordinary.
    if (notice.kind === 'unreadable')
      said.push(
        `The arrangement in ${notice.rel} could not be read (${notice.reason}). The boxes below were placed automatically.`,
      )
    if (notice.kind === 'newerSchema')
      said.push(
        `The arrangement in ${notice.rel} was written by a newer Wobu (version ${notice.found}; this build reads ${notice.supported}). It is left alone rather than downgraded.`,
      )
    if (notice.kind === 'wrongGraph')
      said.push(`The arrangement in ${notice.rel} describes a different graph, so it was not used.`)
    if (notice.kind === 'unplaced')
      said.push(
        `${notice.nodes?.length ?? 0} boxes had no stored position and were placed automatically.`,
      )
    if (notice.kind === 'stale')
      said.push(
        `${notice.nodes?.length ?? 0} stored positions named elements this scene no longer has, and were dropped.`,
      )
  }
  if (outcome?.outcome === 'deferred') {
    said.push(
      `Positions were not written: ${outcome.rel} comes from a newer Wobu, and overwriting it would trade a colleague’s whole arrangement for one drag.`,
    )
  }
  if (outcome?.outcome === 'unwritable') {
    said.push(
      `Positions are only in this session: ${outcome.reason}. Nothing about the story is affected.`,
    )
  }
  if (said.length === 0) return null
  return (
    <p className="nrt-note" role="status">
      <Icon name="folder" size="sm" />
      {said.join(' ')}
    </p>
  )
}
