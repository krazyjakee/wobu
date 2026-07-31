import { create } from 'zustand'
import { errorCode, errorMessage, errorSurface, isRetryable, isWobuError } from '../lib/api'
import type { NodeKind } from '../lib/api'

export type Mode = 'library' | 'board' | 'forge' | 'assets' | 'settings'
export type EditorTab = 'notes' | 'refs' | 'concepts' | 'three' | 'relations'

export const EDITOR_TABS: EditorTab[] = ['notes', 'refs', 'concepts', 'three', 'relations']

export interface Toast {
  id: number
  text: string
  kind: 'info' | 'error'
}

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

interface UIState {
  mode: Mode
  setMode: (m: Mode) => void

  selectedId: string | null
  select: (id: string | null) => void

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

  paletteOpen: boolean
  setPaletteOpen: (v: boolean) => void

  toasts: Toast[]
  pushToast: (text: string, kind?: Toast['kind']) => void
  dropToast: (id: number) => void

  banners: Banner[]
  raiseBanner: (b: Banner) => void
  clearBanner: (code: string) => void
  clearBanners: () => void
}

let toastSeq = 0

export const useUI = create<UIState>((set) => ({
  mode: 'library',
  setMode: (mode) => set({ mode }),

  selectedId: null,
  select: (selectedId) => set({ selectedId }),

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

  paletteOpen: false,
  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),

  toasts: [],
  pushToast: (text, kind = 'info') =>
    set((s) => ({ toasts: [...s.toasts, { id: ++toastSeq, text, kind }] })),
  dropToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

  banners: [],
  raiseBanner: (b) => set((s) => ({ banners: [...s.banners.filter((x) => x.code !== b.code), b] })),
  clearBanner: (code) => set((s) => ({ banners: s.banners.filter((b) => b.code !== code) })),
  clearBanners: () => set({ banners: [] }),
}))

/** Convenience for imperative call-sites (mutation handlers). */
export const toast = (text: string, kind: Toast['kind'] = 'info') =>
  useUI.getState().pushToast(text, kind)

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
    useUI.getState().pushToast(text, 'error')
  }
}
