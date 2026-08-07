import { useEffect } from 'react'
import {
  type InfiniteData,
  useMutation,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
  type UseQueryResult,
} from '@tanstack/react-query'
import { listen } from '@tauri-apps/api/event'
import * as api from '../api'
import type { Asset, AssetRole, WobuNode } from '../api'
import { report, toast } from '../../store/ui'
import { invalidateWorld, qk } from './keys'
/* ── keys ─────────────────────────────────────────────────────────────────── */

/** Every blob in the open project, newest first. */
export function useAssets(enabled: boolean): UseQueryResult<Asset[]> {
  return useQuery({
    queryKey: qk.assets,
    queryFn: api.assetList,
    enabled,
    retry: false,
  })
}

/** Project-wide asset use, including linked-node tags and independent covers. */
export function useAssetUsages(enabled: boolean): UseQueryResult<api.AssetUsage[]> {
  return useQuery({
    queryKey: qk.assetUsages,
    queryFn: api.assetUsageList,
    enabled,
    retry: false,
  })
}

/** Permanently delete a true orphan after the caller has confirmed it. */
export function useDeleteAsset() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (assetId: string) => api.assetDelete(assetId),
    onSuccess: (_nothing, assetId) => {
      qc.setQueryData<Asset[]>(qk.assets, (current) =>
        current?.filter((asset) => asset.id !== assetId),
      )
      qc.removeQueries({ queryKey: qk.assetThumb(assetId) })
      void qc.invalidateQueries({ queryKey: qk.assetUsages })
    },
    onError: (error) => report(error, 'Could not delete that asset'),
  })
}

export const GENERATION_PAGE_SIZE = 60

async function generationPageWithThumbnails(page: api.GenerationPage): Promise<api.GenerationPage> {
  const missing = [
    ...new Set(
      page.items
        .filter((item) => item.firstAssetId && !item.thumbnailPath)
        .map((item) => item.firstAssetId as string),
    ),
  ]
  if (missing.length === 0) return page
  const paths = await api.assetThumbBatch(missing)
  return {
    ...page,
    items: page.items.map((item) => ({
      ...item,
      thumbnailPath: item.firstAssetId ? (paths[item.firstAssetId] ?? item.thumbnailPath) : null,
    })),
  }
}

type GenerationPages = InfiniteData<api.GenerationPage, number>

export function prependGeneration(
  current: GenerationPages | undefined,
  summary: api.GenerationSummary,
): GenerationPages | undefined {
  if (!current || current.pages.length === 0) return current
  if (current.pages.some((page) => page.items.some((item) => item.id === summary.id)))
    return current
  let carry: api.GenerationSummary | undefined = summary
  const pages = current.pages.map((page) => {
    const items = carry ? [carry, ...page.items] : [...page.items]
    carry = items.length > GENERATION_PAGE_SIZE ? items.pop() : undefined
    return { ...page, items, total: page.total + 1 }
  })
  return {
    ...current,
    pages,
  }
}

async function recordedSummary(event: api.GenerationRecorded): Promise<api.GenerationSummary> {
  const generation = event.generation
  const firstAssetId = generation.outputAssetIds[0] ?? null
  let thumbnailPath: string | null = null
  if (firstAssetId) {
    try {
      thumbnailPath = (await api.assetThumbBatch([firstAssetId]))[firstAssetId] ?? null
    } catch {
      // The receipt is durable even when this machine cannot draw its preview.
    }
  }
  const scene = api.sceneComposition(generation)
  const source = generation.params.seedSource
  return {
    id: generation.id,
    nodeId: generation.nodeId,
    createdAt: generation.createdAt,
    preset: generation.preset,
    viewType: generation.viewType,
    backend: generation.backend,
    model: generation.model,
    seed: generation.seed,
    promptExcerpt: `${[...generation.compiledPrompt].slice(0, 240).join('')}${
      [...generation.compiledPrompt].length > 240 ? '…' : ''
    }`,
    firstAssetId,
    outputCount: generation.outputAssetIds.length,
    seedSource: typeof source === 'string' ? source : null,
    usedLockedSeed:
      typeof generation.params.usedLockedSeed === 'boolean'
        ? generation.params.usedLockedSeed
        : null,
    sceneSubjectNames: scene?.subjectNames ?? [],
    thumbnailPath,
  }
}

