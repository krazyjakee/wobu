import { create } from 'zustand'
import { errorCode, errorMessage, errorSurface, isRetryable, isWobuError } from '../lib/api'
import type { NodeKind } from '../lib/api'

export type Mode = 'library' | 'forge' | 'assets' | 'narrative' | 'settings'
export type EditorTab = 'notes' | 'refs' | 'concepts' | 'three' | 'relations'

export const EDITOR_TABS: EditorTab[] = ['notes', 'refs', 'concepts', 'three', 'relations']

/**
 * The Narrative workspace's centre tabs.
 *
 * Deliberately not `EditorTab`. The two sets are over different things — one
 * shows an entity's notes, the other a scene's structure — and sharing a field
 * would mean ⌘3 in the Library left the narrative workspace sitting on Preview.
 */
export type NarrativeTab = 'flow' | 'script' | 'preview' | 'source'

export const NARRATIVE_TABS: NarrativeTab[] = ['flow', 'script', 'preview', 'source']

/** The Library navigator's three work filters, from the #151 mockup. */
export type NarrativeFilter = 'needsText' | 'needsReview' | 'outOfDate'

/**
 * What the Narrative workspace is pointing at — as stable ids, and nothing else.
 *
 * Flow, Script, the inspector, the Library, search results and diagnostics all
 * read and write this one value, which is why selecting a beat on the canvas
 * selects the same beat in Script without either of them knowing the other
 * exists. Anything that needs a name, a position or a status looks it up from
 * the id; none of that is copied in here, so a rename is invisible to the
 * selection and cannot leave a stale label behind.
 *
 * Ids rather than indexes or coordinates, because in a narrative both move
 * constantly: beats are reordered, a node is dragged across the canvas, a scene
 * gains a branch above the one you were reading. "The third beat" follows none
 * of that. `docs`-level identity rules are #152's; this is the UI half of the
 * same promise.
 *
 * The three ids are one *path*, not three independent selections — a beat only
 * means something inside its scene. `selectNarrative` therefore takes the whole
 * path, so a beat from the previous scene can never survive under a new one.
 */
export interface NarrativeSelection {
  choiceId?: string | null
  outcomeId?: string | null
  variantId?: string | null
  sceneId: string | null
  beatId: string | null
  lineId: string | null
}

/** Which surface asked, so a surface can skip a reveal it caused itself. */
export type NarrativeOrigin =
  'library' | 'flow' | 'script' | 'inspector' | 'search' | 'diagnostic' | 'preview'

export type NarrativeField =
  | 'name'
  | 'entry'
  | 'setting'
  | 'participants'
  | 'title'
  | 'intent'
  | 'condition'
  | 'effects'
  | 'destination'
  | 'label'
  | 'speaker'
  | 'text'
  | 'act'
  | 'arc'
  | 'tags'

export interface NarrativeWorldTarget {
  projectKey: string
  collection: 'facts' | 'knowledge' | 'relationships' | 'events' | 'quests' | 'restrictions'
  recordId: string
  seq: number
}

export interface NarrativeRevealOptions {
  projectKey?: string
  focus?: boolean
}

/**
 * A request to bring the selection into view.
 *
 * Separate from the selection because "what is selected" is a state and "scroll
 * to it" is an event: a canvas must not re-centre on every render, only when
 * something asked it to. `seq` rises on every request, so asking twice for the
 * same beat scrolls twice — which is what a user who clicks the same diagnostic
 * again is asking for.
 *
 * Latched rather than consumed. A tab that was unmounted when the request was
 * made still owes the scroll when the user switches to it, so the request stays
 * readable and each surface remembers the last `seq` it honoured.
 */
export interface NarrativeReveal extends NarrativeSelection {
  projectKey: string | null
  variantId: string | null
  choiceId: string | null
  outcomeId: string | null
  field: NarrativeField | null
  focus: boolean
  seq: number
  origin: NarrativeOrigin
}

/** The argument to `selectNarrative`: deeper ids left out are cleared. */
export interface NarrativeTarget {
  variantId?: string | null
  choiceId?: string | null
  outcomeId?: string | null
  field?: NarrativeField | null
  sceneId: string | null
  beatId?: string | null
  lineId?: string | null
}

