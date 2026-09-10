import type { CanonicalFlowActions } from './canonicalFlow'
import { useCallback, useMemo } from 'react'
import { useUI, type NarrativeTarget } from '../../../store/ui'
import { useFlowLevelApi, type FlowConnection } from './flowStore'
import {
  addElement,
  connectPort,
  FLOW_KIND_LABEL,
  removeElement,
  type FlowElement,
  type FlowKind,
  type FlowLevel,
} from './model'

/**
 * The five things a writer can do to a scene's structure, in one place.
 *
 * Both editors use this: the canvas and the outline list. #151 requires the
 * outline to offer every operation the canvas does, and the way to make that
 * true rather than aspirational is for there to be one implementation of each
 * operation and two views of it. A bug fixed in one is fixed in both, and a new
 * operation cannot be added to the canvas alone.
 *
 * Selection is written here too, which is what keeps the two views agreeing
 * with Script and the inspector: `selectNarrative` gets the whole path — the
 * scene *and* the beat — because a beat id under the previous scene's id would
 * open the wrong scene.
 *
 * Both *levels* use it as well (#187). An arc is a level whose elements are
 * whole scenes, so "add", "connect", "remove" and "select" mean the same five
 * things there, and #187's requirement that creating a scene on the canvas
 * produce the same model change as the form path is true by construction
 * rather than by two implementations agreeing. The only thing that differs is
 * what a selected element *is* on the shared narrative path, which is what
 * `targetOf` is for.
 */
/**
 * What this level is allowed to have done to it, and the words for each refusal.
 *
 * A canvas drawing a fixture can do anything, because nothing is written. A
 * canvas drawing a real scene document cannot: the source model has three kinds
 * of destination and no "nowhere", a beat's own choices and outcomes belong to
 * it rather than being wired to it, and an ending is a picture of a field
 * rather than a box with an identity. Every one of those is a real property of
 * `wobu-narrative`, so each refusal carries the reason rather than a shrug —
 * see `flowAuthoring` in `source.ts`, which is where the sentences live.
 *
 * Refusing out loud rather than doing nothing is the point. A gesture that
 * appears to work and changes nothing is the worst of the three options, and it
 * is what a silent guard produces.
 */
export interface FlowAuthoring {
  /** Why *every* structural edit is refused here, or null when they are allowed. */
  refuse?: string | null
  /** Why a destination cannot be cleared, or null when clearing is fine. */
  destinationRequired?: string | null
  /** Why a structural port cannot be re-wired, or null when it can. */
  fixedPort?: string | null
  /** Why a derived box cannot be deleted, or null when it can. */
  derived?: string | null
}

export function useSceneEdits({
  scene,
  onChange,
  readOnly,
  authoring,
  actions,
  targetOf = beatTarget,
}: {
  scene: FlowLevel
  onChange: (scene: FlowLevel) => void
  readOnly: boolean
  /** Absent means "anything goes", which is what a fixture wants. */
  authoring?: FlowAuthoring
  actions?: CanonicalFlowActions
  /**
   * The shared narrative path a selected element means.
   *
   * Inside a scene, an element is a beat of the scene being edited. On the arc,
   * the element *is* a scene, so it names itself and no beat — and getting this
   * wrong would put the arc's id where Script expects a scene's.
   */
  targetOf?: (level: FlowLevel, element: FlowElement | null) => NarrativeTarget
}) {
  const store = useFlowLevelApi()
  const selectNarrative = useUI((s) => s.selectNarrative)
  const forgetNarrative = useUI((s) => s.forgetNarrative)

  const edit = useCallback(
    (next: FlowLevel): boolean => {
      if (readOnly) {
        store.getState().announce('This project folder is read-only.')
        return false
      }
      if (authoring?.refuse) {
        store.getState().announce(authoring.refuse)
        return false
      }
      onChange(next)
      return true
    },
    [authoring, onChange, readOnly, store],
  )

  const select = useCallback(
    (id: string | null) => {
      store.getState().select(id)
      const element = id === null ? null : (scene.elements.find((e) => e.id === id) ?? null)
      selectNarrative(targetOf(scene, element), 'flow', { focus: false })
    },
    [scene, selectNarrative, store, targetOf],
  )

  /**
   * Point a port somewhere, and say why not when the model cannot say it.
   *
   * Returns whether the edit happened, because the keyboard layer announces the
   * result and "Destination removed" after a refusal would be a lie told to the
   * one reader who cannot see the canvas.
   */
  const connect = useCallback(
    (from: FlowConnection, to: string | null): boolean => {
      if (actions) {
        if (readOnly) {
          store
            .getState()
            .announce('Scene editing is currently unavailable in this read-only view.')
          return false
        }
        return actions.connect(from, to)
      }
      const element = scene.elements.find((one) => one.id === from.elementId)
      const port = element?.out.find((one) => one.id === from.portId)
      if (port?.fixed && authoring?.fixedPort) {
        store.getState().announce(authoring.fixedPort)
        return false
      }
      if (to === null && authoring?.destinationRequired) {
        store.getState().announce(authoring.destinationRequired)
        return false
      }
      return edit(connectPort(scene, from, to))
    },
    [actions, readOnly, authoring, edit, scene, store],
  )

  const remove = useCallback(
    (id: string) => {
      if (actions) {
        if (readOnly) {
          store
            .getState()
            .announce('Scene editing is currently unavailable in this read-only view.')
          return false
        }
        return actions.remove(id)
      }
      const gone = scene.elements.find((element) => element.id === id)
      if (gone?.derived && authoring?.derived) {
        store.getState().announce(authoring.derived)
        return false
      }
      if (!edit(removeElement(scene, id))) return false
      /*
       * The selection must let go of an id nothing draws any more — and of
       * *both* of this element's ids.
       *
       * A canvas node is keyed by its layout name (`beat:01J…`) while the
       * shared selection holds the bare narrative id, so forgetting only the
       * one the canvas knows would leave Script and the inspector pointing at a
       * beat that no longer exists.
       */
      forgetNarrative(gone?.beatId ? [id, gone.beatId] : [id])
      if (store.getState().selectedId === id) store.getState().select(null)
      store
        .getState()
        .announce(`Deleted ${gone?.title ?? id}. Every route into it now has no destination.`)
      return true
    },
    [actions, readOnly, authoring, edit, scene, forgetNarrative, store],
  )

  const add = useCallback(
    (kind: FlowKind, afterId: string | null) => {
      if (actions) {
        if (readOnly) {
          store
            .getState()
            .announce('Scene editing is currently unavailable in this read-only view.')
          return
        }
        actions.add(kind, afterId)
        return
      }
      // Wired in through the anchor's first *spare* port, never by displacing a
      // destination the writer already chose.
      const result = addElement(scene, kind, afterId)
      if (!edit(result.scene)) return
      store.getState().select(result.element.id)
      selectNarrative(targetOf(result.scene, result.element), 'flow')
      store.getState().announce(`Added ${FLOW_KIND_LABEL[kind].toLocaleLowerCase()}.`)
    },
    [actions, readOnly, edit, scene, selectNarrative, store, targetOf],
  )

  return useMemo(() => ({ select, connect, remove, add }), [select, connect, remove, add])
}

/** The scene level's answer: the level is the scene, the element hangs off a beat. */
function beatTarget(level: FlowLevel, element: FlowElement | null): NarrativeTarget {
  return { sceneId: level.id, beatId: element?.beatId ?? null }
}
