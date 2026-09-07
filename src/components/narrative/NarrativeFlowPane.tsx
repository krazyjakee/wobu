import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useUI } from '../../store/ui'
import { Icon } from '../Icon'
import { NarrativePlaceholder } from './NarrativePlaceholder'
import { NARRATIVE_UNAVAILABLE } from './narrativeModel'
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
 * The branching canvas, and the contract it keeps with the rest of the
 * workspace.
 *
 * ── the seam ────────────────────────────────────────────────────────────────
 *
 * The canvas owns no selection of its own. There is one selection for the whole
 * narrative workspace, in `store/ui.ts`, and every surface reads and writes
 * that:
 *
 *   const selected = useUI((s) => s.narrative)          // { sceneId, beatId, lineId }
 *   const select = useUI((s) => s.selectNarrative)
 *   select({ sceneId, beatId: node.id }, 'flow')        // clicking a node
 *
 * Two rules that are easy to get wrong and expensive to discover later:
 *
 * 1. **Write the whole path.** `selectNarrative` takes the scene the beat is
 *    in, not just the beat. A beat id on its own would leave the previous
 *    scene's id above it and Script would open the wrong scene.
 * 2. **Never key a node by index or by position.** Node coordinates are
 *    presentation metadata; the selection is ids. A node that moves, is
 *    reordered or is renamed is the same node, and the selection must not
 *    notice.
 *
 * To scroll or centre on something chosen elsewhere, watch `narrativeReveal`.
 * It carries the same three ids plus a rising `seq` and the `origin` that
 * raised it — skip the ones whose origin is `'flow'`, which are the canvas's
 * own clicks coming back, and honour each `seq` once. It is latched rather than
 * consumed, so a canvas mounting on a tab switch still owes the last reveal.
 *
 * When an element is deleted, call `forgetNarrative([...ids])` so the selection
 * lets go of it instead of pointing at a node that is no longer drawn.
 *
 * All of that is honoured in `flow/useSceneEdits.ts` and `flow/FlowCanvas.tsx`.
 *
 * ── where the scene comes from, and who owns the edits ──────────────────────
 *
 * `source` is the seam. Without `onEdit` the pane owns the scene: it keeps a
 * history, its Undo and Redo buttons work, and nothing is written anywhere —
 * which is what the demonstration fixture and the canvas's own tests want.
 *
 * With `onEdit` the pane is **controlled**. It keeps no history at all, draws
 * exactly the scene it is given, and hands every structural change straight to
 * its owner. That is not a style choice: the owner is `NarrativeProjectFlow`,
 * which patches the real document and saves it through `useSaveScene` — so the
 * undo stack, the guarded-write precondition and the conflict card are the
 * workspace's own rather than a second implementation living in this file. A
 * local history beside them would be a second, disagreeing answer to ⌘Z.
 *
 * The "Demonstration data" banner is shown for `demo` and for nothing else.
 * A banner claiming a fixture while a writer's real scene is on screen would be
 * worse than no banner at all.
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
  authoring,
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
  /** What this level allows, and the sentence for each refusal. */
  authoring?: FlowAuthoring
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
      authoring={authoring}
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
  authoring,
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
  authoring?: FlowAuthoring
  creatable?: readonly FlowKind[]
  spare?: (element: FlowElement) => FlowPort | null
  notes?: ReactNode
}) {
  const [mode, setMode] = useState<FlowMode>('canvas')

  // This component is keyed by the scene, so mounting *is* a scene change: the
  // canvas cursor, the half-made connection and the closed groups all belonged
  // to the scene that just went away.
  useEffect(() => resetFlowStore(), [])

  /*
   * Undo, local for now.
   *
   * #186 asks for structural edits to join the app's undo stack so canvas and
   * form edits are indistinguishable in history. There is no narrative undo
   * stack to join — the model that would own one is being written elsewhere —
   * so this is a scene-shaped history in the pane. It is honest about its
   * scope: it undoes structure, it does not survive a tab switch, and the
   * moment a real stack exists this is the one place that changes.
   */
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
      <div className="nrt-pane nrt-flow">
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
                  onClick={() => change(addElement(scene, 'beat', null).scene)}
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
        // The honest label. This build has no narrative storage, so what is on
        // the canvas is a demonstration and nothing a writer does to it is kept.
        <p className="nrt-note" role="status">
          <Icon name="lock" size="sm" />
          Demonstration data. {NARRATIVE_UNAVAILABLE.source} Everything below is a fixture of the
          Ashfall council hearing, held in memory: edits work, and none of them are saved.
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
          authoring={authoring}
          creatable={creatable}
          spare={spare}
        />
      ) : (
        <FlowOutline
          scene={scene}
          onChange={change}
          readOnly={readOnly}
          authoring={authoring}
          creatable={creatable}
          spare={spare}
        />
      )}
    </div>
  )
}