export interface Toast {
  id: number
  /** Incremented in place so a changed toast can restart its own lifetime. */
  revision: number
  text: string
  kind: 'info' | 'error'
  detail?: string
  /** An affordance for a failure the user can recover from. */
  action?: { label: string; run: () => void }
  /** Persistent toasts remain until their action runs or the user dismisses them. */
  persistent: boolean
  durationMs: number
}

export type ToastOptions = Partial<Pick<Toast, 'detail' | 'action' | 'persistent' | 'durationMs'>>
export type ToastUpdate = Partial<
  Pick<Toast, 'text' | 'kind' | 'detail' | 'action' | 'persistent' | 'durationMs'>
>

/**
 * A condition that makes the workspace untrustworthy until it is resolved —
 * the share unmounted, the folder is read-only. Unlike a toast this does not
 * time out, because the thing it describes has not gone away.
 *
 * Keyed by error code rather than by a sequence number: a share that unmounts
 * fails every read under it, and twenty identical banners is a worse bug than
 * no banner. The newest wins so the wording stays current.
 */
export interface Banner {
  code: string
  text: string
  detail?: string
  retryable: boolean
  /** An affordance rendered inside the banner, when there is something to do. */
  action?: { label: string; run: () => void }
  /** Hides the dismiss button. For a banner the user must answer, not read. */
  sticky?: boolean
}

const NAV_MIN = 200
const NAV_MAX = 460

/**
 * How many nodes the navigator's Recent section remembers.
 *
 * Short on purpose. Recent is for the handful of entities in play right now —
 * long enough to cover a morning's back-and-forth between a species, its
 * culture and two characters, short enough that it never becomes a second list
 * to scroll.
 */
const RECENT_LIMIT = 8

interface UIState {
  mode: Mode
  setMode: (m: Mode) => void

  selectedId: string | null
  select: (id: string | null) => void
  /** Most recently opened first, current selection included, capped. */
  recentIds: string[]

  tab: EditorTab
  setTab: (t: EditorTab) => void

  /**
   * The one narrative selection every narrative surface shares, and the tab and
   * filters around it.
   *
   * All of it is machine-local, exactly as `navWidth` and the collapse state
   * above are: which tab this person left open is a fact about their screen,
   * not about the story, and writing it into the project folder would push one
   * author's workspace onto everybody who opens the world.
   */
  narrative: NarrativeSelection
  narrativeEntityTarget: { projectKey: string; entityId: string; seq: number } | null
  openNarrativeEntity: (projectKey: string, entityId: string) => void
  finishNarrativeEntityReveal: (seq: number) => void
  narrativeWorldTarget: NarrativeWorldTarget | null
  openNarrativeWorld: (target: Omit<NarrativeWorldTarget, 'seq'>) => void
  narrativeReveal: NarrativeReveal | null
  selectNarrative: (
    target: NarrativeTarget,
    origin: NarrativeOrigin,
    options?: NarrativeRevealOptions,
  ) => void
  forgetNarrative: (ids: readonly string[]) => void

  narrativeTab: NarrativeTab
  setNarrativeTab: (t: NarrativeTab) => void

  narrativeFilters: Record<NarrativeFilter, boolean>
  toggleNarrativeFilter: (f: NarrativeFilter) => void

  filter: string
  setFilter: (v: string) => void

  navWidth: number
  setNavWidth: (w: number) => void
  navCollapsed: boolean
  inspCollapsed: boolean
  toggleNav: () => void
  toggleInsp: () => void

  closedGroups: Record<string, true>
  toggleGroup: (kind: NodeKind) => void

  collapsedNodes: Record<string, true>
  toggleNodeOpen: (id: string) => void
  openAncestors: (ids: string[]) => void

  /**
   * Open/closed for the navigator's headings that are neither a kind group nor
   * a node — its sections and its letter index. One record rather than two
   * because the defaults differ per kind of heading and the caller, which knows
   * which heading it drew, passes the state it wants rather than a toggle.
   */
  bands: Record<string, boolean>
  setBandOpen: (key: string, open: boolean) => void
  collapseAll: (kinds: NodeKind[], bandKeys: string[]) => void
  expandAll: () => void

