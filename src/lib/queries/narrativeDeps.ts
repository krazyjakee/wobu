import { useQuery } from '@tanstack/react-query'

import { narrativeAffected, type AffectedLine } from '../api/narrativeDeps'

/**
 * Which lines an edit has affected, project-wide (#168).
 *
 * One query for the whole project rather than one per line. The backend
 * compares a stored baseline against a fresh capture in a single coherent read,
 * so asking about one line costs the same as asking about all of them — and a
 * per-line query would fan out into one command per row of a scene the moment
 * anything rendered a list.
 *
 * Deliberately not refetched on an interval. Affectedness is a function of the
 * project folder, and the folder only moves through a save or a watcher event;
 * polling would put a full capture on a timer for an answer that cannot have
 * changed.
 */
export function useNarrativeAffected(enabled = true) {
  return useQuery<AffectedLine[]>({
    queryKey: ['narrative_affected'],
    queryFn: narrativeAffected,
    enabled,
    retry: false,
  })
}