/** One node's paginated immutable Concepts history, newest first. */
export function useGenerations(nodeId: string) {
  const qc = useQueryClient()
  const query = useInfiniteQuery({
    queryKey: qk.generations(nodeId),
    initialPageParam: 0,
    queryFn: async ({ pageParam }) =>
      generationPageWithThumbnails(
        await api.generationList(nodeId, pageParam, GENERATION_PAGE_SIZE),
      ),
    getNextPageParam: (last) => last.nextOffset ?? undefined,
    retry: false,
  })

  useEffect(() => {
    if (!api.isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined
    void listen<api.GenerationRecorded>('generation:recorded', (event) => {
      if (event.payload.subjectId !== nodeId) return
      void recordedSummary(event.payload).then((summary) => {
        if (disposed) return
        qc.setQueryData<GenerationPages>(qk.generations(nodeId), (current) =>
          prependGeneration(current, summary),
        )
        void qc.invalidateQueries({ queryKey: qk.assets })
        void qc.invalidateQueries({ queryKey: qk.meshes(nodeId) })
      })
    })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* the folder watcher remains the catch-up path */
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [nodeId, qc])

  return {
    ...query,
    data: query.data?.pages.flatMap((page) => page.items),
    total: query.data?.pages[0]?.total ?? 0,
  }
}

/** Local LoRA readiness for one Forge subject, refreshed after training settles. */
export function useLoraStatus(nodeId: string | null): UseQueryResult<api.LoraStatus> {
  const qc = useQueryClient()
  const query = useQuery({
    queryKey: qk.loraStatus(nodeId ?? ''),
    queryFn: () => api.loraStatus(nodeId as string),
    enabled: !!nodeId,
    retry: false,
  })

  useEffect(() => {
    if (!nodeId || !api.isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined
    void listen<api.JobDone>(api.JOB_EVENTS.done, (event) => {
      if (event.payload.kind === 'train_lora') {
        void qc.invalidateQueries({ queryKey: qk.loraStatus(nodeId) })
      }
    })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* world:changed remains the slower catch-up path */
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [nodeId, qc])

  return query
}

export function useTrainLora() {
  return useMutation({ mutationFn: api.loraTrainStart })
}

/** Mounted only by the active 3D tab; listing a share's mesh directory is lazy. */
export function useMeshConcepts(nodeId: string): UseQueryResult<api.MeshConcept[]> {
  return useQuery({
    queryKey: qk.meshes(nodeId),
    queryFn: () => api.meshConcepts(nodeId),
    retry: false,
  })
}

/** The only query that causes a complete GLB to cross a share. */
export function useMeshAssetPath(assetId: string | null): UseQueryResult<string | null> {
  return useQuery({
    queryKey: qk.meshPath(assetId ?? ''),
    queryFn: () => api.meshAssetPath(assetId as string),
    enabled: !!assetId,
    staleTime: Infinity,
    retry: false,
  })
}

export function useReplayGeneration() {
  return useMutation({
    mutationFn: api.generationReplay,
    onSuccess: () =>
      toast('Queued again from the record of the original — today’s world was not re-read'),
    onError: (error) => report(error, 'Could not replay that generation'),
  })
}

/**
 * Deleting a concept takes its unclaimed output images with it, so the asset
 * views have to be told as well as the receipt views. Concepts and the 3D
 * gallery are the receipt; the Asset Library is the blobs, and leaving either
 * half stale is the whole bug — a concept that is gone from the tab it was
 * deleted in and still in the Asset Library has not been deleted as far as the
 * user is concerned.
 */
export function useDeleteGeneration() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ generationId }: { generationId: string; nodeId: string }) =>
      api.generationDelete(generationId),
    onSuccess: (_result, { nodeId }) => {
      void qc.invalidateQueries({ queryKey: qk.generations(nodeId) })
      void qc.invalidateQueries({ queryKey: qk.meshes(nodeId) })
      void qc.invalidateQueries({ queryKey: qk.assets })
      void qc.invalidateQueries({ queryKey: qk.assetUsages })
      toast('Concept deleted')
    },
    onError: (error) => report(error, 'Could not delete that concept'),
  })
}

/** Lazily generated tile path; originals have a separate click-only command. */
export function useAssetThumb(assetId: string | null): UseQueryResult<string | null> {
  return useQuery({
    queryKey: qk.assetThumb(assetId ?? ''),
    queryFn: () => api.assetThumb(assetId as string),
    enabled: !!assetId,
    staleTime: Infinity,
    retry: false,
  })
}

/**
 * Attach, detach, re-weight, and choose a cover.
 *
 * One hook for all four because they share a shape and, more to the point, a
 * cache story: each returns the saved node, so the node query is set from the
 * response rather than refetched, and only the summary list is invalidated —
 * the picture strip is part of the node, and nothing else on screen moved.
 *
 * They are *not* recorded for undo. `useUpsertNode` is the choke point that
 * records edits, and these deliberately do not go through it: a reference is
 * attached by dropping a picture, which the user reverses by removing it. An
 * undo stack that also captured every weight-slider drag would bury the text
 * edits ⌘Z exists for.
 */
function useAssetLinkMutation<V>(run: (v: V) => Promise<WobuNode>, whileDoing: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: run,
    onSuccess: (node) => {
      qc.setQueryData(qk.node(node.id), node)
      void qc.invalidateQueries({ queryKey: qk.nodes })
      void qc.invalidateQueries({ queryKey: qk.assetUsages })
    },
    onError: (e) => {
      // A lost race has already parked a sibling on disk, and the card for it
      // has to appear now rather than at the watcher's next poll — the same
      // reasoning as `useUpsertNode`.
      if (api.errorCode(e) === 'write.conflict') invalidateWorld(qc)
      report(e, whileDoing)
    },
  })
}

/** Attach a reference image to a node in a role. */
export function useLinkAsset() {
  return useAssetLinkMutation(
    (v: { nodeId: string; assetId: string; role: AssetRole; weight?: number }) =>
      api.assetLink(v.nodeId, v.assetId, v.role, v.weight),
    'Could not attach that reference',
  )
}

/** Detach one. The picture stays in the library — it may be in use elsewhere. */
export function useUnlinkAsset() {
  return useAssetLinkMutation(
    (v: { nodeId: string; assetId: string; role: AssetRole }) =>
      api.assetUnlink(v.nodeId, v.assetId, v.role),
    'Could not remove that reference',
  )
}

/** Choose the image on a node's card, or clear it with `null`. */
export function useSetCoverAsset() {
  return useAssetLinkMutation(
    (v: { nodeId: string; assetId: string | null }) => api.assetSetCover(v.nodeId, v.assetId),
    'Could not set that cover image',
  )
}

/* ── provider keys ────────────────────────────────────────────────────────── */
