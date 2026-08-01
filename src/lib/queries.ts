import { useCallback, useEffect, useMemo, useState } from 'react'
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
import type {
  Asset,
  AssetKind,
  AssetRole,
  Capability,
  CompiledPrompt,
  Conflict,
  ConflictKeep,
  CorruptFile,
  InfluenceStack,
  KeyStatus,
  KindDef,
  LinkEdge,
  LinkRole,
  NodeKind,
  NodeSummary,
  Peer,
  ProjectSyncStatus,
  ProjectSummary,
  ProviderSelections,
  PromptBudget,
  QueueSnapshot,
  ShotControls,
  SliderSetting,
  WobuNode,
} from './api'
import { applyCommand, birthEntry, deletionEntry, editEntry, moveEntry, useUndoStack } from './undo'
import { PRESENCE_POLL_MS, livePeers } from './presence'
import { report, toast, useUI } from '../store/ui'
import { closeProjectAfterEditorWrites } from './projectClose'

/* ── keys ─────────────────────────────────────────────────────────────────── */

export const qk = {
  kinds: ['kind_registry'] as const,
  presets: (kind: NodeKind) => ['preset_list', kind] as const,
  projectCurrent: ['project_current'] as const,
  projectRecent: ['project_recent'] as const,
  // Installation-wide catch-up for the two live sync event streams.
  syncStatus: ['sync_status'] as const,
  nodes: ['node_list'] as const,
  links: ['node_links'] as const,
  corrupt: ['corrupt_files'] as const,
  conflicts: ['conflicts'] as const,
  assets: ['asset_list'] as const,
  assetUsages: ['asset_usage_list'] as const,
  generations: (nodeId: string) => ['generation_list', nodeId] as const,
  loraStatus: (nodeId: string) => ['lora_status', nodeId] as const,
  meshes: (nodeId: string) => ['mesh_concepts', nodeId] as const,
  meshPath: (assetId: string) => ['mesh_asset_path', assetId] as const,
  generationHistory: ['generation_list_all'] as const,
  assetThumb: (assetId: string) => ['asset_thumb', assetId] as const,
  node: (id: string) => ['node_get', id] as const,
  backlinks: (id: string) => ['node_backlinks', id] as const,
  search: (q: string) => ['node_search', q] as const,
  // Per installation, not per project — which is why nothing in
  // `invalidateWorld` touches it. Opening someone else's world does not change
  // which keys this machine has.
  providerKeys: (providers: string[]) => ['provider_key_status', providers] as const,
  // And its opposite number: per project, shared with everyone who opens the
  // folder. The two are never merged into one key because they are never
  // invalidated by the same thing.
  projectProviders: ['project_providers'] as const,
  statusBarBackend: (project: string) => ['status_bar_backend', project] as const,
  // The engine is a pure function of the world and these arguments, so the
  // arguments belong in the key: two presets, or two positions of the same
  // slider, are two different answers and neither invalidates the other.
  influence: (id: string, opts: InfluenceOptions) => ['influence_resolve', id, opts] as const,
  prompt: (id: string, opts: PromptOptions) => ['prompt_compile', id, opts] as const,
  imageReferences: (id: string, opts: api.GenerateOptions) =>
    ['image_reference_report', id, opts] as const,
  spendStatus: (project: string) => ['spend_status', project] as const,
  // Not part of `invalidateWorld`: a description waiting to be accepted is not
  // in the world yet, and a collaborator's edit does not change what a provider
  // already sent us.
  enhancePending: ['enhance_pending'] as const,
  // Nor is this one: presence moves on its own heartbeat, and a collaborator
  // opening a node changes who is here without changing the world at all.
  peers: ['presence_peers'] as const,
}

