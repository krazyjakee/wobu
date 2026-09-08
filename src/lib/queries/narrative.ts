import { assertProjectSession, projectSessionEpoch } from '../projectSession'
import { useProjectMutation } from './useProjectMutation'
import {
  useMutation,
  useQueries,
  useQuery,
  useQueryClient,
  type UseQueryResult,
} from '@tanstack/react-query'
import * as api from '../api'
import type { GraphKey, LayoutLoad, Scene, SceneFile } from '../api'
import { sceneBirthEntry, sceneDeletionEntry, sceneEditEntry, useUndoStack } from '../undo'
import { report } from '../../store/ui'
import { invalidateNarrative, qk } from './keys'
/* ── keys ─────────────────────────────────────────────────────────────────── */

/**
 * TanStack hooks over the narrative command surface.
 *
 * Three of these write, and all three record undo. `useSceneWrite` is the choke
 * point for *edits* — every structural or textual change to a scene document
 * goes through it, because the backend has one write for a scene document and
 * no per-field commands to route around it. `useCreateScene` and
 * `useDeleteScene` are its lifecycle counterparts, exactly as `useCreateNode`
 * and `useDeleteNode` are for nodes.
 *
 * The point of that arrangement is what it does for code nobody has written
 * yet. A canvas button that connects an outcome to a beat produces a new scene
 * document and hands it to `useSaveScene`; it is undoable the day it is
 * written, its history entry is indistinguishable from the same edit made in a
 * form, and its author never has to have heard of `lib/undo.ts`.
 *
 * **Layout hooks record nothing.** Moving a box is not a story change, and the
 * reasoning for that lives in `lib/undo.ts` beside the primitives it is a
 * statement about.
 */

/* ── reads ────────────────────────────────────────────────────────────────── */

/**
 * Every scene in the project, and the files that could not be identified.
 *
 * Content-checked on each fetch; the rebuildable index accelerates scene discovery,
 * so there is no cheaper answer that is also a correct one.
 */
export function useScenes(): UseQueryResult<api.SceneCatalog> {
  return useQuery({
    queryKey: qk.narrativeScenes,
    queryFn: api.narrativeScenes,
    retry: false,
  })
}

/**
 * One scene, whole.
 *
 * This is where the guarded-write precondition enters the frontend: the cached
 * `SceneFile` carries the stamp the file had when it was read, and every save
 * hands that same object back. Nothing anywhere derives a stamp, stores one
 * beside a scene, or keeps a second copy — the document and its precondition
 * travel together or the precondition is worthless.
 */
export function useScene(sceneId: string | null): UseQueryResult<SceneFile> {
  return useQuery({
    queryKey: qk.narrativeScene(sceneId ?? ''),
    queryFn: () => api.narrativeSceneGet(sceneId as string),
    enabled: !!sceneId,
    retry: false,
  })
}

/** Full documents for bounded form callers such as variable-reference inspection.
 * Arc navigation uses the compact narrative_arc projection and fetches a full
 * scene only when the writer selects its exit controls or enters it.
 */
export function useSceneFiles(ids: readonly string[], enabled = true) {
  return useQueries({
    queries: ids.map((id) => ({
      queryKey: qk.narrativeScene(id),
      queryFn: () => api.narrativeSceneGet(id),
      enabled,
      retry: false,
    })),
  })
}

/**
 * What is wrong with a scene as it is saved on disk.
 *
 * The Validation list and the Library badges read this. An editor holding
 * unsaved edits wants `useDiagnoseScene` instead — see there for why the two
 * are not one hook.
 */
export function useSceneDiagnostics(
  sceneId: string | null,
): UseQueryResult<api.NarrativeDiagnostic[]> {
  return useQuery({
    queryKey: qk.narrativeDiagnostics(sceneId ?? ''),
    queryFn: () => api.narrativeDiagnostics(sceneId as string),
    enabled: !!sceneId,
    retry: false,
  })
}

/**
 * Diagnose a document the writer has not saved yet.
 *
 * A mutation rather than a query, and deliberately so. Diagnostics for an
 * unsaved buffer are a function of a document that changes on every keystroke,
 * so a cache keyed by it would hold one entry per keystroke, and a cache keyed
 * by the scene id would confidently serve the answer for a *different*
 * document — which is worse, because it would show a destination as broken
 * thirty seconds after the writer connected it. A mutation is the honest
 * primitive for "run this now, with exactly these arguments".
 *
 * The scene catalog and the declared variables still come from the project, so
 * a cross-scene link is checked against the scenes that really exist rather
 * than against anything the buffer asserts.
 */
export function useDiagnoseScene() {
  return useMutation({
    mutationFn: (value: { sceneId: string; scene: Scene }) =>
      api.narrativeDiagnostics(value.sceneId, value.scene),
  })
}

/** The declared variables, or an empty document in a project that has none. */
export function useNarrativeState(): UseQueryResult<api.StateFile> {
  return useQuery({
    queryKey: qk.narrativeState,
    queryFn: api.narrativeStateGet,
    retry: false,
  })
}

