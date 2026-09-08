import { useSceneEditSession } from './useSceneEditSession'
import { SceneEditControls } from './SceneEditControls'
import { applySceneEdit, type SceneEditOperation } from './sceneEdits'
import { canonicalFlowActions, flowNodeForTarget } from './flow/canonicalFlow'
import { FlowElementEditor } from './flow/FlowElementEditor'
import { useCallback, useEffect, useMemo, useState, type KeyboardEvent } from 'react'
import type { LayoutNotice, NarrativeDiagnostic, SceneFile } from '../../lib/api'
import {
  useDiagnoseScene,
  useScene,
  useSceneDiagnostics,
  useSceneFiles,
  useScenes,
} from '../../lib/queries'
import { useUI } from '../../store/ui'
import { Icon } from '../Icon'
import { NarrativeFlowPane } from './NarrativeFlowPane'
import { NarrativeFlowView } from './flow/arc/NarrativeFlowView'
import { NARRATIVE_UNAVAILABLE } from './narrativeModel'
import { ArcFlow } from './flow/arc/ArcFlow'
import { sceneNode, worldQuests, type ArcQuests, type FlowArc } from './flow/arc/model'
import { attachDiagnostics } from './flow/badges'
import { useFlowStore } from './flow/flowStore'
import { useFlowPresentation } from './flow/useFlowPresentation'
import { useNarrativeWorld } from '../../lib/queries/narrativeWorld'
import type { Quest } from '../../lib/api/narrativeWorld'
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

/** Stable legacy slug for the all-scenes arrangement; quests use EntityId. */
const PROJECT_ARC = 'project'

/** Nothing is creatable at the arc level: see `authoring` below. */
const ARC_CREATABLE: readonly FlowKind[] = []

