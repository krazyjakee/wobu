import { create } from 'zustand'
import { errorCode, errorMessage, errorSurface, isRetryable, isWobuError } from '../lib/api'
import type { NodeKind } from '../lib/api'

export type Mode = 'library' | 'forge' | 'assets' | 'settings'
export type EditorTab = 'notes' | 'refs' | 'concepts' | 'three' | 'relations'

export const EDITOR_TABS: EditorTab[] = ['notes', 'refs', 'concepts', 'three', 'relations']

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

export const TOAST_DURATION = {
  info: 4_200,
  error: 8_000,
} as const

export const useUI = create<UIState>((set) => ({
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