/**
 * Where the boxes are for one graph.
 *
 * `retry: false` costs nothing here — the command cannot fail — and saying so
 * is worth more than the line: a missing sidecar, a corrupt one and one from a
 * newer Wobu all arrive as notices beside a usable layout, so there is never
 * anything for a retry to fix.
 */
export function useSceneLayout(graph: GraphKey | null): UseQueryResult<LayoutLoad> {
  return useQuery({
    queryKey: qk.narrativeLayout(graph ?? { kind: 'arc', arc: '' }),
    queryFn: () => api.narrativeLayoutGet(graph as GraphKey),
    enabled: !!graph,
    retry: false,
  })
}

/* ── the one place a scene edit is recorded ───────────────────────────────── */

/**
 * Run a write against one scene document and record what it did.
 *
 * `before` is resolved in `onMutate`, i.e. before the write lands, because
 * afterwards the previous version is not knowable from anywhere. A caller that
 * already holds the file it opened returns it; a caller that only has an id
 * fetches it, and a fetch that fails yields `null` — the write still goes
 * ahead, and the only cost is that this one action cannot be undone. An undo
 * that restores a guess is worse than no undo.
 *
 * Errors are reported here rather than at each call site, for the same reason
 * the undo entry is pushed here: a future canvas button should get the whole of
 * Wobu's behaviour by calling one hook.
 */
function useSceneWrite<V>(options: {
  before: (value: V) => Promise<SceneFile | null> | SceneFile | null
  run: (value: V) => Promise<SceneFile>
  whileDoing: string
  projectKey?: string
}) {
  const qc = useQueryClient()
  return useProjectMutation({
    mutationFn: (value: V) => {
      if (
        options.projectKey !== undefined &&
        qc.getQueryData<api.ProjectSummary | null>(qk.projectCurrent)?.path !== options.projectKey
      )
        throw new Error('The project changed before the scene could be saved.')
      return options.run(value)
    },
    onMutate: async (value: V) => ({
      project: qc.getQueryData<api.ProjectSummary | null>(qk.projectCurrent)?.path,
      undoProject: useUndoStack.getState().projectId,
      before: await options.before(value),
    }),
    onSuccess: (file, _value, context) => {
      if (
        context?.project !== qc.getQueryData<api.ProjectSummary | null>(qk.projectCurrent)?.path ||
        context?.undoProject !== useUndoStack.getState().projectId
      )
        return
      // The authoritative answer, including the refreshed precondition, and it
      // is set rather than invalidated on purpose: a refetch would replace a
      // correct stamp with an equal one, and the window where the cache holds
      // neither is a window where the next save sends a stale precondition.
      qc.setQueryData(qk.narrativeScene(file.scene.id), file)
      const entry = context?.before && sceneEditEntry(context.before, file)
      if (entry)
        useUndoStack
          .getState()
          .push(options.projectKey === undefined ? entry : { ...entry, coalesce: false })
      void qc.invalidateQueries({ queryKey: qk.narrativeScenes })
      // The Scene library is a *projection* of what was just written — the
      // title, the text coverage, the recorded status — and it stays mounted
      // behind the editor, so nothing else would ever refetch it. Without this
      // a rename, an approved line or a filled slot is invisible until the
      // writer reopens the project, which reads as the save having been lost.
      void qc.invalidateQueries({ queryKey: ['narrative_library'] })
      void qc.invalidateQueries({ queryKey: ['narrative_arc'] })
      // A scene's own problems changed, and so did every other scene's: a
      // destination that named a beat in here is checked against this document.
      void qc.invalidateQueries({ queryKey: ['narrative_diagnostics'] })
    },
    onError: (error, _value, context) => {
      if (context?.project !== qc.getQueryData<api.ProjectSummary | null>(qk.projectCurrent)?.path)
        return
      // A save that lost the race parked a sibling on disk a moment ago, and
      // the conflict card has to appear now rather than whenever the watcher
      // next fires — on a share that is a five-second poll, and five seconds of
      // believing a scene is gone is the failure the card exists to prevent.
      if (api.errorCode(error) === 'write.conflict') invalidateNarrative(qc)
      report(error, options.whileDoing)
    },
  })
}

/**
 * Write a whole scene document.
 *
 * The hook every editor of a scene calls: the Flow canvas, the outline list,
 * the Script forms and the Source tab. It takes the `SceneFile` the caller
 * loaded — not just the document — because that file is where the write
 * precondition lives, and separating the two is how a save ends up guarded by
 * a stamp from some other version.
 */
export function useSaveScene(projectKey?: string) {
  return useSceneWrite({
    before: (value: { file: SceneFile; scene: Scene }) => value.file,
    run: (value) =>
      api.narrativeSceneSave(value.scene, api.preconditionOf(value.file.stamp), value.file.slug),
    whileDoing: 'Could not save that scene',
    projectKey,
  })
}

/**
 * Change a scene's display name.
 *
 * Takes an id rather than a file, because the Library row that offers Rename
 * has one — and reads the scene itself so the rename is still recorded with a
 * real previous version rather than an invented one.
 */
