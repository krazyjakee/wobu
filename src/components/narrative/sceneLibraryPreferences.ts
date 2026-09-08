import { DEFAULT_LIBRARY_VIEW, type LibraryView } from './sceneLibraryModel'

export interface LibraryPreferences {
  view: LibraryView
  saved: { name: string; view: LibraryView }[]
  pins: string[]
  recent: string[]
  page: number
  scroll: number
  selected: string | null
}
export function emptyPreferences(): LibraryPreferences {
  return {
    view: { ...DEFAULT_LIBRARY_VIEW },
    saved: [],
    pins: [],
    recent: [],
    page: 0,
    scroll: 0,
    selected: null,
  }
}
const key = (project: string) => `wobu:narrative-library:v1:${project}`
function isView(value: unknown): value is LibraryView {
  if (!value || typeof value !== 'object') return false
  const view = value as Record<string, unknown>
  return (
    typeof view.query === 'string' &&
    typeof view.participant === 'string' &&
    (view.quest === undefined || typeof view.quest === 'string') &&
    ['', 'generated', 'edited', 'locked'].includes(String(view.policy)) &&
    ['', 'draft', 'approved'].includes(String(view.review)) &&
    ['', 'current', 'out_of_date'].includes(String(view.freshness)) &&
    typeof view.missing === 'boolean' &&
    typeof view.includeDrafts === 'boolean' &&
    ['name', 'nameDescending'].includes(String(view.sort))
  )
}
export function readPreferences(project: string): LibraryPreferences {
  const empty = emptyPreferences()
  try {
    const value = JSON.parse(
      localStorage.getItem(key(project)) ?? 'null',
    ) as LibraryPreferences | null
    if (!value || !isView(value.view)) return empty
    return {
      view: { ...value.view, quest: value.view.quest ?? '' },
      saved: Array.isArray(value.saved)
        ? value.saved
            .filter((s) => typeof s?.name === 'string' && isView(s.view))
            .map((saved) => ({ ...saved, view: { ...saved.view, quest: saved.view.quest ?? '' } }))
        : [],
      pins: Array.isArray(value.pins) ? value.pins.filter((id) => typeof id === 'string') : [],
      recent: Array.isArray(value.recent)
        ? value.recent.filter((id) => typeof id === 'string').slice(0, 8)
        : [],
      page: Number.isSafeInteger(value.page) && value.page >= 0 ? value.page : 0,
      scroll: Number.isFinite(value.scroll) && value.scroll >= 0 ? value.scroll : 0,
      selected: typeof value.selected === 'string' ? value.selected : null,
    }
  } catch {
    return empty
  }
}
export function writePreferences(project: string, value: LibraryPreferences) {
  try {
    localStorage.setItem(key(project), JSON.stringify(value))
  } catch {
    /* Storage can be unavailable; keep the live view usable. */
  }
}
