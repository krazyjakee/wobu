import { createContext, useContext } from 'react'
import { create, useStore } from 'zustand'
import { ALL_BADGES, type BadgeFilter } from './badges'
import type { FlowDiagnosticCategory } from './model'

/**
 * The canvas's own ephemeral state — which box is current, which half-made
 * connection is in flight, which groups are closed, where the plane is
 * scrolled to, and what was last said out loud.
 *
 * Why a store rather than component state, twice over:
 *
 * 1. **Selection must not be a prop.** The spike measured React Flow's own
 *    selection: pushing `selected` through the `nodes` array rewrites every
 *    node object, which cost 26 ms at 300 nodes and 83 ms at 1,000 in jsdom.
 *    Every node component therefore *subscribes* to `selectedId` here and
 *    re-renders alone. The `nodes` array is untouched by a click.
 * 2. **A pending connection cannot live in a ref.** The keyboard connector runs
 *    per node; a ref inside it would give every node its own copy and the
 *    second keypress would never see the first. This is the exact mistake the
 *    spike says #186 should not rediscover.
 *
 * What is *not* here: the narrative selection. That is one value for the whole
 * workspace and it lives in `store/ui.ts`, where Script, the Library and the
 * inspector can all reach it. `selectedId` below is a canvas cursor — it points
 * at a node, which may be a choice or an outcome rather than a beat — and the
 * canvas mirrors it into the shared selection whenever it moves.
 *
 * `closedGroups` and `viewport` are presentation metadata in the same sense
 * node positions are (#185). They are held here and handed out through setters,
 * so a later step can load and save them without any component changing.
 *
 * ── one store per level ──────────────────────────────────────────────────────
 *
 * Flow is nested: an arc of scenes, and inside one scene its beats and choices
 * (#187). Both levels draw with the same components, so both need all of the
 * above — and they must not share it. Entering a scene sets the cursor to a
 * beat; if that overwrote the arc's cursor, leaving again would land the writer
 * nowhere, and the arc's closed quests and viewport would be gone with it.
 *
 * So this is a *factory* with one instance per level, handed down through
 * `FlowStoreContext`. The default context value is the scene level's store, so
 * a component mounted with no provider behaves exactly as it did before there
 * were two levels — which is what keeps the scene canvas and its tests
 * untouched by any of this. The arc mounts its own store around its canvas, and
 * because the store outlives the component that reads it, leaving a scene and
 * returning to the arc restores the cursor, the closed quests and the viewport
 * without anything having to save and reload them.
 */

export interface FlowConnection {
  elementId: string
  portId: string
  /**
   * What to call the port if it has to be created.
   *
   * Set only when the connection came from a node's *spare* handle — the one
   * standing in for a way out that has not been authored yet. Everywhere else
   * the port already exists and already has a name.
   */
  label?: string
}

/**
 * Where the plane is scrolled and zoomed to.
 *
 * Structural, not React Flow's own type, because nothing else in this file
 * knows the canvas library exists — and because it is stored and restored
 * across an unmount, which is a fact about levels rather than about React Flow.
 */
export interface FlowViewport {
  x: number
  y: number
  zoom: number
}

export interface FlowState {
  /** The node the canvas considers current. Null when nothing is chosen. */
  selectedId: string | null
  select: (id: string | null) => void

  /** The source half of a keyboard connection, waiting for its target. */
  connectFrom: FlowConnection | null
  beginConnect: (from: FlowConnection | null) => void

  closedGroups: string[]
  toggleGroup: (id: string) => void
  setClosedGroups: (ids: readonly string[]) => void

  /**
   * Show only the beats one character is in. A filter *mutes* rather than
   * removes: dropping a node would break the routes through it, and a canvas
   * whose edges lead into nothing is a worse answer than a dimmed box.
   *
   * Read by each node component, like the selection and for the same reason.
   */
  participant: string | null
  setParticipant: (name: string | null) => void