export function useRenameScene() {
  return useSceneWrite({
    before: (value: { sceneId: string; name: string }) =>
      api.narrativeSceneGet(value.sceneId).catch(() => null),
    run: (value) => api.narrativeSceneRename(value.sceneId, value.name),
    whileDoing: 'Could not rename that scene',
  })
}

/* ── lifecycle ────────────────────────────────────────────────────────────── */

export function useCreateScene() {
  const qc = useQueryClient()
  return useProjectMutation({
    mutationFn: (name: string) => api.narrativeSceneCreate(name),
    onSuccess: (file) => {
      qc.setQueryData(qk.narrativeScene(file.scene.id), file)
      useUndoStack.getState().push(sceneBirthEntry(file, 'create'))
      invalidateNarrative(qc)
    },
    onError: (error) => report(error, 'Could not create that scene'),
  })
}

/**
 * Delete a scene.
 *
 * Everything undo will need is read *before* the delete, because afterwards
 * none of it is knowable — the file is gone, and with it every id under it that
 * a destination elsewhere, a locale row or a recording still names. A read that
 * fails is not a delete that fails: the delete goes ahead, and the only cost is
 * that this one action cannot be undone.
 */
export function useDeleteScene() {
  const qc = useQueryClient()
  return useProjectMutation({
    mutationFn: async (sceneId: string) => {
      const epoch = projectSessionEpoch()
      const before = await api.narrativeSceneGet(sceneId).catch(() => null)
      assertProjectSession(epoch)
      await api.narrativeSceneDelete(sceneId)
      return before
    },
    onSuccess: (before, sceneId) => {
      qc.removeQueries({ queryKey: qk.narrativeScene(sceneId) })
      if (before) useUndoStack.getState().push(sceneDeletionEntry(before))
      invalidateNarrative(qc)
    },
    onError: (error) => report(error, 'Could not delete that scene'),
  })
}

/* ── declared state ───────────────────────────────────────────────────────── */

/** Save declarations with exact authored-state guards for canonical undo and redo. */
export function useSaveNarrativeState(projectKey?: string) {
  const qc = useQueryClient()
  return useProjectMutation({
    mutationFn: (value: {
      document: api.StateDocument
      expected: api.Precondition
      file?: api.StateFile
    }) => {
      if (
        projectKey !== undefined &&
        qc.getQueryData<api.ProjectSummary | null>(qk.projectCurrent)?.path !== projectKey
      )
        throw new Error('The project changed before variables could be saved.')
      return api.narrativeStateSave(value.document, value.expected)
    },
    onMutate: async (value) => ({
      project: qc.getQueryData<api.ProjectSummary | null>(qk.projectCurrent)?.path,
      undoProject: useUndoStack.getState().projectId,
      file: value.file ?? (await api.narrativeStateGet()),
    }),
    onSuccess: (file, _value, before) => {
      if (
        before?.project !== qc.getQueryData<api.ProjectSummary | null>(qk.projectCurrent)?.path ||
        before?.undoProject !== useUndoStack.getState().projectId
      )
        return
      if (before && JSON.stringify(before.file.document) !== JSON.stringify(file.document)) {
        useUndoStack.getState().push({
          subjectId: 'narrative-state',
          label: 'edit declared variables',
          coalesce: false,
          undo: [{ type: 'stateRestore', document: before.file.document, expected: file.document }],
          redo: [{ type: 'stateRestore', document: file.document, expected: before.file.document }],
        })
      }
      qc.setQueryData(qk.narrativeState, file)
      void qc.invalidateQueries({ queryKey: ['narrative_world'] })
      // Every condition and effect in the project is typed against these, so
      // the answer to "what is wrong with this scene" just changed everywhere.
      void qc.invalidateQueries({ queryKey: ['narrative_diagnostics'] })
    },
    onError: (error, _value, before) => {
      if (before?.project !== qc.getQueryData<api.ProjectSummary | null>(qk.projectCurrent)?.path)
        return
      if (api.errorCode(error) === 'write.conflict') invalidateNarrative(qc)
      report(error, 'Could not save the declared state')
    },
  })
}

/* ── layout ───────────────────────────────────────────────────────────────── */

/**
 * Save an arrangement.
 *
 * Records nothing on the undo stack — see the note in `lib/undo.ts` about why
 * there is no command there that could carry a coordinate — and reports
 * nothing to the user. The command cannot fail, so there is no rejection to
 * catch; a `deferred` or `unwritable` outcome is information for a canvas to
 * show quietly, in the one place the user is already looking, rather than a
 * toast per rectangle.
 *
 * Refetch only the merged arrangement after a successful write. Source and
 * compilation queries remain untouched; refused saves keep the canonical cache.
 */
export function useSaveLayout() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (layout: api.Layout) => api.narrativeLayoutSave(layout),
    onSuccess: async (outcome, layout) => {
      if (outcome.outcome !== 'written') return
      // The merge may contain a collaborator's moves. Cache the authoritative
      // merged document rather than echoing only the submitted coordinates.
      await qc.fetchQuery({
        queryKey: qk.narrativeLayout(layout.graph),
        queryFn: () => api.narrativeLayoutGet(layout.graph),
        staleTime: 0,
      })
    },
  })
}
