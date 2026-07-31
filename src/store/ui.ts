import { create } from 'zustand'
import type { NodeKind } from '../lib/api'

export type Mode = 'library' | 'board' | 'forge' | 'assets' | 'settings'
export type EditorTab = 'notes' | 'refs' | 'concepts' | 'three' | 'relations'

export const EDITOR_TABS: EditorTab[] = ['notes', 'refs', 'concepts', 'three', 'relations']

export interface Toast {
  id: number
  text: string
  kind: 'info' | 'error'
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
}))

/** Convenience for imperative call-sites (mutation handlers). */
export const toast = (text: string, kind: Toast['kind'] = 'info') =>
  useUI.getState().pushToast(text, kind)
