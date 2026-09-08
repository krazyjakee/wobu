import { call } from './call'
import type { Freshness, GenerationPolicy, ReviewState, SceneSummary } from './narrative'

export interface LibraryLabel {
  id: string
  name: string
  missing?: boolean
}
export interface LibraryQuery {
  query: string
  participant: string
  quest: string
  act: string
  arc: string
  tag: string
  policy: GenerationPolicy | ''
  review: ReviewState | ''
  freshness: Freshness | ''
  missing: boolean
  /** Include accepted generated wording whose recorded review state is Draft. */
  includeDrafts: boolean
  sort: 'name' | 'nameDescending'
  offset: number
  limit: number
  revision: string | null
  /** Independent pinned/recent lookup; at most 16 IDs, with missing IDs reported. */
  ids?: string[]
  unreadableOffset?: number
}
export interface LibraryMatch {
  sceneId: string
  beatId?: string
  lineId?: string
  variantId?: string
  snippet: string
  draft: boolean
}
export interface LibrarySceneRow {
  summary: SceneSummary
  act: LibraryLabel | null
  arc: LibraryLabel | null
  tags: LibraryLabel[]
  quests: LibraryLabel[]
  participants: string[]
  slots: number
  filled: number
  beats: number
  counts: {
    generated: number
    edited: number
    locked: number
    needsReview: number
    outOfDate: number
  }
  matches: LibraryMatch[]
  matchCount: number
}
export interface LibraryPage {
  revision: string
  total: number
  sceneCount: number
  rows: LibrarySceneRow[]
  unreadable: { rel: string; reason: string }[]
  unreadableTotal: number
  unreadableNextOffset: number | null
  missingIds: string[]
  facets: {
    participants: string[]
    acts: LibraryLabel[]
    arcs: LibraryLabel[]
    tags: LibraryLabel[]
    quests: LibraryLabel[]
  }
}
export const narrativeLibraryQuery = (query: LibraryQuery): Promise<LibraryPage> =>
  call('narrative_library_query', { query })
