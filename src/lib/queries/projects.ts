import { useMutation, useQueryClient } from '@tanstack/react-query'
import * as api from '../api'
import type { ProjectSummary } from '../api'
import { closeProjectAfterEditorWrites } from '../projectClose'
import { invalidateWorld, qk } from './keys'
/* ── keys ─────────────────────────────────────────────────────────────────── */

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

export function useForgetRecentProject() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => api.projectRecentForget(id),
    onMutate: async (id) => {
      await qc.cancelQueries({ queryKey: qk.projectRecent })
      const previous = qc.getQueryData<ProjectSummary[]>(qk.projectRecent)
      qc.setQueryData<ProjectSummary[]>(qk.projectRecent, (current) =>
        current?.filter((project) => project.id !== id),
      )
      return { previous }
    },
    onError: (_error, _id, context) => {
      if (context?.previous) qc.setQueryData(qk.projectRecent, context.previous)
    },
    onSettled: () => void qc.invalidateQueries({ queryKey: qk.projectRecent }),
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

export function useApplyStyleTransfer() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (value: { sourcePath: string; rootId: string }) =>
      api.styleTransferApply(value.sourcePath, value.rootId),
    onSuccess: () => invalidateWorld(qc),
  })
}

export function useCloseProject() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: closeProjectAfterEditorWrites,
    onSuccess: () => {
      qc.setQueryData(qk.projectCurrent, null)
      void qc.invalidateQueries({ queryKey: qk.projectRecent })
      qc.removeQueries({ queryKey: qk.nodes })
      qc.removeQueries({ queryKey: ['node_get'] })
      qc.removeQueries({ queryKey: ['node_backlinks'] })
      qc.removeQueries({ queryKey: qk.corrupt })
      qc.removeQueries({ queryKey: qk.conflicts })
      qc.removeQueries({ queryKey: qk.assets })
      qc.removeQueries({ queryKey: qk.assetUsages })
      // Cached hits name nodes in a world that is no longer open.
      qc.removeQueries({ queryKey: ['node_search'] })
      // As are the people in it. Removed rather than left to age out, because a
      // minute of the launcher still knowing who was in the folder we just
      // closed is a minute of describing a project nobody has open.
      qc.removeQueries({ queryKey: qk.peers })
      // As do cached stacks — removed rather than invalidated, because a stale
      // one served for a moment would be the *previous* project's world on
      // screen, which is the one thing a local-first app must never do.
      qc.removeQueries({ queryKey: ['influence_resolve'] })
      qc.removeQueries({ queryKey: ['prompt_compile'] })
    },
  })
}

/* ── node mutations ───────────────────────────────────────────────────────── */
