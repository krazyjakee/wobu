import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/**
 * The nodes this person keeps at the top of their navigator.
 *
 * Local, like `store/settings.ts` and for the same reason: a favourite is one
 * reader's shortcut, not a fact about the world. Writing it into the project
 * folder would sync one person's working set onto everybody else's screen and
 * produce a file that conflicts every time two people star something, while
 * saying nothing about the entities themselves. Nothing here reaches the
 * `wobu-store` layout — the on-disk project is unchanged by starring a node
 * (docs/02-data-model.md is therefore silent about it on purpose).
 *
 * Keyed by project path rather than pooled, so opening a second world does not
 * inherit the first one's shortcuts, and so a project that is deleted takes its
 * ids with it the next time this list is written.
 */

/** Enough to hold a working set; past it the list is a second navigator. */
export const FAVOURITE_LIMIT = 100

interface FavouritesState {
  /** project path → node ids, in the order they were starred. */
  byProject: Record<string, string[]>
  toggle: (project: string, nodeId: string) => void
}

export const useFavourites = create<FavouritesState>()(
  persist(
    (set) => ({
      byProject: {},
      toggle: (project, nodeId) =>
        set((state) => {
          const current = state.byProject[project] ?? []
          const next = current.includes(nodeId)
            ? current.filter((id) => id !== nodeId)
            : [...current, nodeId].slice(-FAVOURITE_LIMIT)
          return { byProject: { ...state.byProject, [project]: next } }
        }),
    }),
    {
      name: 'wobu.favourites',
      // A hand-edited or half-written value must not be able to crash the
      // navigator on load, so anything that is not an array of strings is
      // dropped rather than trusted.
      merge: (stored, current) => {
        const raw = (stored as { byProject?: unknown } | null)?.byProject
        const byProject: Record<string, string[]> = {}
        if (raw && typeof raw === 'object') {
          for (const [project, ids] of Object.entries(raw as Record<string, unknown>)) {
            if (!Array.isArray(ids)) continue
            byProject[project] = ids.filter((id): id is string => typeof id === 'string')
          }
        }
        return { ...current, byProject }
      },
    },
  ),
)
