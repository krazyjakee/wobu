import { useEffect, useMemo } from 'react'
import { keepPreviousData, useQuery, type UseQueryResult } from '@tanstack/react-query'
import * as api from '../api'
import type { LinkEdge, Peer, WobuNode } from '../api'
import { PRESENCE_POLL_MS, livePeers } from '../presence'
import { qk } from './keys'
/* ── keys ─────────────────────────────────────────────────────────────────── */

/**
 * Who else has this project open, polled.
 *
 * `ready` is separate from the list because the two say different things: an
 * empty list before the first answer means "we have not looked yet", and an
 * empty list after it means "nobody is here". The greeting on open is allowed
 * to fire on the second and must never fire on the first.
 *
 * A failed poll is reported nowhere. Either no project is open yet, or the
 * share is away and the banner for that is already on screen — and a toast
 * every ten seconds about a courtesy feature is noise. The list keeps whatever
 * it last knew until `livePeers` ages it out.
 */
export function usePresence(enabled: boolean): { peers: Peer[]; ready: boolean } {
  const { data, isSuccess } = useQuery({
    queryKey: qk.peers,
    queryFn: api.presencePeers,
    enabled,
    refetchInterval: PRESENCE_POLL_MS,
    retry: false,
  })

  const peers = useMemo(() => livePeers(data), [data])
  return { peers, ready: isSuccess }
}

/**
 * Tell everyone else which node this session has open.
 *
 * Fire-and-forget, and a failure is swallowed: this is what puts a dot on
 * somebody else's navigator, and a courtesy that could not be paid — a
 * read-only share, no project open yet — is not worth interrupting anyone for.
 *
 * The whole list every time rather than an add/remove pair, matching
 * `presence_editing`. A `null` selection sends `[]`, and that is the half worth
 * keeping: without it the last node we happened to look at stays marked as ours
 * on everyone else's rows for the rest of the session.
 */
export function useReportEditing(nodeId: string | null): void {
  useEffect(() => {
    if (!api.isTauri()) return
    void api.presenceEditing(nodeId ? [nodeId] : []).catch(() => {
      /* advisory in both directions: nobody needs to hear that a courtesy failed */
    })
  }, [nodeId])
}

/**
 * FTS hits for `query`, in rank order.
 *
 * `placeholderData: keepPreviousData` is what stops the palette flickering
 * between empty and full while the next keystroke's query is in flight — the
 * previous hits stay on screen and are replaced, rather than the list emptying
 * and refilling under the cursor.
 *
 * Below two characters this does not run at all. A one-character prefix matches
 * most of the world, so it costs a query to tell the user nothing, and the
 * local name filter already covers that case instantly.
 */
export function useNodeSearch(query: string): UseQueryResult<string[]> {
  const trimmed = query.trim()
  return useQuery({
    queryKey: qk.search(trimmed),
    queryFn: () => api.nodeSearch(trimmed),
    enabled: trimmed.length >= 2,
    placeholderData: keepPreviousData,
    // The index is local; re-running on focus would only cost latency.
    staleTime: 30_000,
    retry: false,
  })
}

export function useNode(id: string | null): UseQueryResult<WobuNode> {
  return useQuery({
    queryKey: qk.node(id ?? ''),
    queryFn: () => api.nodeGet(id as string),
    enabled: !!id,
    retry: false,
  })
}

/** Explicit links pointing at one node; display names come from `useNodes`. */
export function useNodeBacklinks(id: string | null): UseQueryResult<LinkEdge[]> {
  return useQuery({
    queryKey: qk.backlinks(id ?? ''),
    queryFn: () => api.nodeBacklinks(id as string),
    enabled: !!id,
    retry: false,
  })
}

/* ── influence ────────────────────────────────────────────────────────────── */
