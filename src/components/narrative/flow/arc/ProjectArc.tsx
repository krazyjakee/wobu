import { hasUnsafeInteger } from '../../integerInput'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useStore } from 'zustand'
import type { GraphKey, SceneFile } from '../../../../lib/api'
import { narrativeArc } from '../../../../lib/api/narrativeArc'
import { useCreateScene, useDeleteScene, useScene } from '../../../../lib/queries'
import { useNarrativeWorld } from '../../../../lib/queries/narrativeWorld'
import { useUI, type NarrativeTarget } from '../../../../store/ui'
import { sceneEditKey, useScriptDrafts } from '../../scriptDrafts'
import { applySceneEdit, type SceneEditOperation } from '../../sceneEdits'
import { SceneEditControls } from '../../SceneEditControls'
import { useSceneEditSession } from '../../useSceneEditSession'
import { createFlowStore } from '../flowStore'
import type { CanonicalFlowActions } from '../canonicalFlow'
import type { LayoutRunner } from '../layout'
import { useFlowPresentation } from '../useFlowPresentation'
import { useNarrativeNames } from '../useNarrativeNames'
import { layoutWithPositions, positionsFromLayout } from '../source'
import { ArcFlow, type ArcView } from './ArcFlow'
import { arcGrouping } from './model'
import { draftArcScene, projectArc } from './projectArcModel'
import { projectArcSession } from './projectArcSession'

