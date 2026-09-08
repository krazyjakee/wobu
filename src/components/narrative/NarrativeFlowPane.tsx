import type { CanonicalFlowActions } from './flow/canonicalFlow'
import { flowTarget } from './flow/canonicalFlow'
import type { FlowPresentation } from './flow/useFlowPresentation'
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useUI } from '../../store/ui'
import { Icon } from '../Icon'
import { NarrativePlaceholder } from './NarrativePlaceholder'
import { NARRATIVE_UNAVAILABLE } from './narrativeModel'
import { useFlowReveal } from './flow/useFlowReveal'
import { FlowCanvas } from './flow/FlowCanvas'
import { FlowModeTabs, type FlowMode } from './flow/FlowModeTabs'
import { FlowOutline } from './flow/FlowOutline'
import { councilHearing } from './flow/fixture'
import { resetFlowStore, useFlowStore } from './flow/flowStore'
import type { LayoutRunner } from './flow/layout'
import {
  addElement,
  type FlowElement,
  type FlowKind,
  type FlowPort,
  type FlowPositions,
  type FlowScene,
} from './flow/model'
import type { FlowAuthoring } from './flow/useSceneEdits'

/**
 * Controlled project views render the shared scene session and dispatch canonical
 * operations. The demonstration alone keeps a disposable visual-model history.
 * Selection and latched reveals use stable source IDs across every view.
 */

export type FlowPaneSource =
  /** The in-memory Ashfall council hearing, clearly labelled as such. */
  | { kind: 'demo' }
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  /** A real scene. Zero elements is the *empty* state, and a real answer. */
  | { kind: 'ready'; scene: FlowScene }

export function NarrativeFlowPane({
  source = { kind: 'demo' },
  readOnly = false,
  layout,
  onSceneChange,
  onEdit,
  positions,
  onPositionsChange,
  presentation,
  authoring,
  actions,
  creatable,
  spare,
  notes,
}: {
  source?: FlowPaneSource
  readOnly?: boolean
  /** Swapped for a synchronous fake in tests; the real one starts a worker. */
  layout?: LayoutRunner
  /**
   * Take ownership of every structural edit — see the note above.
   *
   * Present means the pane is controlled and keeps no history of its own.
   */
  onEdit?: (scene: FlowScene) => void
  /** Stored node coordinates (#185). */
  positions?: FlowPositions
  /** Where moved and laid-out coordinates go. The pane persists nothing. */
  onPositionsChange?: (positions: FlowPositions) => void
  presentation?: FlowPresentation
  /** What this level allows, and the sentence for each refusal. */
  authoring?: FlowAuthoring
  actions?: CanonicalFlowActions
  /** Which kinds the toolbar offers. Defaults to the canvas's own six. */
  creatable?: readonly FlowKind[]
  /** The unauthored way out this level offers. See `FlowGraphNode.spare`. */
  spare?: (element: FlowElement) => FlowPort | null
  /** Anything to say above the canvas — layout notices, scene-wide problems. */
  notes?: ReactNode
  /**
   * Every state this scene passes through, for whoever mounted the pane.
   *
   * The arc view (#187) needs it: entering a scene, editing it and going back
   * unmounts this pane, and without a copy of the edit outside it the writer's
   * unsaved work would be gone the moment they looked at the arc. Optional,
   * because the pane is perfectly usable as the whole of the Flow tab and was
   * for a version before there were two levels.
   */
  onSceneChange?: (scene: FlowScene) => void
}) {
  const sceneId = useUI((s) => s.narrative.sceneId)
  // One copy per mount. `councilHearing()` returns a fresh scene every call, so
  // building it in the render body would throw the writer's edits away on every
  // keystroke somewhere else in the workspace.
  const demo = useMemo(() => councilHearing(), [])

  if (source.kind === 'loading') {
    return (
      <div className="nrt-pane nrt-flow">
        <p className="nrt-note" aria-busy="true">
          <Icon name="clock" size="sm" />
          Reading the scene…
        </p>
      </div>
    )
  }

  if (source.kind === 'error') {
    return (
      <div className="nrt-pane nrt-flow">
        <p className="nrt-note inline-error" role="alert">
          Could not read this scene: {source.message}
        </p>
      </div>
    )
  }

  const scene = source.kind === 'demo' ? demo : source.scene
  if (!scene) {
    return (
      <div className="nrt-pane nrt-flow">
        <NarrativePlaceholder title="No scene to draw" reason={NARRATIVE_UNAVAILABLE.flow}>
          {sceneId && (
            <p className="nrt-note">
              Selected scene <code>{sceneId}</code>
            </p>
          )}
        </NarrativePlaceholder>
      </div>
    )
  }

  /*
   * Keyed by the scene.
   *
   * A different scene is a different editor: its own history, its own canvas
   * cursor, its own closed groups. Remounting on the key is React's own way of
   * saying that, and it is the reason nothing below has to synchronise a prop
   * into state in an effect.
   */
  return (
    <FlowSceneEditor
      key={scene.id}
      initial={scene}
      demo={source.kind === 'demo'}
      readOnly={readOnly}
      layout={layout}
      onSceneChange={onSceneChange}
      onEdit={onEdit}
      positions={positions}
      onPositionsChange={onPositionsChange}
      presentation={presentation}
      authoring={authoring}
      actions={actions}
      creatable={creatable}
      spare={spare}
      notes={notes}
    />
  )
}