/** Everything that the file watcher can invalidate. */
export function invalidateWorld(qc: QueryClient) {
  void qc.invalidateQueries({ queryKey: qk.nodes })
  void qc.invalidateQueries({ queryKey: qk.links })
  void qc.invalidateQueries({ queryKey: ['node_get'] })
  void qc.invalidateQueries({ queryKey: ['node_backlinks'] })
  // A file breaking and a file being edited arrive as the same event, so the
  // corrupt list has to move with the node list or it goes stale the moment
  // someone repairs a file.
  void qc.invalidateQueries({ queryKey: qk.corrupt })
  // A conflict sibling can be parked by another machine, so the only signal
  // this side ever gets that one appeared is the folder having changed.
  void qc.invalidateQueries({ queryKey: qk.conflicts })
  void qc.invalidateQueries({ queryKey: qk.assetUsages })
  void qc.invalidateQueries({ queryKey: ['mesh_concepts'] })
  // Editing a node changes what it matches. Without this the palette keeps
  // offering a hit for a phrase the user just deleted.
  void qc.invalidateQueries({ queryKey: ['node_search'] })
  // A collaborator's import produces no event on this machine either — the
  // backend only learns about it by listing the folder, which it does as part
  // of the same reconcile that raised this.
  void qc.invalidateQueries({ queryKey: qk.assets })
  void qc.invalidateQueries({ queryKey: ['generation_list'] })
  void qc.invalidateQueries({ queryKey: qk.generationHistory })
  // A stack is built from other people's nodes as much as from the subject's:
  // an edit two layers out changes the compiled prompt without touching
  // anything the panel is pointing at, so these move with the world rather than
  // with the node in the Inspector.
  void qc.invalidateQueries({ queryKey: ['influence_resolve'] })
  void qc.invalidateQueries({ queryKey: ['prompt_compile'] })
  void qc.invalidateQueries({ queryKey: ['image_reference_report'] })
  // Reference pins, provider selection, and the entity's attached weight all
  // participate in LoRA readiness.
  void qc.invalidateQueries({ queryKey: ['lora_status'] })
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
          'That node changed while the conflict was open. Nothing was written — here it is again.',
        )
      } else if (result.outcome === 'conflict') {
        toast(`Someone saved first. Your pick was kept as ${result.conflictPath}.`, 'error')
      }
      invalidateWorld(qc)
    },
    onError: (e) => report(e, 'Could not resolve the conflict'),
  })
}

/* ── presence ─────────────────────────────────────────────────────────────── */

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

/** What `influenceResolve` varies on, and therefore what goes in its key. */
export interface InfluenceOptions {
  preset?: string
  sliders?: SliderSetting[]
  shot?: ShotControls
}

export interface PromptOptions extends InfluenceOptions {
  budget?: PromptBudget
}

/**
 * The resolved stack for a subject — one card per layer, outermost first.
 *
 * `staleTime: Infinity` because the answer is a pure function of the world and
 * the arguments: nothing but a world change can move it, and that arrives as
 * `world:changed` and invalidates this by hand. Refetching on window focus would
 * be a round trip guaranteed to return what is already on screen.
 */
export function useInfluenceStack(
  subjectId: string | null,
  options: InfluenceOptions = {},
): UseQueryResult<InfluenceStack> {
  return useQuery({
    queryKey: qk.influence(subjectId ?? '', options),
    queryFn: () => api.influenceResolve(subjectId as string, options),
    enabled: !!subjectId,
    staleTime: Infinity,
    retry: false,
  })
}

/**
 * The compiled prompt, its spans, and the account of what was dropped.
 *
 * `keepPreviousData` is what makes this usable while a slider is moving: every
 * value is a new key, so without it the prompt box would empty and refill under
 * the cursor on every frame of a drag. The backend does no file I/O for this, so
 * running it per drag is cheap — it is the blanking that would be unacceptable,
 * not the call.
 *
 * `gcTime` is short for the same reason. A single drag leaves one cache entry
 * per position it passed through, and none of them will ever be asked for again.
 */
export function useCompiledPrompt(
  subjectId: string | null,
  options: PromptOptions = {},
): UseQueryResult<CompiledPrompt> {
  return useQuery({
    queryKey: qk.prompt(subjectId ?? '', options),
    queryFn: () => api.promptCompile(subjectId as string, options),
    enabled: !!subjectId,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
    gcTime: 30_000,
    retry: false,
  })
}

export function useImageReferenceReport(
  subjectId: string | null,
  options: Pick<
    api.GenerateOptions,
    'preset' | 'sliders' | 'shot' | 'aspect' | 'model' | 'seed' | 'grid'
  > = {},
): UseQueryResult<api.ImageReferenceReport> {
  return useQuery({
    queryKey: qk.imageReferences(subjectId ?? '', options),
    queryFn: () => api.imageReferenceReport(subjectId as string, options),
    enabled: !!subjectId,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
    gcTime: 30_000,
    retry: false,
  })
}