/** What a source-backed scene allows, in the source model's own terms. */
/** What a source-backed arc allows, which is looking and going in. */
const ARC_AUTHORING = { refuse: flowAuthoring.arcReadOnly } as const

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
  // Not until the catalog has answered: a reveal marked as handled against an
  // empty `ids` would be a request quietly thrown away, and the writer would be
  // left on the arc having clicked a scene.
  if (reveal !== null && reveal.seq !== honoured && catalog.isSuccess) {
    setHonoured(reveal.seq)
    if (
      (reveal.projectKey === null || reveal.projectKey === projectKey) &&
      reveal.sceneId !== null &&
      ids.includes(reveal.sceneId)
    ) {
      setEntered(reveal.sceneId)
    }
  }

  const enter = useCallback(
    (sceneId: string, beatId?: string | null) => {
      setEntered(sceneId)
      // The whole path, and the origin that raised it. `diagnostic` is what
      // carries a writer following a broken destination to the beat that
      // authored it rather than to the scene's start; the reveal is latched, so
      // the canvas honours it when it mounts a moment later.
      selectNarrative({ sceneId, beatId: beatId ?? null }, beatId ? 'diagnostic' : 'flow')
    },
    [selectNarrative],
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

  /*
   * An empty project gets the demonstration, and is told that is what it is.
   *
   * This is the *only* place the beacon fixture is drawn now. #186 built the
   * canvas against it because there was no backend; there is one, so a project
   * with scenes draws its own and a fixture would be a lie. A project with none
   * has nothing to draw at all, and a canvas somebody can push around while
   * they decide whether this is the tool for them is worth more than an empty
   * rectangle — as long as the banner says, in the pane, that nothing they do
   * to it is saved.
   */
  if (ids.length === 0) {
    return (
      <div className="nrt-project-demo">
        <p className="nrt-note" role="status">
          <Icon name="lock" size="sm" />
          This project has no scenes yet, so the arc below is a demonstration. Create the first
          scene in the Library and this becomes your own.
        </p>
        <NarrativeFlowView readOnly={readOnly} layout={layout} />
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

  return <ArcLevel ids={ids} active={active} readOnly={readOnly} layout={layout} onEnter={enter} />
}

/* ── the arc ──────────────────────────────────────────────────────────────── */

function ArcLevel(props: {
  ids: string[]
  active: boolean
  readOnly: boolean
  layout?: LayoutRunner
  onEnter: (sceneId: string, beatId?: string | null) => void
}) {
  const world = useNarrativeWorld()
  const [questId, setQuestId] = useState('')
  const quest = world.data?.document.quests.find((quest) => quest.id === questId)
  /*
   * The project's own quest membership, for #187's grouping.
   *
   * `world.data` and not `world.data ?? []`: a read that has not answered, or
   * that failed, produces `quests: null` — "this build cannot say" — and the
   * grouping control refuses itself with that reason. An empty list would say
   * the project has no quests, which is a different and possibly false claim.
   */
  const quests = useMemo(() => worldQuests(world.data?.document.quests), [world.data])
  return (
    <div className="nrt-quest-arrangement">
      <label className="nrt-quest-selector">
        Flow scope{' '}
        <select
          aria-label="Flow scope"
          disabled={world.isPending}
          value={quest?.id ?? ''}
          onChange={(event) => setQuestId(event.target.value)}
        >
          <option value="">Every scene</option>
          {world.data?.document.quests.map((quest) => (
            <option key={quest.id} value={quest.id}>
              {quest.name}
            </option>
          ))}
        </select>
      </label>
      {world.isError && (
        <p className="nrt-note" role="status">
          Quest scopes could not be loaded; the project arrangement is still available.
        </p>
      )}
      <ArcPage key={quest?.id ?? 'project'} {...props} quest={quest} quests={quests} />
    </div>
  )
}

/** Load complete source only for the displayed arc page, never a hidden scene editor. */
const ARC_PAGE_SIZE = 50
function ArcPage({
  ids,
  active,
  quest,
  ...props
}: {
  ids: string[]
  active: boolean
  quest?: Quest
  quests: ArcQuests
  readOnly: boolean
  layout?: LayoutRunner
  onEnter: (sceneId: string, beatId?: string | null) => void
}) {
  const [page, setPage] = useState(0)
  const scoped = useMemo(
    () => (quest ? ids.filter((id) => quest.scene_ids.includes(id)) : ids),
    [ids, quest],
  )
  const last = Math.max(0, Math.ceil(scoped.length / ARC_PAGE_SIZE) - 1)
  const current = Math.min(page, last)
  const visible = useMemo(
    () => scoped.slice(current * ARC_PAGE_SIZE, (current + 1) * ARC_PAGE_SIZE),
    [scoped, current],
  )
  const files = useSceneFiles(visible, active)
  return (
    <>
      {scoped.length > ARC_PAGE_SIZE && (
        <nav className="nrt-crumbs" aria-label="Flow scene pages">
          <button
            type="button"
            className="btn"
            disabled={current === 0}
            onClick={() => setPage(current - 1)}
          >
            Previous scenes
          </button>
          <span role="status">
            Scenes {current * ARC_PAGE_SIZE + 1}–
            {Math.min((current + 1) * ARC_PAGE_SIZE, scoped.length)} of {scoped.length}. Connections
            outside this page are not shown; narrow Flow scope or use the Library to open any scene.
          </span>
          <button
            type="button"
            className="btn"
            disabled={current === last}
            onClick={() => setPage(current + 1)}
          >
            Next scenes
          </button>
        </nav>
      )}
      {files.some((one) => one.isError) && (
        <p role="alert" className="nrt-note inline-error">
          Some scenes on this page could not be read. Open the Library to inspect their source.
        </p>
      )}
      <ArcArrangement
        {...props}
        quest={quest}
        files={files}
        loading={files.some((one) => one.isPending)}
      />
    </>
  )
}

function ArcArrangement({
  files,
  loading,
  readOnly,
  layout,
  onEnter,
  quest,
  quests,
}: {
  quest?: Quest
  quests: ArcQuests
  files: { data?: SceneFile }[]
  loading: boolean
  readOnly: boolean
  layout?: LayoutRunner
  onEnter: (sceneId: string, beatId?: string | null) => void
}) {
  const graph = useMemo(
    () =>
      quest
        ? { kind: 'quest' as const, quest: quest.id }
        : { kind: 'arc' as const, arc: PROJECT_ARC },
    [quest],
  )
  const { stored, outcome, presentation } = useFlowPresentation(graph)
  const { nameOf, sceneName } = useNarrativeNames()

  /*
   * The arc, built only from authored `sceneLink` destinations.
   *
   * `sceneToFlow` produces a `sceneLink` element for a `Destination::Scene` and
   * for nothing else, and `sceneNode` reads those and nothing else. No name, no
   * summary and no line of prose is inspected anywhere on this path, which is
   * the property #187 requires and `arc/model.test.ts` holds to with a scene
   * whose prose is nothing but other scenes' names.
   */
  const arc: FlowArc = useMemo(() => {
    const scenes = files.flatMap((one) =>
      one.data && (!quest || quest.scene_ids.includes(one.data.scene.id))
        ? [sceneToFlow(one.data.scene, { nameOf, sceneName })]
        : [],
    )
    return {
      level: {
        id: quest ? `quest:${quest.id}` : `arc:${PROJECT_ARC}`,
        name: quest?.name ?? 'Every scene',
        groups: Object.values(presentation?.layout.groups ?? {}).map((group) => ({
          id: group.id,
          name: group.label ?? group.id,
        })),
        elements: scenes.map((scene) => ({
          // Two independent memberships on one node, and neither is written
          // anywhere: the arrangement group a writer drew (#185), and the World
          // quest that lists this scene (#187). Which one carves the canvas up
          // is the grouping control's business, not the model's.
          ...sceneNode(scene, quests.questOf(scene.id)),
          groupId:
            Object.values(presentation?.layout.groups ?? {}).find((group) =>
              (group.members ?? []).includes(`scene:${scene.id}`),
            )?.id ?? null,
        })),
        // The first scene in the catalog, which is alphabetical by file. There
        // is nothing in a project that declares where the story starts, and
        // Names remain labels; World scene_ids determines the selected scope.
        entryId: scenes[0]?.id ?? null,
      },
      // Presentation groups are separate from World quest membership; the arc
      // can be carved up by either, and by neither.
      quests: quests.quests,
    }
  }, [files, nameOf, sceneName, quest, quests, presentation?.layout.groups])

  const positions = useMemo(
    () => positionsFromLayout(presentation?.layout, 'arc'),
    [presentation?.layout],
  )

  const onPositions = useCallback(
    (next: FlowPositions) => {
      const base = presentation?.layout
      if (!base) return
      presentation?.onChange(layoutWithPositions(base, next, 'arc'))
    },
    [presentation],
  )

  if (loading || stored.isPending) {
    return (
      <div className="nrt-pane nrt-flow">
        <p className="nrt-note" aria-busy="true">
          <Icon name="clock" size="sm" />
          Reading the scenes…
        </p>
      </div>
    )
  }

  if (arc.level.elements.length === 0) {
    return (
      <div className="nrt-pane nrt-flow">
        <div className="nrt-empty">
          <p>
            This project has no scenes yet. A scene is a place in the story where people speak and
            choices are made; the Library is where the first one is created.
          </p>
        </div>
      </div>
    )
  }

  return (
    <div className="nrt-pane nrt-flow">
      <nav className="nrt-crumbs" aria-label="Flow level">
        <span className="chip is-on" aria-current="true">
          <Icon name="layers" size="sm" />
          {arc.level.name}
        </span>
      </nav>

      <LayoutNotices notices={stored.data?.notices} outcome={outcome} />

      <p className="nrt-note" role="note">
        <Icon name="lock" size="sm" />
        {flowAuthoring.arcReadOnly} {NARRATIVE_UNAVAILABLE.affectedScope}
      </p>

      {/* The id is what the grouping control points at when it refuses itself,
          so the refusal has somewhere to be read. */}
      <p className="nrt-note" id="nrt-quests-unavailable" role="note">
        <Icon name="folder" size="sm" />
        {quests.quests === null
          ? `The project’s quests could not be read, so the arc cannot be grouped by them. ${NARRATIVE_UNAVAILABLE.quests}`
          : NARRATIVE_UNAVAILABLE.quests}
        {quests.shared.length > 0 &&
          ` ${quests.shared.length} scene${quests.shared.length === 1 ? ' is' : 's are'} listed by more than one quest, and appear under the first.`}
      </p>

      <ArcFlow
        arc={arc}
        // Never called: every edit at this level is refused before it reaches
        // `onChange`. Present because `ArcFlow` is the same component the
        // demonstration arc uses, where edits are real.
        onChange={() => {}}
        onEnter={onEnter}
        readOnly={readOnly}
        layout={layout}
        positions={positions}
        presentation={presentation}
        onPositionsChange={onPositions}
        authoring={ARC_AUTHORING}
        creatable={ARC_CREATABLE}
      />
    </div>
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
  return (
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
          source={stored.isPending ? { kind: 'loading' } : { kind: 'ready', scene: attached.level }}
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
              <p className="nrt-note" role="note">
                {NARRATIVE_UNAVAILABLE.witness} {NARRATIVE_UNAVAILABLE.affectedScope}
              </p>
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
