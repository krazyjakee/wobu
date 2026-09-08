import { type QueryClient } from '@tanstack/react-query'
import * as api from '../api'
import type { NodeKind } from '../api'
import { refreshNodeThumbs } from '../nodeThumbs'
import type { InfluenceOptions, PromptOptions } from './influence'
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
  imageGenerationCapabilities: (project: string, model?: string) =>
    ['image_generation_capabilities', project, model] as const,
  // Not part of `invalidateWorld`: a description waiting to be accepted is not
  // in the world yet, and a collaborator's edit does not change what a provider
  // already sent us.
  enhancePending: ['enhance_pending'] as const,
  // Nor is this one: presence moves on its own heartbeat, and a collaborator
  // opening a node changes who is here without changing the world at all.
  peers: ['presence_peers'] as const,

  // ── narrative ──────────────────────────────────────────────────────────
  narrativeScenes: ['narrative_scenes'] as const,
  narrativeScene: (id: string) => ['narrative_scene', id] as const,
  // The *saved* scene's problems. Diagnostics for an unsaved buffer are not in
  // the cache at all — see `useDiagnoseScene` — because keying them by the
  // document would hold one entry per keystroke and keying them by the scene id
  // would serve the answer for a different document.
  narrativeDiagnostics: (id: string) => ['narrative_diagnostics', id] as const,
  narrativeState: ['narrative_state'] as const,
  // One key per canvas. `graphKeyId` collapses the tagged object to a string so
  // two renders of the same graph are the same key rather than two structurally
  // equal objects that TanStack has to hash identically for ever.
  narrativeLayout: (graph: api.GraphKey) => ['narrative_layout', api.graphKeyId(graph)] as const,
}

/**
 * Everything narrative that a write or a folder change can invalidate.
 *
 * Layout is deliberately not here. An arrangement is not the world, and
 * refetching it because somebody saved a scene would fight a canvas the user is
 * dragging on — the merging write already means two people arranging the same
 * scene both keep their boxes, so there is nothing a refetch would rescue.
 */
export function invalidateNarrative(qc: QueryClient) {
  void qc.invalidateQueries({ queryKey: ['narrative_review'] })
  void qc.invalidateQueries({ queryKey: ['narrative_world'] })
  void qc.invalidateQueries({ queryKey: qk.narrativeScenes })
  void qc.invalidateQueries({ queryKey: ['narrative_scene'] })
  // Diagnostics are computed against the whole project — the scene catalog and
  // the declared variables — so a *different* scene being saved or deleted
  // changes this scene's answer. Invalidating the family rather than one key is
  // what keeps a dangling cross-scene link from lingering after the scene it
  // named came back.
  void qc.invalidateQueries({ queryKey: ['narrative_diagnostics'] })
  void qc.invalidateQueries({ queryKey: qk.narrativeState })
}

/** A different project must never render the previous project's scene cache.
 * Draft keys are explicitly project-scoped and remain until saved/discarded. */
export async function clearNarrativeReads(qc: QueryClient) {
  const families = new Set([
    'narrative_scenes',
    'narrative_scene',
    'narrative_diagnostics',
    'narrative_state',
    'narrative_layout',
    'narrative_source',
    'narrative_world',
    'narrative_review',
  ])
  const filter = {
    predicate: (query: { queryKey: readonly unknown[] }) => families.has(String(query.queryKey[0])),
  }
  await qc.cancelQueries(filter)
  qc.removeQueries(filter)
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
  // Scenes are not in the SQLite index yet (the remaining half of #153), so a
  // reconcile is the only signal this side gets that a collaborator edited one.
  // Until they are indexed that signal is coarse — the watcher raises this for
  // the folder, not for the scene — which is exactly why the whole narrative
  // family moves together rather than one key at a time.
  invalidateNarrative(qc)
  // Row thumbnails are keyed by node rather than by query, so they are not in
  // the client's cache at all — see `lib/nodeThumbs.ts`. Choosing a cover or
  // attaching a reference changes what a hundred rows should draw, and this is
  // the one event that says so.
  refreshNodeThumbs()
}

/* ── reads ────────────────────────────────────────────────────────────────── */