/**
 * Spend changes while paid jobs run, even when no Inspector control changes.
 * Poll only while a paid estimate is visible; local ComfyUI has no provider
 * charge and should not repeatedly reopen the shared project ledger.
 */
export function useSpendStatus(project: string, enabled: boolean): UseQueryResult<api.SpendStatus> {
  return useQuery({
    queryKey: qk.spendStatus(project),
    queryFn: api.spendStatus,
    enabled,
    refetchInterval: 5_000,
    retry: false,
  })
}

export function useSetSpendCeiling(project: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: api.spendCeilingSet,
    onSuccess: (status) => {
      qc.setQueryData(qk.spendStatus(project), status)
      void qc.invalidateQueries({ queryKey: ['image_reference_report'] })
    },
  })
}

export function useRecoverSpendLedger(project: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: api.spendRecoveryReset,
    onSuccess: (status) => {
      qc.setQueryData(qk.spendStatus(project), status)
      void qc.invalidateQueries({ queryKey: ['image_reference_report'] })
    },
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

/** One node's immutable Concepts history, newest first. */
export function useGenerations(nodeId: string): UseQueryResult<api.Generation[]> {
  const qc = useQueryClient()
  const query = useQuery({
    queryKey: qk.generations(nodeId),
    queryFn: () => api.generationList(nodeId),
    retry: false,
  })

  useEffect(() => {
    if (!api.isTauri()) return
    let disposed = false
    const unlisteners: Array<() => void> = []
    void listen<api.JobDone>(api.JOB_EVENTS.done, (event) => {
      if (event.payload.kind === 'generate') {
        void qc.invalidateQueries({ queryKey: qk.generations(nodeId) })
      }
    })
      .then((fn) => {
        if (disposed) fn()
        else unlisteners.push(fn)
      })
      .catch(() => {
        /* the folder watcher remains the slower catch-up path */
      })
    void listen<{ subjectId: string }>('generation:recorded', (event) => {
      if (event.payload.subjectId === nodeId) {
        void qc.invalidateQueries({ queryKey: qk.generations(nodeId) })
        void qc.invalidateQueries({ queryKey: qk.assets })
        void qc.invalidateQueries({ queryKey: qk.meshes(nodeId) })
      }
    })
      .then((fn) => {
        if (disposed) fn()
        else unlisteners.push(fn)
      })
      .catch(() => {
        /* job:done and the folder watcher remain catch-up paths */
      })
    return () => {
      disposed = true
      for (const unlisten of unlisteners) unlisten()
    }
  }, [nodeId, qc])

  return query
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

/** Project-wide immutable generation history, newest first. */
export function useGenerationHistory(): UseQueryResult<api.Generation[]> {
  const qc = useQueryClient()
  const query = useQuery({
    queryKey: qk.generationHistory,
    queryFn: api.generationListAll,
    retry: false,
  })

  useEffect(() => {
    if (!api.isTauri()) return
    let disposed = false
    const unlisteners: Array<() => void> = []
    const refresh = () => void qc.invalidateQueries({ queryKey: qk.generationHistory })
    void listen<api.JobDone>(api.JOB_EVENTS.done, (event) => {
      if (event.payload.kind === 'generate') refresh()
    })
      .then((fn) => {
        if (disposed) fn()
        else unlisteners.push(fn)
      })
      .catch(() => {})
    void listen('generation:recorded', refresh)
      .then((fn) => {
        if (disposed) fn()
        else unlisteners.push(fn)
      })
      .catch(() => {})
    return () => {
      disposed = true
      for (const unlisten of unlisteners) unlisten()
    }
  }, [qc])

  return query
}

export function useReplayGeneration() {
  return useMutation({
    mutationFn: api.generationReplay,
    onSuccess: () => toast('Replay queued from the immutable snapshot'),
    onError: (error) => report(error, 'Could not replay that generation'),
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
 * Bring a picture into the project folder.
 *
 * Only the asset list is invalidated, not the whole world: an import writes a
 * blob and touches nothing a node knows about, so refetching every node and
 * every search would be work for no change on screen.
 *
 * A re-import is silent about being a re-import. `deduped` is there for a
 * caller that wants to say "already in your library", but it is not a failure
 * and nothing here treats it as one — the user asked for that picture to be in
 * the project, and it is.
 */
export function useImportAsset() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { path: string; kind?: AssetKind }) => api.assetImport(v.path, v.kind),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: qk.assets })
    },
    onError: (e) => report(e, 'Could not import that image'),
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

/** Re-weight or mute a reference without detaching it. */
export function useUpdateAssetLink() {
  return useAssetLinkMutation(
    (v: { nodeId: string; assetId: string; role: AssetRole; weight?: number; enabled?: boolean }) =>
      api.assetLinkUpdate(v.nodeId, v.assetId, v.role, {
        weight: v.weight,
        enabled: v.enabled,
      }),
    'Could not change that reference',
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

/**
 * Whether this machine has a key for each of these providers.
 *
 * Presence, never value: there is no query here that returns key material,
 * because there is no command that returns it. What this answers is the
 * question the UI actually has — "Gemini is selected; can this machine run it?"
 *
 * `staleTime: Infinity` because the answer only changes when this app changes
 * it, and the two mutations below invalidate when they do. A key edited in
 * Seahorse or Keychain Access while Wobu is open is picked up on the next run;
 * the Rust side caches for the same reason, which is that a locked Secret
 * Service prompts the user on every read.
 */
export function useProviderKeys(providers: string[]): UseQueryResult<KeyStatus[]> {
  return useQuery({
    queryKey: qk.providerKeys(providers),
    queryFn: () => api.providerKeyStatus(providers),
    staleTime: Infinity,
    retry: false,
  })
}

/**
 * Save a key for a provider.
 *
 * Deliberately **not** a `useMutation`, and that is the whole point of the hook.
 * React Query keeps `variables` on a mutation until something resets it, so a
 * key passed through one stays reachable from the mutation cache — and the
 * mutation cache is a long-lived object hanging off the `QueryClient`, which
 * means a pasted key would outlive the form, the pane, and every re-render, in
 * the one process the rest of this design works to keep key material out of.
 * `keys.rs` guarantees nothing sends a key back to the webview; that guarantee
 * is worth very little if the webview keeps its own copy.
 *
 * So the key is an argument to a plain call and a local for the length of one
 * await. The caller passes it straight from the DOM node the user typed it into
 * and clears that node afterwards — see `Settings.tsx`, which never puts one in
 * React state either.
 *
 * `saving` is the provider id, not a boolean: the pane renders every provider at
 * once and a shared flag would put "Saving…" on all of them.
 */
export function useSetProviderKey() {
  const qc = useQueryClient()
  const [saving, setSaving] = useState<string | null>(null)

  const save = useCallback(
    async (provider: string, key: string) => {
      setSaving(provider)
      try {
        await api.providerKeySet(provider, key)
        void qc.invalidateQueries({ queryKey: ['provider_key_status'] })
        void qc.invalidateQueries({ queryKey: ['status_bar_backend'] })
      } finally {
        setSaving(null)
      }
    },
    [qc],
  )

  return { save, saving }
}

/**
 * Remove this machine's stored key for a provider.
 *
 * The result says whether anything was actually removed, and what the provider
 * resolves to now — which on a development build can still be "configured",
 * because the repo-root `.env` answers after the keychain does not.
 */
export function useDeleteProviderKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (provider: string) => api.providerKeyDelete(provider),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['provider_key_status'] })
      void qc.invalidateQueries({ queryKey: ['status_bar_backend'] })
    },
  })
}