  /**
   * Which diagnostic badges this canvas is drawing.
   *
   * Here, and not in `store/ui.ts` or anywhere near a project file, because a
   * badge is a derived view: #189 requires that toggling one writes nothing,
   * and the way to guarantee that is for the switch to live in memory that dies
   * with the pane. Per level for the same reason the cursor is: the arc's
   * categories are not the scene's.
   *
   * Errors are not in here at all. `badgeShown` returns true for one before it
   * looks at this value, so there is no state a release-blocking finding could
   * be hidden by — not because the UI never offers it, but because there is
   * nothing to offer.
   */
  badges: BadgeFilter
  setBadgeWarnings: (on: boolean) => void
  toggleBadgeCategory: (category: FlowDiagnosticCategory) => void

  /**
   * The last viewport this level was left at, so entering a scene and coming
   * back does not dump the writer at the origin of a graph they had panned
   * across. Null until the plane has been moved at least once.
   */
  nodeReveal: { id: string; seq: number } | null
  requestNodeReveal: (id: string | null) => void

  outlinePage: number
  setOutlinePage: (page: number) => void

  viewport: FlowViewport | null
  setViewport: (viewport: FlowViewport | null) => void

  /**
   * The last thing worth saying to a screen reader. Carries a sequence number
   * so repeating a message still counts as a change to the live region.
   */
  announcement: { text: string; seq: number }
  announce: (text: string) => void
}

/**
 * Shared across every level on purpose: the sequence number only has to rise,
 * and a per-level counter would let two levels emit the same `seq` and leave a
 * live region believing nothing had changed.
 */
let announceSeq = 0

export function createFlowStore() {
  return create<FlowState>((set) => ({
    selectedId: null,
    select: (selectedId) => set({ selectedId }),

    connectFrom: null,
    beginConnect: (connectFrom) => set({ connectFrom }),

    closedGroups: [],
    toggleGroup: (id) =>
      set((state) => ({
        closedGroups: state.closedGroups.includes(id)
          ? state.closedGroups.filter((other) => other !== id)
          : [...state.closedGroups, id],
      })),
    setClosedGroups: (ids) => set({ closedGroups: [...ids] }),

    participant: null,
    setParticipant: (participant) => set({ participant }),

    badges: ALL_BADGES,
    setBadgeWarnings: (on) => set((state) => ({ badges: { ...state.badges, warnings: on } })),
    toggleBadgeCategory: (category) =>
      set((state) => ({
        badges: {
          ...state.badges,
          categories: {
            ...state.badges.categories,
            [category]: !state.badges.categories[category],
          },
        },
      })),

    nodeReveal: null,
    requestNodeReveal: (id) => set({ nodeReveal: id ? { id, seq: ++announceSeq } : null }),
    outlinePage: 0,
    setOutlinePage: (outlinePage) => set({ outlinePage }),

    viewport: null,
    setViewport: (viewport) => set({ viewport }),

    announcement: { text: '', seq: 0 },
    announce: (text) => set({ announcement: { text, seq: ++announceSeq } }),
  }))
}

export type FlowStoreApi = ReturnType<typeof createFlowStore>

/** The scene level's store, and the default for anything mounted without one. */
export const useFlowStore = createFlowStore()

/**
 * Which level's store the components below are drawing.
 *
 * Defaulted rather than required, so `FlowCanvas`, `FlowOutline` and every node
 * face keep working with no provider at all — the scene canvas and its sixty
 * tests never mention this.
 */
export const FlowStoreContext = createContext<FlowStoreApi>(useFlowStore)

/** Subscribe to the current level's store. The one hook node faces use. */
export function useFlowLevel<T>(selector: (state: FlowState) => T): T {
  return useStore(useContext(FlowStoreContext), selector)
}

/** The current level's store itself, for reads and writes outside a render. */
export function useFlowLevelApi(): FlowStoreApi {
  return useContext(FlowStoreContext)
}

/** Everything back to first mount. Used when a level swaps what it is drawing. */
export function resetFlowStore(store: FlowStoreApi = useFlowStore) {
  store.setState({
    selectedId: null,
    connectFrom: null,
    closedGroups: [],
    participant: null,
    nodeReveal: null,
    outlinePage: 0,
    badges: ALL_BADGES,
    viewport: null,
    announcement: { text: '', seq: ++announceSeq },
  })
}