interface Props {
  projectKey: string
  active: boolean
  readOnly: boolean
  layout?: LayoutRunner
  onEnter: (id: string, beatId?: string | null, target?: Partial<NarrativeTarget>) => void
}
export function ProjectArc(props: Props) {
  const session = projectArcSession(props.projectKey)
  const [scope, setScope] = useState(session.scope)
  const world = useNarrativeWorld()
  const questNames = new Map(world.data?.document.quests.map((quest) => [quest.id, quest.name]))
  const questFindings =
    world.data?.diagnostics.filter(
      (finding) => finding.recordId && questNames.has(finding.recordId),
    ) ?? []
  const [findingPage, setFindingPage] = useState(0)
  const finalPage = Math.max(0, Math.ceil(questFindings.length / 25) - 1)
  const page = Math.min(findingPage, finalPage)
  const set = (value: string) => {
    projectArcSession(props.projectKey).scope = value
    setScope(value)
  }
  return (
    <div className="nrt-quest-arrangement">
      <nav aria-label="Flow level">
        <span aria-current="page">
          {world.data?.document.quests.find((q) => q.id === scope)?.name ?? 'Every scene'}
        </span>
      </nav>
      <label className="nrt-quest-selector">
        Flow scope{' '}
        <select aria-label="Flow scope" value={scope} onChange={(e) => set(e.target.value)}>
          <option value="">Every scene</option>
          {world.data?.document.quests.map((quest) => (
            <option key={quest.id} value={quest.id}>
              {quest.name}
            </option>
          ))}
        </select>
      </label>
      {world.isError && (
        <p role="alert">World could not be read. Quest stages and memberships are unavailable.</p>
      )}
      {questFindings.length > 0 && (
        <details className="nrt-arc-checks">
          <summary>
            {questFindings.length} World quest findings. Includes filtered and collapsed quests.
          </summary>
          <ul aria-label="World quest findings">
            {questFindings.slice(page * 25, (page + 1) * 25).map((finding, index) => (
              <li key={`${finding.recordId}:${finding.field}:${index}`}>
                {questNames.get(finding.recordId!)} · {finding.field}: {finding.message}{' '}
                <button
                  className="btn btn-sm"
                  onClick={() =>
                    useUI.getState().openNarrativeWorld({
                      projectKey: props.projectKey,
                      collection: 'quests',
                      recordId: finding.recordId!,
                    })
                  }
                >
                  Open quest
                </button>
              </li>
            ))}
          </ul>
          {finalPage > 0 && (
            <nav aria-label="World quest finding pages">
              <button className="btn" disabled={!page} onClick={() => setFindingPage(page - 1)}>
                Previous findings
              </button>
              <span>
                {page + 1} / {finalPage + 1}
              </span>
              <button
                className="btn"
                disabled={page === finalPage}
                onClick={() => setFindingPage(page + 1)}
              >
                Next findings
              </button>
            </nav>
          )}
        </details>
      )}
      <ArcGraph key={scope} {...props} scope={scope} quests={world.data?.document.quests} />
    </div>
  )
}
function ArcGraph({
  projectKey,
  active,
  readOnly,
  layout,
  onEnter,
  scope,
  quests,
}: Props & { scope: string; quests: Parameters<typeof projectArc>[1] }) {
  const cache = useQueryClient()
  const query = useQuery({
    queryKey: ['narrative_arc', projectKey],
    queryFn: narrativeArc,
    enabled: active,
    retry: false,
  })
  const sessions = projectArcSession(projectKey)
  const [view, setView] = useState<ArcView>(
    sessions.views.get(scope) ?? { mode: 'canvas', grouping: 'quest' },
  )
  const changeView = (next: ArcView) => {
    sessions.views.set(scope, next)
    setView(next)
  }
  const storeKey = `${scope}:${view.grouping}`
  const store = useMemo(() => {
    let found = sessions.stores.get(storeKey)
    if (!found) {
      found = createFlowStore()
      found.getState().select(useUI.getState().narrative.sceneId)
      sessions.stores.set(storeKey, found)
    }
    return found
  }, [sessions, storeKey])
  useEffect(() => {
    const current = useUI.getState().narrative.sceneId
    if (current && query.data?.scenes.some((one) => one.summary.id === current))
      store.getState().select(current)
  }, [store, query.data?.scenes])
  const selectedId = useStore(store, (s) => s.selectedId)
  const selected = useScene(
    query.data?.scenes.some((one) => one.summary.id === selectedId) ? selectedId : null,
  )
  const drafts = useScriptDrafts((s) => s.drafts)
  const { nameOf } = useNarrativeNames()
  const arc = useMemo(
    () =>
      projectArc(
        (query.data?.scenes ?? []).map((base) => {
          const draft = drafts[sceneEditKey(projectKey, base.summary.id)]
          return draft ? draftArcScene(draft.scene, base) : base
        }),
        quests,
        scope,
        nameOf,
      ),
    [query.data, drafts, projectKey, quests, scope, nameOf],
  )
  const graph = useMemo<GraphKey>(
    () =>
      view.grouping === 'arrangement'
        ? scope
          ? { kind: 'quest', quest: scope }
          : { kind: 'arc', arc: 'project' }
        : { kind: 'arc', arc: `${scope || 'project'}-group-${view.grouping}` },
    [scope, view.grouping],
  )
  const stored = useFlowPresentation(graph)
  const derived = view.grouping === 'quest' || view.grouping === 'questState'
  const grouping = useMemo(() => arcGrouping(arc, view.grouping), [arc, view.grouping])
  const presentation = useMemo(() => {
    if (!stored.presentation) return undefined
    const base = stored.presentation.layout
    const groups = derived
      ? Object.fromEntries(
          grouping.groups.map((group) => [
            group.id,
            {
              ...base.groups[group.id],
              id: group.id,
              label: group.name,
              members: arc.level.elements
                .filter((element) => grouping.of(element) === group.id)
                .map((element) => (element.kind === 'scene' ? `scene:${element.id}` : element.id)),
              collapsed: base.groups[group.id]?.collapsed ?? arc.level.elements.length > 300,
              updatedAt: base.groups[group.id]?.updatedAt ?? '1970-01-01T00:00:00.000Z',
            },
          ]),
        )
      : view.grouping === 'none'
        ? {}
        : base.groups
    return {
      ...stored.presentation,
      derivedGroups: derived,
      layout: { ...base, schemaVersion: 3, groups },
    }
  }, [stored.presentation, derived, grouping, arc.level.elements, view.grouping])
  const drawn = useMemo(
    () => ({
      ...arc,
      level: {
        ...arc.level,
        groups: Object.values(presentation?.layout.groups ?? {}).map((group) => ({
          id: group.id,
          name: group.label || group.id,
        })),
        elements: arc.level.elements.map((element) => ({
          ...element,
          groupId:
            Object.values(presentation?.layout.groups ?? {}).find((group) =>
              group.members?.includes(
                element.kind === 'scene' ? `scene:${element.id}` : element.id,
              ),
            )?.id ?? null,
        })),
      },
    }),
    [arc, presentation?.layout.groups],
  )
  const [exitBeat, setExitBeat] = useState<string>('')
  const create = useCreateScene()
  const remove = useDeleteScene()
  const deny = useCallback(
    (message: string) => {
      store.getState().announce(message)
      return false
    },
    [store],
  )
  const dispatch = useCallback(
    (file: SceneFile, operation: SceneEditOperation) => {
      const key = sceneEditKey(projectKey, file.scene.id)
      const draft = useScriptDrafts.getState().drafts[key]
      if (readOnly || draft?.pending) return deny('This scene cannot be edited right now.')
      if (hasUnsafeInteger(draft?.scene ?? file.scene))
        return deny(
          'This scene contains an integer the webview cannot represent exactly. Repair it in Source before editing exits.',
        )
      const result = applySceneEdit(draft?.scene ?? file.scene, operation)
      if ('refused' in result) return deny(result.refused)
      useScriptDrafts.getState().put(key, { file, scene: result.scene })
      store.getState().announce('Scene exit updated in the shared draft. Save scene to commit it.')
      return true
    },
    [deny, projectKey, readOnly, store],
  )
  const actions = useMemo<CanonicalFlowActions>(
    () => ({
      disabled: readOnly || create.isPending || remove.isPending,
      connectionsDisabled:
        !!selected.data &&
        (hasUnsafeInteger(
          drafts[sceneEditKey(projectKey, selected.data.scene.id)]?.scene ?? selected.data.scene,
        ) ||
          !!drafts[sceneEditKey(projectKey, selected.data.scene.id)]?.pending),
      add: () => {
        if (readOnly) return deny('This project is read-only.')
        create.mutate('New scene', {
          onSuccess: (file) => {
            store.getState().select(file.scene.id)
            store.getState().requestNodeReveal(file.scene.id)
            useUI
              .getState()
              .selectNarrative({ sceneId: file.scene.id }, 'flow', { projectKey, focus: false })
            store.getState().announce('Created New scene.')
            void cache.invalidateQueries({ queryKey: ['narrative_arc', projectKey] })
          },
        })
        return true
      },
      connect: (from, to) => {
        if (readOnly) return deny('This project is read-only.')
        if (!selected.data || selected.data.scene.id !== from.elementId)
          return deny('Select the source scene and wait for its exit controls before connecting.')
        if (to && !query.data?.scenes.some((one) => one.summary.id === to))
          return deny('Scene exits connect to scenes. Edit quest transitions in World.')
        const key = sceneEditKey(projectKey, from.elementId)
        const scene = useScriptDrafts.getState().drafts[key]?.scene ?? selected.data.scene
        const existing = arc.level.elements
          .find((one) => one.id === from.elementId)
          ?.out.find((port) => port.id === from.portId)
        if (existing?.routeId && existing.via)
          return dispatch(selected.data, {
            kind: 'setDestination',
            route: {
              kind: existing.choice ? 'choice' : 'outcome',
              beatId: existing.via,
              id: existing.routeId,
            },
            value: to ? { scene: to } : { unresolved: {} },
          })
        const beat = scene.beats?.find((one) => one.id === exitBeat) ?? scene.beats?.at(-1)
        if (!beat) return deny('Open the scene and add a beat before creating its exit.')
        return dispatch(selected.data, {
          kind: 'addOutcome',
          beatId: beat.id,
          to: to ? { scene: to } : { unresolved: {} },
        })
      },
      remove: (id) => {
        if (readOnly) return deny('This project is read-only.')
        if (!query.data?.scenes.some((one) => one.summary.id === id))
          return deny('Quest stages are edited in World.')
        if (useScriptDrafts.getState().drafts[sceneEditKey(projectKey, id)])
          return deny('Save or discard this scene draft before deleting the scene.')
        remove.mutate(id, { onSuccess: () => store.getState().select(null) })
        return true
      },
    }),
    [
      readOnly,
      create,
      remove,
      deny,
      store,
      cache,
      projectKey,
      selected.data,
      drafts,
      query.data,
      arc.level.elements,
      dispatch,
      exitBeat,
    ],
  )
  const root = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const parent = root.current?.closest('.nrt-panel[role=tabpanel]') ?? root.current?.parentElement
    if (parent) parent.scrollTop = sessions.scroll
    const frame = requestAnimationFrame(() => {
      if (document.activeElement !== document.body || !root.current) return
      const selected = store.getState().selectedId
      const node = Array.from(
        root.current.querySelectorAll<HTMLElement>('[data-flow-id],.react-flow__node[data-id]'),
      ).find((one) => (one.dataset.flowId ?? one.dataset.id) === selected)
      const target =
        node ??
        root.current.querySelector<HTMLButtonElement>(
          '[aria-label="Arc view mode"] button[aria-pressed="true"]',
        )
      target?.focus({ preventScroll: true })
    })
    return () => {
      cancelAnimationFrame(frame)
      if (parent) sessions.scroll = parent.scrollTop
    }
  }, [sessions, store, query.isSuccess])
  if (query.isPending) return <p role="status">Reading the project arc…</p>
  if (query.isError)
    return (
      <div role="alert">
        Could not read the arc.{' '}
        <button className="btn" onClick={() => void query.refetch()}>
          Retry arc
        </button>
      </div>
    )
  const counts = arc.level.elements.reduce(
    (sum, node) => ({
      needsText: sum.needsText + (node.counts?.needsText ?? 0),
      needsReview: sum.needsReview + (node.counts?.needsReview ?? 0),
      outOfDate: sum.outOfDate + (node.counts?.outOfDate ?? 0),
    }),
    { needsText: 0, needsReview: 0, outOfDate: 0 },
  )
  return (
    <div ref={root} className="nrt-pane nrt-flow" data-arc-scenes={query.data.scenes.length}>
      <p role="status">
        {query.data.scenes.length} scenes · {counts.needsText} missing text · {counts.needsReview}{' '}
        needs review · {counts.outOfDate} out of date. Counts include filtered and collapsed scenes.
      </p>
      <details>
        <summary>Reading scene exits and quest stages</summary>
        <p>
          Scene exits come from authored choices and outcomes. Quest stage edges come from World
          transitions; they do not imply a scene order. Group by quest state means the quest’s
          authored initial stage. Shared scenes appear once in a combined membership group. Open a
          stage to edit its quest in World.
        </p>
      </details>
      {!!query.data.unreadable.length && (
        <p role="alert">
          {query.data.unreadable.length} unreadable or ambiguous scene files remain release
          blockers. Open the Scene library to repair them.
        </p>
      )}
      {stored.outcome && stored.outcome.outcome !== 'written' && (
        <p role="status">Arrangement could not be saved. Your source is unchanged.</p>
      )}
      {selected.isError && (
        <p role="alert">
          Could not read the selected scene's exits.{' '}
          <button className="btn" onClick={() => void selected.refetch()}>
            Retry scene exits
          </button>
        </p>
      )}
      {selected.data && (
        <ArcSceneControls
          key={selected.data.scene.id}
          file={selected.data}
          projectKey={projectKey}
          readOnly={readOnly}
          exitBeat={exitBeat}
          onExitBeat={setExitBeat}
          onEnter={() => onEnter(selected.data!.scene.id)}
        />
      )}
      <ArcFlow
        arc={drawn}
        onChange={() => {}}
        onEnter={onEnter}
        onQuest={(questId) =>
          useUI
            .getState()
            .openNarrativeWorld({ projectKey, collection: 'quests', recordId: questId })
        }
        readOnly={readOnly}
        layout={layout}
        positions={positionsFromLayout(presentation?.layout, 'arc')}
        onPositionsChange={(next) =>
          presentation?.onChange(layoutWithPositions(presentation.layout, next, 'arc'))
        }
        presentation={presentation}
        actions={actions}
        authoring={{ fixedPort: 'Quest transitions are edited in World.' }}
        store={store}
        view={view}
        onView={changeView}
        persistentGrouping
      />
    </div>
  )
}
function ArcSceneControls({
  file,
  projectKey,
  readOnly,
  exitBeat,
  onExitBeat,
  onEnter,
}: {
  file: SceneFile
  projectKey: string
  readOnly: boolean
  exitBeat: string
  onExitBeat: (id: string) => void
  onEnter: () => void
}) {
  const session = useSceneEditSession(file, projectKey, readOnly)
  return (
    <section aria-label="Selected scene exits">
      <h3>{session.scene.name}</h3>
      <SceneEditControls session={session} />
      {session.unsafeInteger && (
        <p role="alert">
          This scene contains an unsafe integer. Source editing is required before changing its
          exits.
        </p>
      )}
      {!session.scene.beats?.length && (
        <button
          className="btn"
          disabled={session.disabled}
          onClick={() => {
            const result = applySceneEdit(session.scene, { kind: 'addBeat', title: 'New beat' })
            if (!('refused' in result)) session.edit(result.scene)
          }}
        >
          Add first beat for scene exits
        </button>
      )}
      <label>
        New exit belongs to beat{' '}
        <select
          aria-label="New exit beat"
          disabled={session.disabled}
          value={
            session.scene.beats?.some((beat) => beat.id === exitBeat)
              ? exitBeat
              : (session.scene.beats?.at(-1)?.id ?? '')
          }
          onChange={(e) => onExitBeat(e.target.value)}
        >
          {session.scene.beats?.map((beat) => (
            <option key={beat.id} value={beat.id}>
              {beat.title}
            </option>
          ))}
        </select>
      </label>
      <button className="btn" onClick={onEnter}>
        Open selected scene
      </button>
    </section>
  )
}