/* ── the provider selection ───────────────────────────────────────────────── */

/**
 * Which provider this project has chosen for each capability.
 *
 * The counterpart to `useProviderKeys`, and kept apart from it on purpose: this
 * one is a property of the *project folder* and travels to everyone on the
 * share, that one is a property of *this machine* and travels nowhere. The pane
 * renders them as two bands for the same reason.
 *
 * Not part of `invalidateWorld`: a collaborator editing a node does not change
 * what `project.json` selects, and the only thing that changes it here is the
 * mutation below.
 */
export function useProviderSelections(): UseQueryResult<ProviderSelections> {
  return useQuery({
    queryKey: qk.projectProviders,
    queryFn: api.projectProviders,
    retry: false,
  })
}

export const BACKEND_HEALTH_POLL_MS = 5_000
export const BACKEND_HEALTH_MAX_BACKOFF_MS = 60_000

interface BackendPollState {
  project: string
  /** A non-empty external queue is the only reason to keep probing. */
  liveQueue: boolean
  failures: number
  dataUpdates: number
  errorUpdates: number
}

function backendPollInterval(
  query: {
    state: {
      data?: api.StatusBarBackend
      dataUpdateCount: number
      errorUpdateCount: number
    }
  },
  poll: BackendPollState,
): number | false {
  const { state } = query

  if (state.dataUpdateCount !== poll.dataUpdates) {
    poll.dataUpdates = state.dataUpdateCount
    const health = state.data?.health
    if (health?.state === 'connected') {
      poll.liveQueue = health.externalQueue !== null && health.externalQueue > 0
      poll.failures = 0
    } else if (health?.state === 'unavailable' && poll.liveQueue) {
      poll.failures += 1
    } else {
      poll.liveQueue = false
      poll.failures = 0
    }
  }

  // A bridge/network rejection has no fresh health payload. If a queue was
  // live immediately before it, retain that fact and back off until a check
  // succeeds; otherwise an idle error must not start a polling loop.
  if (state.errorUpdateCount !== poll.errorUpdates) {
    poll.failures += poll.liveQueue ? 1 : 0
    poll.errorUpdates = state.errorUpdateCount
  }

  if (!poll.liveQueue) return false
  return Math.min(
    BACKEND_HEALTH_POLL_MS * 2 ** Math.max(0, poll.failures - 1),
    BACKEND_HEALTH_MAX_BACKOFF_MS,
  )
}