/** Past, present and future in one value, so every update is a pure function. */
interface History {
  present: FlowScene
  past: FlowScene[]
  future: FlowScene[]
}

function FlowSceneEditor({
  initial,
  demo,
  readOnly,
  layout,
  onSceneChange,
  onEdit,
  positions,
  onPositionsChange,
  presentation,
  authoring,
  actions,
  creatable,
  spare,
  notes,
}: {
  initial: FlowScene
  demo: boolean
  readOnly: boolean
  layout?: LayoutRunner
  onSceneChange?: (scene: FlowScene) => void
  onEdit?: (scene: FlowScene) => void
  positions?: FlowPositions
  onPositionsChange?: (positions: FlowPositions) => void
  presentation?: FlowPresentation
  authoring?: FlowAuthoring
  actions?: CanonicalFlowActions
  creatable?: readonly FlowKind[]
  spare?: (element: FlowElement) => FlowPort | null
  notes?: ReactNode
}) {
  const [mode, setMode] = useState<FlowMode>('canvas')

  // This component is keyed by the scene, so mounting *is* a scene change: the
  // canvas cursor, the half-made connection and the closed groups all belonged
  // to the scene that just went away.
  useLayoutEffect(() => resetFlowStore(), [])

  // This history is used only by the standalone demonstration.

  const [history, setHistory] = useState<History>(() => ({
    present: initial,
    past: [],
    future: [],
  }))
  /*
   * Controlled or not, in one line.
   *
   * When somebody else owns the document, the scene on screen is the scene they
   * hand in — never a copy of it kept here. A copy would drift the moment a save
   * was refused: the canvas would go on showing an edit that is not in the file,
   * and the writer would find out when they reopened the project.
   */
  const controlled = onEdit !== undefined
  const scene = controlled ? initial : history.present
  const emptyRoot = useRef<HTMLDivElement>(null)
  useFlowReveal({
    scene,
    container: emptyRoot,
    projectKey: actions?.projectKey,
    enabled: scene.elements.length === 0,
  })

  /*
   * Reported through a ref rather than as an effect dependency.
   *
   * A caller that rebuilt its callback on every render would otherwise put this
   * effect in a loop, and the callers that want it are exactly the ones holding
   * the scene in state. Undo and redo go through here too, because they change
   * the scene as much as an edit does.
   */
  const report = useRef(onSceneChange)
  useEffect(() => {
    report.current = onSceneChange
  }, [onSceneChange])
  useEffect(() => report.current?.(scene), [scene])

  const change = (next: FlowScene) => {
    if (onEdit) return onEdit(next)
    setHistory((h) => ({ present: next, past: [...h.past, h.present], future: [] }))
  }

  const undo = () => {
    // Said as well as done: the canvas is the only thing that visibly changes,
    // and a reader who is not looking at it gets nothing otherwise.
    useFlowStore.getState().announce('Undone.')
    setHistory((h) => {
      const previous = h.past[h.past.length - 1]
      if (!previous) return h
      return { present: previous, past: h.past.slice(0, -1), future: [h.present, ...h.future] }
    })
  }

  const redo = () => {
    useFlowStore.getState().announce('Redone.')
    setHistory((h) => {
      const next = h.future[0]
      if (!next) return h
      return { present: next, past: [...h.past, h.present], future: h.future.slice(1) }
    })
  }

  if (scene.elements.length === 0) {
    return (
      <div ref={emptyRoot} className="nrt-pane nrt-flow">
        <div className="nrt-empty">
          <p>
            <b>{scene.name}</b> has no beats yet. A scene is a place where people speak and choices
            are made; the first thing in one is usually somebody arriving.
          </p>
          {readOnly ? (
            <p className="nrt-note">
              <Icon name="lock" size="sm" />
              This project folder is read-only, so nothing can be added to it here.
            </p>
          ) : (
            <div className="nrt-empty-actions">
              {controlled ? (
                /* A real, empty scene gets a real, empty beat. Pouring the
                   Ashfall fixture into somebody's document would write its
                   invented ids into their project, and no id in that fixture is
                   one the backend ever minted. */
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={actions?.disabled}
                  data-flow-empty
                  onClick={() =>
                    actions
                      ? actions.add('beat', null)
                      : change(addElement(scene, 'beat', null).scene)
                  }
                >
                  Add the first beat
                </button>
              ) : (
                <button
                  type="button"
                  className="btn btn-primary"
                  onClick={() =>
                    change({ ...scene, ...councilHearing(), id: scene.id, name: scene.name })
                  }
                >
                  Fill it with the Ashfall example
                </button>
              )}
            </div>
          )}
        </div>
      </div>
    )
  }

  return (
    <div className="nrt-pane nrt-flow">
      <div className="nrt-flow-head">
        <FlowModeTabs mode={mode} onMode={setMode} label="Flow view mode" />
        {controlled ? (
          /* No buttons of its own, on purpose. A saved edit is on the
             workspace's undo stack beside every other write, so a second pair
             here would be a second, disagreeing answer to the same keystroke. */
          <span className="nrt-note-inline">Undo and redo are the workspace’s own.</span>
        ) : (
          <>
            <button
              type="button"
              className="btn btn-sm"
              onClick={undo}
              disabled={history.past.length === 0}
            >
              Undo
            </button>
            <button
              type="button"
              className="btn btn-sm"
              onClick={redo}
              disabled={history.future.length === 0}
            >
              Redo
            </button>
          </>
        )}
        {readOnly && (
          <span className="nrt-badge">
            <Icon name="lock" size="sm" />
            Read-only
          </span>
        )}
      </div>

      {demo && (
        // Demonstration edits are held in memory.
        <p className="nrt-note" role="status">
          <Icon name="lock" size="sm" />
          Demonstration data. Everything below is a fixture of the Ashfall council hearing, held in
          memory: edits work, and none of them are saved.
        </p>
      )}

      {notes}

      {mode === 'canvas' ? (
        <FlowCanvas
          scene={scene}
          onChange={change}
          readOnly={readOnly}
          layout={layout}
          positions={positions}
          onPositionsChange={onPositionsChange}
          presentation={presentation}
          authoring={authoring}
          actions={actions}
          targetOf={actions ? flowTarget : undefined}
          creatable={creatable}
          spare={spare}
        />
      ) : (
        <FlowOutline
          scene={scene}
          presentation={presentation}
          onChange={change}
          readOnly={readOnly}
          authoring={authoring}
          actions={actions}
          targetOf={actions ? flowTarget : undefined}
          creatable={creatable}
          spare={spare}
        />
      )}
    </div>
  )
}
