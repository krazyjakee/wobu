import type { LibraryQuery } from '../../lib/api/narrativeLibrary'
export type LibraryView = Omit<
  LibraryQuery,
  'offset' | 'limit' | 'revision' | 'ids' | 'unreadableOffset'
>
export const DEFAULT_LIBRARY_VIEW: LibraryView = {
  query: '',
  participant: '',
  quest: '',
  act: '',
  arc: '',
  tag: '',
  policy: '',
  review: '',
  freshness: '',
  missing: false,
  includeDrafts: false,
  sort: 'name',
}