interface BackendHealthSubscription {
  references: number
  dispose: () => void
}

const backendHealthSubscriptions = new WeakMap<QueryClient, BackendHealthSubscription>()

/**
 * One event bridge per QueryClient, even though both Workspace and Inspector
 * observe the same health query. Job outcomes are meaningful health signals;
 * they replace the old unconditional idle poll without doubling requests.
 */
function subscribeBackendHealth(qc: QueryClient): () => void {
  const existing = backendHealthSubscriptions.get(qc)
  if (existing) {
    existing.references += 1
    return () => {
      existing.references -= 1
      if (existing.references === 0) existing.dispose()
    }
  }

  let disposed = false
  let pendingWhileHidden = false
  const unlisteners: Array<() => void> = []
  const refresh = () => {
    if (document.visibilityState === 'hidden') {
      pendingWhileHidden = true
      return
    }
    pendingWhileHidden = false
    void qc.invalidateQueries({ queryKey: ['status_bar_backend'] })
  }
  const onVisible = () => {
    if (document.visibilityState === 'visible' && pendingWhileHidden) refresh()
  }
  window.addEventListener('online', refresh)
  document.addEventListener('visibilitychange', onVisible)

  const attach = <T>(name: string, handler: (payload: T) => void) => {
    if (!api.isTauri()) return
    void listen<T>(name, (event) => handler(event.payload))
      .then((unlisten) => {
        if (disposed) unlisten()
        else unlisteners.push(unlisten)
      })
      .catch(() => {
        /* explicit refresh and queue-aware polling remain available */
      })
  }

  // A running transition is the actual provider attempt, rather than merely a
  // button press or a queued local job. Track attempts so unrelated queue
  // transitions do not repeatedly probe health.
  const attempts = new Map<string, number>()
  attach<QueueSnapshot>(api.JOB_EVENTS.state, (snapshot) => {
    let attempted = false
    const present = new Set<string>()
    for (const job of snapshot.jobs) {
      if (job.kind !== 'generate') continue
      present.add(job.id)
      if (job.state === 'running' && attempts.get(job.id) !== job.attempt) {
        attempts.set(job.id, job.attempt)
        attempted = true
      }
    }
    for (const id of attempts.keys()) if (!present.has(id)) attempts.delete(id)
    if (attempted) refresh()
  })
  attach<api.JobFailed>(api.JOB_EVENTS.error, (failure) => {
    if (failure.kind === 'generate') refresh()
  })
  attach<api.JobDone>(api.JOB_EVENTS.done, (done) => {
    if (done.kind === 'generate') refresh()
  })

  const subscription: BackendHealthSubscription = {
    references: 1,
    dispose: () => {
      if (subscription.references !== 0) return
      disposed = true
      window.removeEventListener('online', refresh)
      document.removeEventListener('visibilitychange', onVisible)
      for (const unlisten of unlisteners) unlisten()
      backendHealthSubscriptions.delete(qc)
    },
  }
  backendHealthSubscriptions.set(qc, subscription)

  return () => {
    subscription.references -= 1
    if (subscription.references === 0) subscription.dispose()
  }
}