  paletteOpen: boolean
  setPaletteOpen: (v: boolean) => void

  /** The keyboard reference. Ephemeral like the palette; the bindings are not. */
  shortcutsOpen: boolean
  setShortcutsOpen: (v: boolean) => void

  toasts: Toast[]
  pushToast: (text: string, kind?: Toast['kind'], options?: ToastOptions) => number
  updateToast: (id: number, update: ToastUpdate) => void
  dropToast: (id: number) => void

  banners: Banner[]
  raiseBanner: (b: Banner) => void
  clearBanner: (code: string) => void
  clearBanners: () => void
}

let toastSeq = 0
let narrativeSeq = 0

/** Nothing selected. One instance, so "unchanged" stays an identity check. */
const NO_NARRATIVE: NarrativeSelection = { sceneId: null, beatId: null, lineId: null }

function samePath(a: NarrativeSelection, b: NarrativeSelection): boolean {
  return (
    a.sceneId === b.sceneId &&
    a.beatId === b.beatId &&
    a.lineId === b.lineId &&
    (a.choiceId ?? null) === (b.choiceId ?? null) &&
    (a.outcomeId ?? null) === (b.outcomeId ?? null) &&
    (a.variantId ?? null) === (b.variantId ?? null)
  )
}

/**
 * Drop the levels that no longer exist, keeping their surviving ancestors.
 *
 * A forgotten beat takes its line with it even when nobody named the line: the
 * line was inside the beat, so it went too. Returns the same object when
 * nothing was hit, which is what lets `forgetNarrative` skip the write.
 */
function prunePath(sel: NarrativeSelection, gone: Set<string>): NarrativeSelection {
  if (sel.sceneId !== null && gone.has(sel.sceneId)) return NO_NARRATIVE
  if (sel.beatId !== null && gone.has(sel.beatId)) {
    return { sceneId: sel.sceneId, beatId: null, lineId: null }
  }
  const next = { ...sel }
  if (sel.lineId !== null && gone.has(sel.lineId)) {
    next.lineId = null
    delete next.variantId
  }
  for (const key of ['choiceId', 'outcomeId', 'variantId'] as const)
    if (next[key] && gone.has(next[key]!)) delete next[key]
  return samePath(sel, next) ? sel : next
}

export const TOAST_DURATION = {
  info: 4_200,
  error: 8_000,
} as const

