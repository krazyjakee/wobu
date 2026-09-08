import { useCallback, useMemo, useState, type KeyboardEvent } from 'react'
import type { LayoutNotice, NarrativeDiagnostic, SceneFile } from '../../lib/api'
import {
  useDiagnoseScene,
  useSaveScene,
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
import { sceneNode, type FlowArc } from './flow/arc/model'
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
  patchScene,
  positionsFromLayout,
  sceneToFlow,
  NEW_OUTCOME_PORT,
} from './flow/source'

/** One array, so "no diagnostics" is not a new object on every render. */
const EMPTY: NarrativeDiagnostic[] = []

/**
 * The Flow tab, drawing the project's own scenes.
 *
 * This is the component that joins the canvas to the backend, and everything
 * load-bearing about it is a decision about *who owns the document*.
 *
 * ── the save is a patch, never a rebuild ─────────────────────────────────────
 *
 * The canvas hands back a `FlowLevel`, which has no dialogue in it. So this
 * component never turns one into a `Scene`. It hands the level and the document
 * it was drawn from to `patchScene`, which clones the document and applies only
 * the operations the canvas can express. Every line of dialogue in the project
 * depends on that distinction; `flow/source.test.ts` is where it is proved.
 *
 * ── one edit, one save, one undo entry ───────────────────────────────────────
 *
 * There is no local history and no "unsaved changes". A structural edit is
 * patched and saved immediately through `useSaveScene`, which is the workspace's
 * single choke point for a narrative write: it carries the guarded-write
 * precondition off the loaded file, pushes the undo entry, refreshes the cache
 * with the document the backend actually wrote, and turns a lost race into the
 * existing `write.conflict` card. None of that is reimplemented here, and the
 * canvas draws the *saved* document — so a refused save leaves the canvas
 * showing the truth rather than an edit that never landed.
 *
 * ── layout is a different file and cannot fail ───────────────────────────────
 *
 * Coordinates go through `useSaveLayout` to `narrative/layout/`, on their own
 * command, with no precondition and no undo entry. A failure there is a notice
 * above the canvas, never an error and never anything that can stop a source
 * save — the two share no state, so that holds because there is nothing to get
 * wrong rather than because the calls are ordered carefully.
 *
 * Quest membership comes from the World model. This pane only selects a quest
 * scope and edits cosmetic groups; source membership is authored in World.
 */

/** Stable legacy slug for the all-scenes arrangement; quests use EntityId. */
const PROJECT_ARC = 'project'

/** Nothing is creatable at the arc level: see `authoring` below. */
const ARC_CREATABLE: readonly FlowKind[] = []

/** What a source-backed scene allows, in the source model's own terms. */
const SCENE_AUTHORING = {
  destinationRequired: flowAuthoring.destinationRequired,
  fixedPort: flowAuthoring.fixedPort,
  derived: flowAuthoring.derived,
} as const

/** What a source-backed arc allows, which is looking and going in. */
const ARC_AUTHORING = { refuse: flowAuthoring.arcReadOnly } as const

/** A beat's offer of one more way out. Connecting from it authors an outcome. */
function beatSpare(element: FlowElement): FlowPort | null {
  if (element.kind !== 'beat') return null
  return { id: NEW_OUTCOME_PORT, label: 'Add a way out', to: null }
}

export function NarrativeProjectFlow({
  readOnly,
  layout,
}: {
  readOnly: boolean
  /** Swapped for a synchronous fake in tests; the real one starts a worker. */
  layout?: LayoutRunner
}) {
  const catalog = useScenes()
  const ids = useMemo(() => (catalog.data?.scenes ?? []).map((one) => one.id), [catalog.data])
  const files = useSceneFiles(ids)
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
    if (reveal.origin === 'library' && reveal.sceneId !== null && ids.includes(reveal.sceneId)) {
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
        readOnly={readOnly}
        layout={layout}
        onLeave={leave}
      />
    )
  }

  return (
    <ArcLevel
      files={files}
      loading={files.some((one) => one.isPending)}
      readOnly={readOnly}
      layout={layout}
      onEnter={enter}
    />
  )
}

/* ── the arc ──────────────────────────────────────────────────────────────── */

function ArcLevel(props: {
  files: { data?: SceneFile }[]
  loading: boolean
  readOnly: boolean
  layout?: LayoutRunner
  onEnter: (sceneId: string, beatId?: string | null) => void
}) {
  const world = useNarrativeWorld()
  const [questId, setQuestId] = useState('')
  const quest = world.data?.document.quests.find((quest) => quest.id === questId)
  return (
    <div className="nrt-quest-arrangement">
      <label className="nrt-quest-selector">
        Flow scope{' '}
        <select
          aria-label="Flow scope"
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
      <ArcArrangement key={quest?.id ?? 'project'} {...props} quest={quest} />
    </div>
  )
}

function ArcArrangement({
  files,
  loading,
  readOnly,
  layout,
  onEnter,
  quest,
}: {
  quest?: Quest
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
          ...sceneNode(scene),
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
      // Presentation groups are separate from World quest membership.
      quests: null,
    }
  }, [files, nameOf, sceneName, quest, presentation?.layout.groups])

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

