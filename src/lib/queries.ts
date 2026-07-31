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
