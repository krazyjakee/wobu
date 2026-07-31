import { useEffect } from 'react'
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
  type UseQueryResult,
} from '@tanstack/react-query'
import { listen } from '@tauri-apps/api/event'
import * as api from './api'
import type { KindDef, NodeKind, NodeSummary, ProjectSummary, WobuNode } from './api'
import { toast, useUI } from '../store/ui'

/* ── keys ─────────────────────────────────────────────────────────────────── */

export const qk = {
  kinds: ['kind_registry'] as const,
  projectCurrent: ['project_current'] as const,
  projectRecent: ['project_recent'] as const,
  nodes: ['node_list'] as const,
  node: (id: string) => ['node_get', id] as const,
}

/** Everything that the file watcher can invalidate. */
export function invalidateWorld(qc: QueryClient) {
  void qc.invalidateQueries({ queryKey: qk.nodes })
  void qc.invalidateQueries({ queryKey: ['node_get'] })
}

/* ── reads ────────────────────────────────────────────────────────────────── */

export function useKinds(): UseQueryResult<KindDef[]> {
  return useQuery({
    queryKey: qk.kinds,
    queryFn: api.kindRegistry,
    staleTime: Infinity,
    retry: false,
  })
}

export function useCurrentProject(): UseQueryResult<ProjectSummary | null> {
  return useQuery({
    queryKey: qk.projectCurrent,
    queryFn: api.projectCurrent,
    retry: false,
  })
}

export function useRecentProjects(): UseQueryResult<ProjectSummary[]> {
  return useQuery({
    queryKey: qk.projectRecent,
    queryFn: api.projectRecent,
    retry: false,
  })
}

export function useNodes(enabled: boolean): UseQueryResult<NodeSummary[]> {
  return useQuery({
    queryKey: qk.nodes,
    queryFn: api.nodeList,
    enabled,
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

/* ── project mutations ────────────────────────────────────────────────────── */

export function useOpenProject() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (path: string) => api.projectOpen(path),
    onSuccess: (p) => {
      qc.setQueryData(qk.projectCurrent, p)
      void qc.invalidateQueries({ queryKey: qk.projectRecent })
      invalidateWorld(qc)
    },
  })
}

export function useCreateProject() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { parentDir: string; name: string }) => api.projectCreate(v.parentDir, v.name),
    onSuccess: (p) => {
      qc.setQueryData(qk.projectCurrent, p)
      void qc.invalidateQueries({ queryKey: qk.projectRecent })
      invalidateWorld(qc)
    },
  })
}

export function useCloseProject() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: () => api.projectClose(),
    onSuccess: () => {
      qc.setQueryData(qk.projectCurrent, null)
      void qc.invalidateQueries({ queryKey: qk.projectRecent })
      qc.removeQueries({ queryKey: qk.nodes })
      qc.removeQueries({ queryKey: ['node_get'] })
    },
  })
}

/* ── node mutations ───────────────────────────────────────────────────────── */

export function useCreateNode() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { kind: NodeKind; name: string; parentId: string | null }) =>
      api.nodeCreate(v.kind, v.name, v.parentId),
    onSuccess: (node) => {
      qc.setQueryData(qk.node(node.id), node)
      invalidateWorld(qc)
    },
  })
}

export function useUpsertNode() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (node: WobuNode) => api.nodeUpsert(node),
    onSuccess: (node) => {
      qc.setQueryData(qk.node(node.id), node)
      void qc.invalidateQueries({ queryKey: qk.nodes })
    },
  })
}

export function useDeleteNode() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => api.nodeDelete(id),
    onSuccess: (_r, id) => {
      qc.removeQueries({ queryKey: qk.node(id) })
      invalidateWorld(qc)
    },
  })
}

/**
 * There is no `node_duplicate` command, so a copy is composed from the ones
 * that exist: read the source, create an empty node, then upsert the source's
 * content onto the new id. Nothing is fabricated — links and attributes are
 * carried across verbatim, and the description is deliberately not, because a
 * copy has not been enhanced.
 */
