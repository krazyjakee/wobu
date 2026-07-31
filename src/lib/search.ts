import type { NodeSummary } from './api'

/**
 * The palette searches twice, and these are the two halves.
 *
 * `nameMatches` runs locally on every keystroke over summaries already in
 * memory, so typing a node's name never waits on a round trip. `textMatches`
 * takes the ids FTS returned — which reach into notes and descriptions that are
 * not in memory and could not be searched here at all — and keeps only what the
 * first half did not already show.
 *
 * Pure and exported rather than inline in the component, because the dedup and
 * the ordering are the parts that would go wrong silently: a duplicate row is
 * an obvious bug, but a *dropped* row looks exactly like "no results".
 */

export const NAME_LIMIT = 40
export const TEXT_LIMIT = 20

/** Case-insensitive substring match on name or summary, best match first. */
export function nameMatches(nodes: NodeSummary[], query: string): NodeSummary[] {
  const needle = query.trim().toLowerCase()
  const list = needle
    ? nodes.filter(
        (n) => n.name.toLowerCase().includes(needle) || n.summary.toLowerCase().includes(needle),
      )
    : nodes

  return [...list]
    .sort((a, b) => {
      if (needle) {
        // Earlier in the name wins, so typing "kae" puts "Kael" above
        // "Broken Kaelstone". A name that does not match at all — it matched on
        // summary — sorts after every one that does.
        const ai = a.name.toLowerCase().indexOf(needle)
        const bi = b.name.toLowerCase().indexOf(needle)
        if (ai !== bi) return (ai < 0 ? 99 : ai) - (bi < 0 ? 99 : bi)
      }
      return a.name.localeCompare(b.name)
    })
    .slice(0, NAME_LIMIT)
}

/**
 * FTS hits not already shown by `nameMatches`, in the rank order SQLite gave.
 *
 * The order is never re-sorted: rank is the one piece of information the
 * backend has and this side does not.
 */
export function textMatches(
  byId: Map<string, NodeSummary>,
  ftsIds: string[],
  alreadyShown: Iterable<NodeSummary>,
  limit = TEXT_LIMIT,
): NodeSummary[] {
  const shown = new Set<string>()
  for (const n of alreadyShown) shown.add(n.id)

  const out: NodeSummary[] = []
  for (const id of ftsIds) {
    if (shown.has(id)) continue
    const node = byId.get(id)
    // An id the index knows and `node_list` does not is a node created since
    // the last refetch. There is nothing to render, so it is skipped rather
    // than counted against the limit.
    if (!node) continue
    out.push(node)
    if (out.length === limit) break
  }
  return out
}