/**
 * Models resolved by the backend plus a real, non-generating health check.
 * The project id belongs in the cache key even though the command needs no
 * argument: the command reads the open project, while this cache can outlive a
 * Workspace during a project switch.
 */
export function useStatusBarBackend(project: string): UseQueryResult<api.StatusBarBackend> {
  const qc = useQueryClient()
  const poll = useMemo<BackendPollState>(
    () => ({ project, liveQueue: false, failures: 0, dataUpdates: 0, errorUpdates: 0 }),
    [project],
  )
  const query = useQuery({
    queryKey: qk.statusBarBackend(project),
    queryFn: api.statusBarBackend,
    // Gemini reports no external queue, and an idle ComfyUI reports zero: both
    // stop here after the initial/event-driven check. Only a queue whose depth
    // is visibly changing justifies polling the remote backend.
    refetchInterval: (current) => backendPollInterval(current, poll),
    refetchIntervalInBackground: false,
    retry: false,
  })

  useEffect(() => subscribeBackendHealth(qc), [qc])
  return query
}

/**
 * Choose a provider for one capability.
 *
 * The result is the whole selection map, written into the cache rather than
 * refetched: the backend has just re-read `project.json` to answer, so a second
 * round trip would only ask the same question again.
 *
 * Rejects with `write.read_only` on a read-only folder, which the pane already
 * knows and disables for — the rejection is the backstop for a folder that
 * turned read-only mid-session.
 */
export function useSelectProvider() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: {
      capability: Capability
      provider: string
      model?: string
      region?: string
    }) => api.projectProviderSelect(v.capability, v.provider, v.model, v.region),
    onSuccess: (selections) => {
      qc.setQueryData(qk.projectProviders, selections)
      void qc.invalidateQueries({ queryKey: ['status_bar_backend'] })
      void qc.invalidateQueries({ queryKey: ['image_reference_report'] })
    },
    onError: (e: unknown) => report(e, 'Could not change the provider'),
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

/* ── the job queue ────────────────────────────────────────────────────────── */

/**
 * The queue, live.
 *
 * Not a `useQuery`: there is nothing to invalidate and nothing to refetch. The
 * backend sends the whole queue on every transition, so this is a subscription
 * with one catch-up read for the case events cannot cover — a webview that
 * reloaded while three generations were in flight.
 *
 * Whole snapshots rather than accumulated deltas, deliberately. A queue
 * reassembled on this side from `progress`/`done`/`error` would be wrong the
 * first time an event was dropped or arrived out of order, and it would be
 * wrong in a way that shows: a job stuck on screen that finished minutes ago.
 */
export function useJobQueue(): QueueSnapshot {
  const [snapshot, setSnapshot] = useState<QueueSnapshot>(EMPTY_QUEUE)

  useEffect(() => {
    if (!api.isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined

    void listen<QueueSnapshot>(api.JOB_EVENTS.state, (event) => setSnapshot(event.payload))
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* nothing to listen to yet; the catch-up read below still applies */
      })

    void api
      .jobList()
      .then((current) => {
        if (!disposed) setSnapshot(current)
      })
      .catch(() => {
        /* no queue to ask — an empty one is the right answer */
      })

    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])

  return snapshot
}

/** Shared so that an idle queue is referentially stable and renders nothing. */
const EMPTY_QUEUE: QueueSnapshot = { jobs: [], queued: 0, running: 0, retrying: 0 }

/* ── enhance ──────────────────────────────────────────────────────────────── */

