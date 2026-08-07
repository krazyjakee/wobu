import { useMutation, useQueryClient } from '@tanstack/react-query'
import * as api from '../api'
import type { LinkRole, NodeKind, NodeSummary, WobuNode } from '../api'
import { birthEntry, deletionEntry, editEntry, moveEntry, useUndoStack } from '../undo'
import { report } from '../../store/ui'
import { invalidateWorld, qk } from './keys'
/* ── keys ─────────────────────────────────────────────────────────────────── */

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
    // A save that lost the race just parked a sibling on disk, and the card for
    // it has to appear now rather than whenever the watcher next fires — on a
    // network share that is a five-second poll, and five seconds of the user
    // believing their paragraph is gone is the whole failure this feature is
    // for. The caller's own `onError` still runs; this only refetches.
    onError: (e) => {
      if (api.errorCode(e) === 'write.conflict') invalidateWorld(qc)
    },
  })
}

export function useSetLockedSeed() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ nodeId, seed }: { nodeId: string; seed: number | null }) =>
      api.nodeSeedLockSet(nodeId, seed),
    onSuccess: (node) => {
      qc.setQueryData(qk.node(node.id), node)
      void qc.invalidateQueries({ queryKey: qk.nodes })
      void qc.invalidateQueries({ queryKey: ['image_reference_report', node.id] })
    },
    onError: (error) => {
      if (api.errorCode(error) === 'write.conflict') invalidateWorld(qc)
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
 * content onto the new id. Nothing is fabricated — links, attributes and
 * reference images are carried across verbatim, and the description is
 * deliberately not, because a copy has not been enhanced.
 *
 * The reference images come along because they cost nothing to share: assets
 * are content-addressed, so a copy pointing at the same blobs is pointing at
 * the same file rather than duplicating it, and a duplicated character that
 * arrived with an empty picture strip would look like the copy had failed.
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
        assetLinks: src.assetLinks.map((a) => ({ ...a })),
        coverAssetId: src.coverAssetId,
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

/* ── node links ───────────────────────────────────────────────────────────── */

function useNodeLinkMutation<V>(run: (value: V) => Promise<WobuNode>, whileDoing: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: run,
    onSuccess: (node) => {
      qc.setQueryData(qk.node(node.id), node)
      void qc.invalidateQueries({ queryKey: qk.nodes })
      void qc.invalidateQueries({ queryKey: ['node_backlinks'] })
      void qc.invalidateQueries({ queryKey: ['influence_resolve'] })
      void qc.invalidateQueries({ queryKey: ['prompt_compile'] })
    },
    onError: (error) => {
      if (api.errorCode(error) === 'write.conflict') invalidateWorld(qc)
      report(error, whileDoing)
    },
  })
}

export function useAddNodeLink() {
  return useNodeLinkMutation(
    (value: { nodeId: string; toId: string; role: LinkRole; weight?: number; enabled?: boolean }) =>
      api.nodeLinkAdd(value.nodeId, value.toId, value.role, {
        weight: value.weight,
        enabled: value.enabled,
      }),
    'Could not add that relation',
  )
}

export function useRemoveNodeLink() {
  return useNodeLinkMutation(
    (value: { nodeId: string; toId: string; role: LinkRole }) =>
      api.nodeLinkRemove(value.nodeId, value.toId, value.role),
    'Could not remove that relation',
  )
}

export function useUpdateNodeLink() {
  return useNodeLinkMutation(
    (value: { nodeId: string; toId: string; role: LinkRole; weight?: number; enabled?: boolean }) =>
      api.nodeLinkUpdate(value.nodeId, value.toId, value.role, {
        weight: value.weight,
        enabled: value.enabled,
      }),
    'Could not change that relation',
  )
}

/* ── assets ───────────────────────────────────────────────────────────────── */
