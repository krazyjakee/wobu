import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient, type UseQueryResult } from '@tanstack/react-query'
import { listen } from '@tauri-apps/api/event'
import * as api from '../api'
import type {
  Conflict,
  ConflictKeep,
  CorruptFile,
  KindDef,
  LinkEdge,
  NodeKind,
  NodeSummary,
  ProjectSyncStatus,
  ProjectSummary,
} from '../api'
import { report, toast } from '../../store/ui'
import { invalidateWorld, qk } from './keys'
/* ── keys ─────────────────────────────────────────────────────────────────── */

export function useKinds(): UseQueryResult<KindDef[]> {
  return useQuery({
    queryKey: qk.kinds,
    queryFn: api.kindRegistry,
    staleTime: Infinity,
    retry: false,
  })
}

export function usePresets(kind: NodeKind | null): UseQueryResult<api.Preset[]> {
  return useQuery({
    queryKey: qk.presets(kind ?? 'character'),
    queryFn: () => api.presetList(kind as NodeKind),
    enabled: !!kind,
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

/**
 * Runtime sync state for one project.
 *
 * Events make the status live; `sync_status` makes it correct after a webview
 * reload or an event fired just before this hook mounted. Both event names use
 * the full project snapshot, so state and peer changes cannot drift apart.
 */
export function useProjectSync(project: string): ProjectSyncStatus | null {
  const qc = useQueryClient()
  const status = useQuery({
    queryKey: qk.syncStatus,
    queryFn: api.syncStatus,
    staleTime: Infinity,
    // Binding is asynchronous. A first `running: false` is expected, not a
    // permanent answer; stop polling the moment the manager is available.
    refetchInterval: (query) => (query.state.data?.running === false ? 1_000 : false),
    retry: false,
  })

  useEffect(() => {
    if (!api.isTauri()) return
    let disposed = false
    const unlisteners: Array<() => void> = []

    const replace = (snapshot: ProjectSyncStatus) => {
      qc.setQueryData<api.SyncStatus>(qk.syncStatus, (current) => {
        if (!current) return current
        const found = current.projects.some((entry) => entry.project === snapshot.project)
        return {
          ...current,
          projects: found
            ? current.projects.map((entry) =>
                entry.project === snapshot.project ? snapshot : entry,
              )
            : [...current.projects, snapshot],
        }
      })
    }

    const attach = (name: 'sync:state' | 'sync:peer') => {
      void listen<ProjectSyncStatus>(name, (event) => replace(event.payload))
        .then((unlisten) => {
          if (disposed) unlisten()
          else unlisteners.push(unlisten)
        })
        .catch(() => {
          /* the catch-up query remains truthful when events are unavailable */
        })
    }

    attach('sync:state')
    attach('sync:peer')
    return () => {
      disposed = true
      for (const unlisten of unlisteners) unlisten()
    }
  }, [qc])

  return status.data?.projects.find((entry) => entry.project === project) ?? null
}

export function useNodes(enabled: boolean): UseQueryResult<NodeSummary[]> {
  return useQuery({
    queryKey: qk.nodes,
    queryFn: api.nodeList,
    enabled,
    retry: false,
  })
}

/** All explicit edges for project-wide, read-only relationship views. */
export function useNodeLinks(enabled: boolean): UseQueryResult<LinkEdge[]> {
  return useQuery({
    queryKey: qk.links,
    queryFn: api.nodeLinks,
    enabled,
    retry: false,
  })
}

/**
 * Files the last reconcile could not parse.
 *
 * Invalidated by `world:changed` alongside the node list, because a file
 * breaking and a file being edited arrive through exactly the same event.
 */
export function useCorruptFiles(enabled: boolean): UseQueryResult<CorruptFile[]> {
  return useQuery({
    queryKey: qk.corrupt,
    queryFn: api.corruptFiles,
    enabled,
    retry: false,
  })
}

/**
 * Versions of nodes that lost a save race and are waiting for a decision.
 *
 * Read from the folder on every fetch. A sibling can be parked by a
 * collaborator's Wobu on the far side of a share, so there is no event here
 * that could keep a cached list honest — `world:changed` invalidates it, and so
 * does a save that comes back `write.conflict`.
 */
export function useConflicts(enabled: boolean): UseQueryResult<Conflict[]> {
  return useQuery({
    queryKey: qk.conflicts,
    queryFn: api.conflicts,
    enabled,
    retry: false,
  })
}

/**
 * Apply a decision about one conflict, deleting the version the user rejected.
 *
 * `outcome` is not always `done`, and the two other answers are the point
 * rather than an edge case: `stale` means the node file moved while the card
 * was open and the question has changed, `conflict` means the write itself lost
 * a race. Both left everything on disk alone, so both are told to the user and
 * the list refetched rather than treated as a failure.
 */
export function useResolveConflict() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { relPath: string; keep: ConflictKeep; expectedHash: string }) =>
      api.conflictResolve(v.relPath, v.keep, v.expectedHash),
    onSuccess: (result) => {
      if (result.outcome === 'stale') {
        toast(
          'That entity changed while its two versions were open. Nothing was written — here it is again.',
        )
      } else if (result.outcome === 'conflict') {
        toast(
          `Somebody saved first. The version you chose was kept as ${result.conflictPath}.`,
          'error',
        )
      }
      invalidateWorld(qc)
    },
    onError: (e) => report(e, 'Could not settle the two versions'),
  })
}

/* ── presence ─────────────────────────────────────────────────────────────── */