/**
 * Start an Enhance and get a job id back.
 *
 * Nothing here waits for the description. The call returns before the provider
 * is even asked, and the answer arrives over `enhance:delta` and `job:done` —
 * which is what lets the user carry on editing a different node while one is
 * running, and what makes Stop a real stop rather than an abandoned promise.
 *
 * Nothing is invalidated on success either, because nothing has changed yet: an
 * Enhance writes when it is *accepted*.
 */
export function useEnhance() {
  return useMutation({ mutationFn: (nodeId: string) => api.enhanceStart(nodeId) })
}

/**
 * Write a finished description to its node.
 *
 * `refusedEdit` comes back as a *success*, not a rejection, and the caller has
 * to handle it: the description on disk was written by hand and nobody has said
 * to replace it. Show what is about to be overwritten, and call again with
 * `force` if the user says yes. Treating it as a failure would turn a question
 * into a dialog to dismiss.
 */
export function useAcceptEnhanced() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { jobId: string; description?: api.WobuDescription; force?: boolean }) =>
      api.enhanceAccept(v.jobId, v.description, v.force),
    onSuccess: (accepted) => {
      // A refusal changed nothing on disk, so there is nothing to invalidate and
      // refetching would only flicker the pane the question is being asked in.
      if (accepted.outcome !== 'saved') return
      qc.setQueryData(qk.node(accepted.node.id), accepted.node)
      // The description state moved to `fresh`, which the navigator draws, and
      // a description is the whole input to the influence engine.
      void qc.invalidateQueries({ queryKey: qk.nodes })
      void qc.invalidateQueries({ queryKey: ['influence_resolve'] })
      void qc.invalidateQueries({ queryKey: ['prompt_compile'] })
      // It is no longer waiting — the backend dropped it the moment the write
      // landed, and a stale entry here would offer to accept it twice.
      void qc.invalidateQueries({ queryKey: qk.enhancePending })
    },
    onError: (e) => {
      if (api.errorCode(e) === 'write.conflict') invalidateWorld(qc)
    },
  })
}

export function useDiscardEnhanced() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (jobId: string) => api.enhanceDiscard(jobId),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: qk.enhancePending })
    },
  })
}

/**
 * Descriptions this process is still holding, waiting to be accepted.
 *
 * The catch-up read after a reload, and the reason it exists is money: the
 * `job:done` that carried the answer is gone with the old page, and without this
 * the only route back to a description that has already been paid for is paying
 * again. Match on `nodeId`, answer with `jobId`.
 *
 * `staleTime: Infinity` because nothing on this side changes it but the two
 * mutations above, and both invalidate. A description that arrives while the app
 * is running arrives on `job:done`; this is only ever for the ones that did not.
 */
export function useEnhancePending(enabled: boolean): UseQueryResult<api.EnhanceReady[]> {
  return useQuery({
    queryKey: qk.enhancePending,
    queryFn: () => api.enhancePending(),
    enabled,
    staleTime: Infinity,
  })
}

/**
 * The description one Enhance has streamed so far.
 *
 * A subscription rather than a query, for the same reason `useJobQueue` is one:
 * there is nothing to fetch and nothing to invalidate, and the backend already
 * sends whole snapshots. Rendering the payload as-is is the whole contract —
 * accumulating fragments on this side would be reassembling state the other
 * side is already sending correctly.
 *
 * `null` until the first frame, and again whenever `jobId` changes, so a pane
 * switched from one node to another never shows the previous node's text.
 * **Nothing here has been saved.** The node keeps whatever it had until
 * `useAcceptEnhanced` runs.
 */
export function useEnhanceStream(jobId: string | null): api.EnhanceDelta | null {
  const [delta, setDelta] = useState<api.EnhanceDelta | null>(null)

  useEffect(() => {
    setDelta(null)
    if (!jobId || !api.isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined

    void listen<api.EnhanceDelta>(api.ENHANCE_DELTA, (event) => {
      // Filtered here rather than by subscribing per job: one listener per
      // mounted pane is cheap, and two enhances can be in flight at once —
      // which is exactly when showing the wrong one would be hardest to spot.
      if (event.payload.jobId === jobId) setDelta(event.payload)
    })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* nothing to listen to; the pane simply shows no progress */
      })

    return () => {
      disposed = true
      unlisten?.()
    }
  }, [jobId])

  return delta
}
