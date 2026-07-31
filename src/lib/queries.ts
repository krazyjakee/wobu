import { useCallback, useEffect } from 'react'
import {
  keepPreviousData,
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
  type UseQueryResult,
} from '@tanstack/react-query'
import { listen } from '@tauri-apps/api/event'
import * as api from './api'
import type { CorruptFile, KindDef, NodeKind, NodeSummary, ProjectSummary, WobuNode } from './api'
import {
  applyCommand,
  birthEntry,
  deletionEntry,
  editEntry,
  moveEntry,
  useUndoStack,
} from './undo'
import { report, toast, useUI } from '../store/ui'

/* ── keys ─────────────────────────────────────────────────────────────────── */

export const qk = {
  kinds: ['kind_registry'] as const,
  projectCurrent: ['project_current'] as const,
  projectRecent: ['project_recent'] as const,
  nodes: ['node_list'] as const,
  corrupt: ['corrupt_files'] as const,
  node: (id: string) => ['node_get', id] as const,
  search: (q: string) => ['node_search', q] as const,
}

/** Everything that the file watcher can invalidate. */
export function invalidateWorld(qc: QueryClient) {
  void qc.invalidateQueries({ queryKey: qk.nodes })
  void qc.invalidateQueries({ queryKey: ['node_get'] })
  // A file breaking and a file being edited arrive as the same event, so the
  // corrupt list has to move with the node list or it goes stale the moment
  // someone repairs a file.
  void qc.invalidateQueries({ queryKey: qk.corrupt })
  // Editing a node changes what it matches. Without this the palette keeps
  // offering a hit for a phrase the user just deleted.
  void qc.invalidateQueries({ queryKey: ['node_search'] })
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
      qc.removeQueries({ queryKey: qk.corrupt })
      // Cached hits name nodes in a world that is no longer open.
      qc.removeQueries({ queryKey: ['node_search'] })
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
      useUndoStack.getState().push(birthEntry(node, 'create'))
      invalidateWorld(qc)
    },
  })
}

/**
 * The choke point every content edit goes through — rename, notes,
 * description, links, tags, cover — and therefore the one place any of them
 * needs to be recorded for undo.
 *
 * The previous state comes from the query cache in `onMutate`, i.e. before the
 * write lands. A node nothing has ever read is not in the cache, and rather
 * than invent a "before" that was never verified against disk, the edit simply
 * goes unrecorded: an undo that restores a guess is worse than no undo.
 */
export function useUpsertNode() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (node: WobuNode) => api.nodeUpsert(node),
    onMutate: (node) => ({ before: qc.getQueryData<WobuNode>(qk.node(node.id)) ?? null }),
    onSuccess: (node, _v, ctx) => {
      qc.setQueryData(qk.node(node.id), node)
      void qc.invalidateQueries({ queryKey: qk.nodes })
      const entry = ctx?.before && editEntry(ctx.before, node)
      if (entry) useUndoStack.getState().push(entry)
    },
  })
}

export function useDeleteNode() {
  const qc = useQueryClient()
  return useMutation({
    // Everything undo will need is gathered *before* the delete, because
    // afterwards none of it is knowable: the file is gone, and the children the
    // backend promotes to the deleted node's parent no longer say where they
    // came from. The id has to be the original one or every link to it breaks,
    // which is why the whole node is read rather than reconstructed later.
    //
    // A read that fails is not a delete that fails — the delete goes ahead, and
    // the only cost is that this one action cannot be undone.
    mutationFn: async (id: string) => {
      const before = await api.nodeGet(id).catch(() => null)
      const childIds = (qc.getQueryData<NodeSummary[]>(qk.nodes) ?? [])
        .filter((n) => n.parentId === id)
        .map((n) => n.id)
      await api.nodeDelete(id)
      return { before, childIds }
    },
    onSuccess: ({ before, childIds }, id) => {
      qc.removeQueries({ queryKey: qk.node(id) })
      if (before) useUndoStack.getState().push(deletionEntry(before, childIds))
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
      useUndoStack.getState().push(birthEntry(node, 'duplicate'))
      invalidateWorld(qc)
    },
  })
}

export function useMoveNode() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { id: string; newParentId: string | null }) =>
      api.nodeMove(v.id, v.newParentId),
    // Where it came from has to be read before the move, and the summary list
    // is the only place that holds it — `node_move` returns nothing.
    onMutate: (v) => ({
      from: (qc.getQueryData<NodeSummary[]>(qk.nodes) ?? []).find((n) => n.id === v.id) ?? null,
    }),
    onSuccess: (_r, v, ctx) => {
      const entry = ctx?.from && moveEntry(ctx.from, v.newParentId)
      if (entry) useUndoStack.getState().push(entry)
      invalidateWorld(qc)
    },
  })
}

/* ── undo ─────────────────────────────────────────────────────────────────── */

/**
 * Drive the undo stack from the UI.
 *
 * The commands run through `applyCommand`, which calls the backend directly
 * rather than going back through the mutation hooks above — those record what
 * they do, and an undo that recorded itself would put its own inverse on the
 * stack and make ⌘Z a toggle.
 *
 * Invalidation happens in `finally` on purpose. A sequence that failed halfway
 * has still changed the world, and the conflict path pulls the winner's version
 * into the index before it reports, so the cache is stale either way.
 */
export function useUndoRunner() {
  const qc = useQueryClient()

  const undo = useCallback(async () => {
    try {
      const entry = await useUndoStack.getState().undo(applyCommand)
      if (!entry) return
      toast(entry.caveat ? `Undone: ${entry.label}. ${entry.caveat}` : `Undone: ${entry.label}`)
    } catch (e) {
      report(e, 'Undo failed')
    } finally {
      invalidateWorld(qc)
    }
  }, [qc])

  const redo = useCallback(async () => {
    try {
      const entry = await useUndoStack.getState().redo(applyCommand)
      if (entry) toast(`Redone: ${entry.label}`)
    } catch (e) {
      report(e, 'Redo failed')
    } finally {
      invalidateWorld(qc)
    }
  }, [qc])

  return { undo, redo }
}

/**
 * The same two actions, plus what they would do.
 *
 * Separate from `useUndoRunner` because reading the stack subscribes to it, and
 * the keyboard hook lives in the Workspace — re-rendering the entire workspace
 * every time an entry is pushed, to run a callback that does not depend on it,
 * is a cost with nothing to show for it. Only a surface that *names* the next
 * entry needs this one.
 */
export function useUndo() {
  const { undo, redo } = useUndoRunner()
  const past = useUndoStack((s) => s.past)
  const future = useUndoStack((s) => s.future)

  return {
    undo,
    redo,
    /** The entry ⌘Z would reverse, for naming it on the surface that offers it. */
    nextUndo: past[past.length - 1] ?? null,
    nextRedo: future[future.length - 1] ?? null,
  }
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
