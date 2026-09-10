import { useQuery } from '@tanstack/react-query'
import { narrativeLibraryQuery, type LibraryQuery } from '../api/narrativeLibrary'
export function useNarrativeLibraryQuery(projectKey: string, query: LibraryQuery, enabled = true) {
  return useQuery({
    queryKey: ['narrative_library', projectKey, query],
    queryFn: () => narrativeLibraryQuery(query),
    enabled,
    retry: false,
  })
}