function SceneLevel({
  sceneId,
  readOnly,
  layout,
  onLeave,
}: {
  sceneId: string
  readOnly: boolean
  layout?: LayoutRunner
  onLeave: () => void
}) {
  const file = useScene(sceneId)
  const graph = useMemo(() => ({ kind: 'scene' as const, scene: sceneId }), [sceneId])
  const { stored, outcome, presentation } = useFlowPresentation(graph)
  const saved = useSceneDiagnostics(sceneId)
  const diagnose = useDiagnoseScene()
  const save = useSaveScene()
  const { nameOf, sceneName } = useNarrativeNames()

  /*
   * Which answer describes what is on screen.
   *
   * `useSceneDiagnostics` reads the *saved* scene, which is normally the same
   * document — every edit here is saved as it is made. The mutation covers the
   * gap: it is run against the patched document at the moment of the edit, so
   * badges follow the writer's hand rather than the round trip. Whichever ran
   * more recently wins, which is the only comparison that cannot go stale in
   * either direction.
   */
  const savedDiagnostics = saved.data
  const savedAt = saved.dataUpdatedAt
  const pending = diagnose.data
  const pendingAt = diagnose.submittedAt
  const diagnostics: NarrativeDiagnostic[] = useMemo(
    () =>
      pending !== undefined && pendingAt > (savedAt || 0) ? pending : (savedDiagnostics ?? EMPTY),
    [pending, pendingAt, savedDiagnostics, savedAt],
  )

  const attached = useMemo(() => {
    if (!file.data) return null
    const level = sceneToFlow(file.data.scene, {
      layout: presentation?.layout ?? null,
      nameOf,
      sceneName,
    })
    return attachDiagnostics(level, diagnostics)
  }, [file.data, presentation?.layout, nameOf, sceneName, diagnostics])

  const positions = useMemo(
    () => positionsFromLayout(presentation?.layout, 'scene'),
    [presentation?.layout],
  )

  const onPositions = useCallback(
    (next: FlowPositions) => {
      const base = presentation?.layout
      if (!base) return
      // Fire and forget, deliberately. `useSaveLayout` cannot reject and
      // records nothing; a refused arrangement comes back as an outcome shown
      // in `LayoutNotices`, never as a toast and never as a failed save.
      presentation?.onChange(layoutWithPositions(base, next, 'scene'))
    },
    [presentation],
  )

  const onEdit = useCallback(
    (next: Parameters<NonNullable<Parameters<typeof NarrativeFlowPane>[0]['onEdit']>>[0]) => {
      const loaded = file.data
      if (!loaded) return
      const patch = patchScene(loaded.scene, next)
      // A gesture the source model cannot express changed nothing, and saving a
      // document identical to the one on disk would put a meaningless entry on
      // the undo stack. `useSceneEdits` has already said why it refused.
      if (patch.unchanged) return
      save.mutate({ file: loaded, scene: patch.scene })
      // Diagnostics for the document that is about to be written, so a badge
      // does not lag a round trip behind the canvas.
      diagnose.mutate({ sceneId, scene: patch.scene })
      const created = patch.created[0]
      if (created) useFlowStore.getState().select(created)
    },
    [diagnose, file.data, save, sceneId],
  )

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Escape' || event.defaultPrevented) return
    event.preventDefault()
    onLeave()
  }

  const source =
    file.isPending || stored.isPending
      ? ({ kind: 'loading' } as const)
      : file.isError
        ? ({ kind: 'error', message: String(file.error) } as const)
        : attached
          ? ({ kind: 'ready', scene: attached.level } as const)
          : ({ kind: 'loading' } as const)

  return (
    <div className="nrt-levels" onKeyDown={onKeyDown}>
      <nav className="nrt-crumbs" aria-label="Flow level">
        <button type="button" className="chip" onClick={onLeave}>
          <Icon name="layers" size="sm" />
          Every scene
        </button>
        <span aria-hidden>/</span>
        <span className="chip is-on" aria-current="true">
          <Icon name="place" size="sm" />
          {file.data?.scene.name ?? sceneId}
        </span>
        <span className="nrt-note-inline">Escape returns to the arc.</span>
      </nav>

      <NarrativeFlowPane
        source={source}
        readOnly={readOnly}
        layout={layout}
        onEdit={readOnly ? undefined : onEdit}
        positions={positions}
        presentation={presentation}
        onPositionsChange={onPositions}
        authoring={SCENE_AUTHORING}
        creatable={SCENE_CREATABLE}
        spare={beatSpare}
        notes={
          <>
            <LayoutNotices notices={stored.data?.notices} outcome={outcome} />
            <SceneWideProblems found={attached?.sceneWide ?? []} />
            <p className="nrt-note" role="note">
              <Icon name="lock" size="sm" />
              {NARRATIVE_UNAVAILABLE.witness} {NARRATIVE_UNAVAILABLE.affectedScope}
            </p>
          </>
        }
      />
    </div>
  )
}

/**
 * The only kind a source-backed scene canvas offers to create.
 *
 * A `Choice` and an `Outcome` live inside a beat and both need a destination
 * the moment they exist — `Destination` has no "nowhere" — so an Add button for
 * either would have to invent an ending nobody chose. They are authored instead
 * by connecting from a beat's spare handle, where the gesture supplies the
 * destination. A `Condition` is not an element of the source model at all: a
 * condition is a field on a choice or an outcome.
 */
const SCENE_CREATABLE: readonly FlowKind[] = ['beat']

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