export const useUI = create<UIState>((set, get) => ({
  mode: 'library',
  setMode: (mode) => set({ mode }),

  selectedId: null,
  // Every route into a node — a row, the palette, a breadcrumb, a backlink —
  // goes through `select`, so the recent list is kept here rather than at the
  // call sites, where one forgotten path would make it quietly wrong.
  recentIds: [],
  select: (selectedId) =>
    set((s) => ({
      selectedId,
      recentIds: selectedId
        ? [selectedId, ...s.recentIds.filter((id) => id !== selectedId)].slice(0, RECENT_LIMIT)
        : s.recentIds,
    })),

  tab: 'notes',
  setTab: (tab) => set({ tab }),

  narrative: NO_NARRATIVE,
  narrativeEntityTarget: null,
  openNarrativeEntity: (projectKey, entityId) => {
    get().select(entityId)
    set({
      mode: 'library',
      narrativeEntityTarget: { projectKey, entityId, seq: ++narrativeSeq },
    })
  },
  finishNarrativeEntityReveal: (seq) =>
    set((state) =>
      state.narrativeEntityTarget?.seq === seq ? { narrativeEntityTarget: null } : {},
    ),
  narrativeWorldTarget: null,
  openNarrativeWorld: (target) =>
    set({ mode: 'narrative', narrativeWorldTarget: { ...target, seq: ++narrativeSeq } }),
  narrativeReveal: null,
  // The single writer for every route into a narrative element — a Library row,
  // a Flow node, a Script line, a search hit, a diagnostic. Each of them hands
  // over the whole path and says who it is; nothing sets `narrative` directly,
  // because a half-written path is exactly how Flow and Script would come to
  // disagree about what is selected.
  selectNarrative: (target, origin, options) =>
    set((state) => {
      const next: NarrativeSelection = {
        sceneId: target.sceneId,
        beatId: target.sceneId ? (target.beatId ?? null) : null,
        lineId:
          target.sceneId && target.beatId && !target.choiceId && !target.outcomeId
            ? (target.lineId ?? null)
            : null,
        ...(target.sceneId && target.beatId
          ? target.choiceId
            ? { choiceId: target.choiceId }
            : target.outcomeId
              ? { outcomeId: target.outcomeId }
              : target.lineId && target.variantId
                ? { variantId: target.variantId }
                : {}
          : {}),
      }
      // The previous object back when nothing actually moved. A canvas
      // subscribed to `narrative` re-runs its layout when the value changes,
      // and re-selecting the row you are already on must not cost that.
      const narrative = samePath(state.narrative, next) ? state.narrative : next
      return {
        narrative,
        narrativeReveal: {
          ...next,
          seq: ++narrativeSeq,
          origin,
          projectKey: options?.projectKey ?? null,
          focus: options?.focus ?? true,
          variantId: target.variantId ?? null,
          choiceId: target.choiceId ?? null,
          outcomeId: target.outcomeId ?? null,
          field: target.field ?? null,
        },
      }
    }),
  // What a deletion does to a selection. Only ids are held here, so a removed
  // element leaves a live-looking id pointing at nothing; whoever owns the
  // model says what went away. Surviving ancestors are kept — deleting a line
  // should leave the writer on its beat rather than throw them back to the
  // project — and a lost scene clears the path outright, because a beat outside
  // its scene is not somewhere the workspace can show.
  forgetNarrative: (ids) =>
    set((state) => {
      const gone = new Set(ids)
      if (gone.size === 0) return {}
      const narrative = prunePath(state.narrative, gone)
      const reveal = state.narrativeReveal
      const stale =
        reveal !== null &&
        [
          reveal.sceneId,
          reveal.beatId,
          reveal.lineId,
          reveal.variantId,
          reveal.choiceId,
          reveal.outcomeId,
        ].some((id) => id !== null && gone.has(id))
      if (narrative === state.narrative && !stale) return {}
      return { narrative, narrativeReveal: stale ? null : reveal }
    }),

  narrativeTab: 'flow',
  // Only the tab moves. The selection is a sibling field precisely so that
  // reading a beat on the canvas and then opening Script lands on the same
  // beat rather than at the top of the scene.
  setNarrativeTab: (narrativeTab) => set({ narrativeTab }),

  narrativeFilters: { needsText: false, needsReview: false, outOfDate: false },
  toggleNarrativeFilter: (f) =>
    set((state) => ({
      narrativeFilters: { ...state.narrativeFilters, [f]: !state.narrativeFilters[f] },
    })),

  filter: '',
  setFilter: (filter) => set({ filter }),

  navWidth: 272,
  setNavWidth: (w) => set({ navWidth: Math.min(NAV_MAX, Math.max(NAV_MIN, Math.round(w))) }),
  navCollapsed: false,
  inspCollapsed: false,
  toggleNav: () => set((s) => ({ navCollapsed: !s.navCollapsed })),
  toggleInsp: () => set((s) => ({ inspCollapsed: !s.inspCollapsed })),

  closedGroups: {},
  toggleGroup: (kind) =>
    set((s) => {
      const next = { ...s.closedGroups }
      if (next[kind]) delete next[kind]
      else next[kind] = true
      return { closedGroups: next }
    }),

  collapsedNodes: {},
  toggleNodeOpen: (id) =>
    set((s) => {
      const next = { ...s.collapsedNodes }
      if (next[id]) delete next[id]
      else next[id] = true
      return { collapsedNodes: next }
    }),
  openAncestors: (ids) =>
    set((s) => {
      if (!ids.length) return {}
      const next = { ...s.collapsedNodes }
      let changed = false
      for (const id of ids) {
        if (next[id]) {
          delete next[id]
          changed = true
        }
      }
      return changed ? { collapsedNodes: next } : {}
    }),

  bands: {},
  setBandOpen: (key, open) => set((s) => ({ bands: { ...s.bands, [key]: open } })),
  // Closing the kind groups hides everything under them, so the node and letter
  // state below is left exactly as it was: re-opening a group should return the
  // reader to the shape they had, not to a wall of every branch expanded.
  collapseAll: (kinds, bandKeys) =>
    set((s) => {
      const closedGroups: Record<string, true> = { ...s.closedGroups }
      for (const kind of kinds) closedGroups[kind] = true
      const bands = { ...s.bands }
      for (const key of bandKeys) bands[key] = false
      return { closedGroups, bands }
    }),
  // Groups, sections and branches all open; the letter index returns to its
  // default, which is closed. The index is the *shape* of an oversized group
  // rather than something collapsed inside it, and pouring nine hundred names
  // back onto the screen is the state this restructure exists to end (#145).
  expandAll: () => set({ closedGroups: {}, collapsedNodes: {}, bands: {} }),

  paletteOpen: false,
  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),

  shortcutsOpen: false,
  setShortcutsOpen: (shortcutsOpen) => set({ shortcutsOpen }),

  toasts: [],
  pushToast: (text, kind = 'info', options = {}) => {
    const id = ++toastSeq
    set((s) => ({
      toasts: [
        ...s.toasts,
        {
          id,
          revision: 0,
          text,
          kind,
          detail: options.detail,
          action: options.action,
          persistent: options.persistent ?? Boolean(options.detail || options.action),
          durationMs: options.durationMs ?? TOAST_DURATION[kind],
        },
      ],
    }))
    return id
  },
  updateToast: (id, update) =>
    set((s) => ({
      toasts: s.toasts.map((toast) => {
        if (toast.id !== id) return toast
        const next = { ...toast, ...update, revision: toast.revision + 1 }
        if (update.kind && update.durationMs === undefined) {
          next.durationMs = TOAST_DURATION[update.kind]
        }
        if (
          update.persistent === undefined &&
          (update.action !== undefined || update.detail !== undefined)
        ) {
          next.persistent = Boolean(next.action || next.detail)
        }
        return next
      }),
    })),
  dropToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

  banners: [],
  raiseBanner: (b) => set((s) => ({ banners: [...s.banners.filter((x) => x.code !== b.code), b] })),
  clearBanner: (code) => set((s) => ({ banners: s.banners.filter((b) => b.code !== code) })),
  clearBanners: () => set({ banners: [] }),
}))

