import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/**
 * Preferences that belong to this machine, not to the world.
 *
 * Deliberately *not* in the project folder. A project is a shared thing —
 * putting one person's editor preferences beside the Markdown would sync them
 * onto everyone else's machine, and produce a file that conflicts constantly
 * while meaning nothing to anyone but its author. Local storage is the right
 * scope for exactly the same reason the SQLite index is local.
 *
 * Separate from `useUI` because that store is ephemeral view state — which node
 * is selected, which panes are open — and nothing in it should survive a
 * restart. Merging them would mean either persisting the selection or being
 * unable to persist these.
 */

/** Below this the fixed-width rail and inspector stop fitting their contents. */
export const SCALE_MIN = 0.8
export const SCALE_MAX = 1.5
export const SCALE_STEP = 0.1

/**
 * Anything under ~150ms writes on nearly every keystroke, which on a network
 * share is a write per character. The cap is the point past which an
 * unexpected quit could plausibly lose a sentence.
 */
export const AUTOSAVE_MIN = 150
export const AUTOSAVE_MAX = 3000
export const AUTOSAVE_DEFAULT = 500

interface SettingsState {
  /** Whole-interface zoom. The type scale is fixed px, so this scales layout too. */
  uiScale: number
  setUiScale: (v: number) => void

  /** Debounce for `useAutosaveNode`, in milliseconds. */
  autosaveDelay: number
  setAutosaveDelay: (v: number) => void

  reset: () => void
}

/**
 * Clamp, treating anything that is not a finite number as absent.
 *
 * The non-finite check is not defensive padding: `Math.min(hi, Math.max(lo, x))`
 * propagates `NaN` rather than clamping it, so a stored `"big"` — or a value
 * from a build that kept something else under this key — would sail through and
 * end up as `zoom: NaN` on the document root. A nullish fallback does not catch
 * it either, since the value is present, just not a number.
 */
function clamp(v: unknown, lo: number, hi: number, fallback: number): number {
  const n = typeof v === 'number' ? v : Number(v)
  if (!Number.isFinite(n)) return fallback
  return Math.min(hi, Math.max(lo, n))
}

const DEFAULTS = { uiScale: 1, autosaveDelay: AUTOSAVE_DEFAULT }

export const useSettings = create<SettingsState>()(
  persist(
    (set) => ({
      ...DEFAULTS,
      // Clamped on the way in rather than at each read: a value edited by hand
      // in local storage, or left behind by an older build with a different
      // range, must not be able to make the app unusable.
      setUiScale: (v) => set({ uiScale: clamp(v, SCALE_MIN, SCALE_MAX, DEFAULTS.uiScale) }),
      setAutosaveDelay: (v) =>
        set({
          autosaveDelay: Math.round(
            clamp(v, AUTOSAVE_MIN, AUTOSAVE_MAX, DEFAULTS.autosaveDelay),
          ),
        }),
      reset: () => set(DEFAULTS),
    }),
    {
      name: 'wobu.settings',
      // Same clamp on rehydrate, for a stored value written before a range
      // changed. `persist` merges the stored object over the defaults, so
      // without this an out-of-range number survives a restart.
      merge: (stored, current) => {
        const s = (stored ?? {}) as Partial<SettingsState>
        return {
          ...current,
          uiScale: clamp(s.uiScale, SCALE_MIN, SCALE_MAX, DEFAULTS.uiScale),
          autosaveDelay: Math.round(
            clamp(s.autosaveDelay, AUTOSAVE_MIN, AUTOSAVE_MAX, DEFAULTS.autosaveDelay),
          ),
        }
      },
    },
  ),
)
