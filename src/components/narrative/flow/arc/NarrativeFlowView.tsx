import { useCallback, useEffect, useMemo, useState, type KeyboardEvent } from 'react'
import { useUI } from '../../../../store/ui'
import { Icon } from '../../../Icon'
import { NarrativeFlowPane } from '../../NarrativeFlowPane'
import { NARRATIVE_UNAVAILABLE } from '../../narrativeModel'
import type { LayoutRunner } from '../layout'
import type { FlowScene } from '../model'
import { ArcFlow } from './ArcFlow'
import { beaconArc, beaconScenes } from './fixture'
import type { FlowArc } from './model'

/**
 * The Flow tab: an arc of scenes, and the scene you went into.
 *
 * This is the nested-flow model #187 names — the one articy uses. One canvas is
 * on screen at a time, and going down a level is a navigation rather than a
 * second window.
 *
 * ── what survives going down and coming back, and how ────────────────────────
 *
 * | | Where it lives | Why there |
 * | --- | --- | --- |
 * | The arc's model, with unsaved edits | `arc` state, here | Outlives the canvas, which unmounts |
 * | The arc's cursor, closed quests, viewport | `ArcFlow`'s module-scope store | Outlives the canvas for the same reason |
 * | A scene's unsaved edits | `scenes` state, here, by scene id | The scene pane is remounted per visit |
 * | Where either canvas is scrolled | The level store's `viewport` | Read once on mount by `FlowCanvas` |
 *
 * The other level's canvas is **unmounted**, not hidden. Keeping both mounted
 * would preserve everything for free and would also put two levels' worth of
 * nodes in React Flow's store at once — and the spike's budget is 300 nodes *in
 * the store*, not 300 on screen. Lifting the state out is what lets the answer
 * be "one level's worth, always".
 *
 * ── where the arc comes from ─────────────────────────────────────────────────
 *
 * This component is the **demonstration**, and by the time you read this it has
 * exactly one caller: `NarrativeProjectFlow`, when the project has no scenes at
 * all. A project with scenes draws its own arc from its own authored scene
 * links, through the same `ArcFlow` below.
 *
 * The quests are a fixture for a second and separate reason, which the banner
 * keeps apart from the first: there is no quest model *at all* (#155), so they
 * would still be invented on a real project's arc. That is why the grouping
 * note is shown whether or not the scenes are demonstration data.
 */

export type FlowViewSource =
  /** The in-memory beacon inquiry, clearly labelled as such. */
  | { kind: 'demo' }
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  /** A real arc, and the scenes it can open. Zero scenes is the empty state. */
  | { kind: 'ready'; arc: FlowArc; scenes: Record<string, FlowScene> }

export function NarrativeFlowView({
  source = { kind: 'demo' },
  readOnly = false,
  layout,
}: {
  source?: FlowViewSource
  readOnly?: boolean
  /** Swapped for a synchronous fake in tests; the real one starts a worker. */
  layout?: LayoutRunner
}) {
  // One copy per mount. Both fixtures return fresh objects every call, so
  // building them in the render body would throw the writer's edits away on
  // every keystroke somewhere else in the workspace.
  const demo = useMemo(() => {
    const scenes = beaconScenes()
    return { arc: beaconArc(scenes), scenes }
  }, [])

  if (source.kind === 'loading') {
    return (
      <div className="nrt-pane nrt-flow">
        <p className="nrt-note" aria-busy="true">
          <Icon name="clock" size="sm" />
          Reading the arc…
        </p>
      </div>
    )
  }

  if (source.kind === 'error') {
    return (
      <div className="nrt-pane nrt-flow">
        <p className="nrt-note inline-error" role="alert">
          Could not read this arc: {source.message}
        </p>
      </div>
    )
  }

  const opened = source.kind === 'demo' ? demo : source
  return (
    <Levels
      key={opened.arc.level.id}
      initial={opened}
      demo={source.kind === 'demo'}
      readOnly={readOnly}
      layout={layout}
    />
  )
}