/** Convenience for imperative call-sites (mutation handlers). */
export const toast = (text: string, kind: Toast['kind'] = 'info', options?: ToastOptions): number =>
  useUI.getState().pushToast(text, kind, options)

/**
 * Report a failed command on whichever surface its code calls for.
 *
 * Call sites should not be choosing between `toast` and `raiseBanner`
 * themselves — that decision belongs to the code, in one place, or it drifts
 * per handler. `report` is what every `onError` should use.
 */
export function report(e: unknown, prefix?: string): void {
  const surface = errorSurface(e)
  // A cancellation is not a failure and gets no UI at all. Handled here rather
  // than at the call sites so that nobody has to remember it.
  if (surface === 'silent') return

  const text = prefix ? `${prefix} — ${errorMessage(e)}` : errorMessage(e)
  if (surface === 'banner') {
    useUI.getState().raiseBanner({
      code: errorCode(e) ?? 'internal',
      text,
      detail: isWobuError(e) ? e.detail : undefined,
      retryable: isRetryable(e),
    })
  } else {
    const detail = isWobuError(e) ? e.detail : undefined
    useUI.getState().pushToast(text, 'error', {
      detail,
      // A retryable command needs a durable surface even when the caller has
      // no safe, self-contained retry function to offer here. It remains
      // dismissible, but never vanishes while the user is deciding what to do.
      persistent: isRetryable(e) || Boolean(detail),
    })
  }
}