export function useDuplicateNode() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string): Promise<WobuNode> => {
      const src = await api.nodeGet(id)
      const created = await api.nodeCreate(src.kind, `${src.name} copy`, src.parentId)
      return api.nodeUpsert({
        ...created,
        summary: src.summary,
        notesRaw: src.notesRaw,
        attributes: src.attributes,
        tags: [...src.tags],
        links: src.links.map((l) => ({ ...l })),
        description: null,
        descriptionState: 'none',
      })
    },
    onSuccess: (node) => {
      qc.setQueryData(qk.node(node.id), node)
      invalidateWorld(qc)
    },
  })
}

export function useMoveNode() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { id: string; newParentId: string | null }) =>
      api.nodeMove(v.id, v.newParentId),
    onSuccess: () => invalidateWorld(qc),
  })
}

/* ── file-watcher bridge ──────────────────────────────────────────────────── */

/**
 * The backend emits `world:changed` whenever the project folder is reconciled
 * (its own writes, an Obsidian edit, a git pull, a collaborator on a share).
 * There is no meaningful payload — it is purely a cache-invalidation signal.
 */
export function useWorldChangedListener() {
  const qc = useQueryClient()
  useEffect(() => {
    if (!api.isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined
    void listen('world:changed', () => invalidateWorld(qc))
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* no watcher available — reads still work, they just aren't live */
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [qc])
}

/* ── share connectivity ───────────────────────────────────────────────────── */

const OFFLINE_TEXT =
  'The project folder is not reachable — the share may be unmounted. Everything here is ' +
  'still readable from the local index, but nothing can be saved until it is back. Retrying…'

function raiseOffline() {
  useUI.getState().raiseBanner({ code: 'share.unmounted', text: OFFLINE_TEXT, retryable: true })
}

/**
 * The share going away and coming back.
 *
 * The important half is what this deliberately does *not* do: going offline
 * never invalidates a query. The SQLite index lives in local app data, so
 * every node the user was looking at is still readable — refetching would
 * replace a working workspace with spinners and empty states, which is exactly
 * the failure this is meant to prevent. The cache is already right; all that
 * changes is that a banner appears and writes start being refused.
 *
 * Coming back *does* invalidate, because by then the backend has reconciled
 * against whatever happened to the folder while we were away.
 */
export function useShareListener() {
  const qc = useQueryClient()

  useEffect(() => {
    if (!api.isTauri()) return
    let disposed = false
    const unlisteners: Array<() => void> = []

    const attach = (event: string, handler: () => void) => {
      void listen(event, handler)
        .then((fn) => {
          if (disposed) fn()
          else unlisteners.push(fn)
        })
        .catch(() => {
          /* nothing to listen to is not worth surfacing */
        })
    }

    attach('share:offline', raiseOffline)

    attach('share:online', () => {
      const ui = useUI.getState()
      ui.clearBanner('share.unmounted')
      ui.clearBanner('share.quit_blocked')
      toast('The project folder is back.')
      invalidateWorld(qc)
    })

    attach('share:quit-blocked', () => {
      useUI.getState().raiseBanner({
        code: 'share.quit_blocked',
        text:
          'Wobu did not quit, because the share is still away and any edit being held would go ' +
          'with it. Wait for the folder to come back, or quit and lose them.',
        retryable: false,
        sticky: true,
        action: { label: 'Quit anyway', run: () => void api.forceQuit() },
      })
    })

    // A reload while disconnected misses the event that would have raised the
    // banner, so the state is asked for once on mount as well.
    void api
      .shareOffline()
      .then((offline) => {
        if (!disposed && offline) raiseOffline()
      })
      .catch(() => {
        /* no project open yet */
      })

    return () => {
      disposed = true
      unlisteners.forEach((fn) => fn())
    }
  }, [qc])
}