function Levels({
  initial,
  demo,
  readOnly,
  layout,
}: {
  initial: { arc: FlowArc; scenes: Record<string, FlowScene> }
  demo: boolean
  readOnly: boolean
  layout?: LayoutRunner
}) {
  const [arc, setArc] = useState(initial.arc)
  const [scenes, setScenes] = useState(initial.scenes)
  const [entered, setEntered] = useState<{ sceneId: string; beatId: string | null } | null>(null)
  const selectNarrative = useUI((s) => s.selectNarrative)

  const enter = useCallback(
    (sceneId: string, beatId?: string | null) => {
      // Only into a scene there is something to show. A dangling destination is
      // a box on the arc, but it is a *hole*: there is no scene behind it, and
      // opening an empty editor would suggest there was one to fix.
      if (!scenes[sceneId]) return
      setEntered({ sceneId, beatId: beatId ?? null })
    },
    [scenes],
  )

  /*
   * The selection is written *after* the scene pane has mounted, not while it
   * is being asked to.
   *
   * A parent's effect runs after its children's, and the scene pane clears the
   * scene level's cursor in a mount effect of its own — so a selection raised
   * before the switch would be honoured by the canvas and then wiped a
   * microsecond later. Raising it here means the reveal arrives at a canvas
   * that is already up, which is what carries a writer following a diagnostic
   * to the beat that authored the destination rather than to the scene's start.
   *
   * The arc canvas does **not** read the reveal channel, and that is deliberate
   * rather than unfinished. The most recent reveal when it remounts is the one
   * *it* raised to get into the scene, so honouring it would move the arc's
   * cursor to whatever the writer just came out of — overwriting the selection
   * #187 requires the round trip to preserve. The cost is that choosing a scene
   * in the Library does not yet highlight it on the arc; the fix is a reveal
   * origin that says "a level went down", which is `store/ui.ts`'s to add.
   */
  useEffect(() => {
    if (entered === null) return
    selectNarrative({ sceneId: entered.sceneId, beatId: entered.beatId }, 'diagnostic')
  }, [entered, selectNarrative])

  const leave = useCallback(() => setEntered(null), [])

  const onSceneChange = useCallback(
    (next: FlowScene) => setScenes((all) => ({ ...all, [next.id]: next })),
    [],
  )

  const scene = entered === null ? null : (scenes[entered.sceneId] ?? null)

  /*
   * Escape leaves the scene — unless the canvas has already answered it.
   *
   * `useFlowKeyboard` calls `preventDefault` when Escape cancels a half-made
   * connection, and that is the more local meaning: a writer who is mid-connect
   * is asking to stop connecting, not to leave the scene they are working in.
   */
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Escape' || event.defaultPrevented) return
    event.preventDefault()
    leave()
  }

  if (scene !== null) {
    return (
      <div className="nrt-levels" onKeyDown={onKeyDown}>
        <Breadcrumb arc={arc} scene={scene} onLeave={leave} />
        <NarrativeFlowPane
          source={{ kind: 'ready', scene }}
          readOnly={readOnly}
          layout={layout}
          onSceneChange={onSceneChange}
        />
      </div>
    )
  }

  return (
    <div className="nrt-pane nrt-flow">
      <Breadcrumb arc={arc} scene={null} onLeave={leave} />

      {demo && (
        // Two claims, kept apart on purpose. The scenes are a fixture because
        // this build has no narrative storage; the quests are a fixture because
        // the quest model does not exist yet, and will still be one the day a
        // real read lands. Merging them would let the second disappear.
        <p className="nrt-note" role="status">
          <Icon name="lock" size="sm" />
          Demonstration data. {NARRATIVE_UNAVAILABLE.source} The scenes below are a fixture of the
          beacon inquiry, held in memory: edits work, and none of them are saved.
        </p>
      )}
      <p className="nrt-note" id="nrt-quests-unavailable" role="note">
        <Icon name="folder" size="sm" />
        {arc.quests === null
          ? NARRATIVE_UNAVAILABLE.quests
          : `Grouping uses demonstration quests. ${NARRATIVE_UNAVAILABLE.quests}`}
      </p>

      <ArcFlow arc={arc} onChange={setArc} onEnter={enter} readOnly={readOnly} layout={layout} />
    </div>
  )
}

/** Which level you are on, and the way back up. */
function Breadcrumb({
  arc,
  scene,
  onLeave,
}: {
  arc: FlowArc
  scene: FlowScene | null
  onLeave: () => void
}) {
  return (
    <nav className="nrt-crumbs" aria-label="Flow level">
      <button
        type="button"
        className={scene === null ? 'chip is-on' : 'chip'}
        aria-current={scene === null ? 'true' : undefined}
        onClick={onLeave}
      >
        <Icon name="layers" size="sm" />
        {arc.level.name}
      </button>
      {scene !== null && (
        <>
          <span aria-hidden>/</span>
          <span className="chip is-on" aria-current="true">
            <Icon name="place" size="sm" />
            {scene.name}
          </span>
          <span className="nrt-note-inline">Escape returns to the arc.</span>
        </>
      )}
    </nav>
  )
}
