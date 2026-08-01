import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties } from 'react'
import type { Conflict, NodeKind, NodeSummary, ProjectSummary } from '../lib/api'
import { errorMessage } from '../lib/api'
import { useConflicts, useCorruptFiles, useKinds, useNodes } from '../lib/queries'
import { indexKinds } from '../lib/kinds'
import { ancestorsOf, buildGroups, indexNodes } from '../lib/tree'
import { useUI, report } from '../store/ui'
import { TitleBar } from './TitleBar'
import { ModeRail } from './ModeRail'
import { StatusBar } from './StatusBar'
import { Navigator } from './navigator/Navigator'
import { Editor } from './editor/Editor'
import { Inspector } from './Inspector'
import { PromptBox } from './inspector/PromptBox'
import { CommandPalette } from './CommandPalette'
import { NewNodeSheet } from './NewNodeSheet'
import { MilestoneMode } from './MilestoneMode'
import { Settings } from './Settings'
import { Banners } from './Banners'
import { ConflictCard, ConflictsElsewhere } from './ConflictCard'
import { useKeyboard } from '../hooks/useKeyboard'

const RAIL = 52

export function Workspace({ project }: { project: ProjectSummary }) {
  const kindsQ = useKinds()
  const nodesQ = useNodes(true)
  const corruptQ = useCorruptFiles(true)
  const conflictsQ = useConflicts(true)

  const mode = useUI((s) => s.mode)
  const navWidth = useUI((s) => s.navWidth)
  const setNavWidth = useUI((s) => s.setNavWidth)
  const navCollapsed = useUI((s) => s.navCollapsed)
  const inspCollapsed = useUI((s) => s.inspCollapsed)
  const selectedId = useUI((s) => s.selectedId)
  const select = useUI((s) => s.select)
  const openAncestors = useUI((s) => s.openAncestors)

  const [newNode, setNewNode] = useState<
    { kind: NodeKind | null; parentId: string | null } | null
  >(null)

  const kindIndex = useMemo(() => indexKinds(kindsQ.data), [kindsQ.data])
  const nodes = useMemo(() => nodesQ.data ?? [], [nodesQ.data])
  const byId = useMemo(() => indexNodes(nodes), [nodes])

  /** Registry order drives group order; singletons are pinned, not grouped. */
  const kindOrder = useMemo<NodeKind[]>(
    () => (kindsQ.data ?? []).filter((k) => !k.singleton).map((k) => k.kind),
    [kindsQ.data],
  )

  const singletonKinds = useMemo(
    () => new Set((kindsQ.data ?? []).filter((k) => k.singleton).map((k) => k.kind)),
    [kindsQ.data],
  )

  const pinned = useMemo<NodeSummary[]>(() => {
    const order = (kindsQ.data ?? []).filter((k) => k.singleton).map((k) => k.kind)
    const out: NodeSummary[] = []
    for (const k of order) for (const n of nodes) if (n.kind === k) out.push(n)
    return out
  }, [kindsQ.data, nodes])

  const groups = useMemo(
    () => buildGroups(nodes.filter((n) => !singletonKinds.has(n.kind)), kindOrder, kindIndex),
    [nodes, singletonKinds, kindOrder, kindIndex],
  )

  const conflicts = useMemo(() => conflictsQ.data ?? [], [conflictsQ.data])

  const selected = selectedId ? (byId.get(selectedId) ?? null) : null
  const chain = useMemo(
    () => (selected ? ancestorsOf(selected.id, byId) : []),
    [selected, byId],
  )

  // Conflicts on the node the editor is showing get a card; the rest get one
  // line saying where they are. A conflict on a node nobody happens to open is
  // otherwise invisible forever, which is the same silent loss the whole
  // feature exists to prevent — just slower.
  const [conflictsHere, conflictsElsewhere] = useMemo(() => {
    const here: Conflict[] = []
    const elsewhere: Conflict[] = []
    for (const c of conflicts) {
      if (selected && c.nodeId === selected.id) here.push(c)
      else elsewhere.push(c)
    }
    return [here, elsewhere]
  }, [conflicts, selected])

  // A selection that no longer exists (deleted, or removed on disk) is dropped
  // rather than left pointing at nothing.
  useEffect(() => {
    if (selectedId && !nodesQ.isPending && !byId.has(selectedId)) select(null)
  }, [selectedId, byId, nodesQ.isPending, select])

  const jumpTo = useCallback(
    (id: string) => {
      const target = byId.get(id)
      if (target) openAncestors(ancestorsOf(id, byId).map((n) => n.id))
      select(id)
      useUI.getState().setMode('library')
    },
    [byId, select, openAncestors],
  )

  const openNewNode = useCallback(() => {
    const sel = useUI.getState().selectedId
    const s = sel ? byId.get(sel) : undefined
    setNewNode({
      kind: s && !singletonKinds.has(s.kind) ? s.kind : null,
      parentId: s?.parentId ?? null,
    })
  }, [byId, singletonKinds])

  useKeyboard({ onNewNode: openNewNode })

  /* ── navigator resize ── */
  const dragging = useRef(false)
  const [isDragging, setIsDragging] = useState(false)
  useEffect(() => {
    if (!isDragging) return
    const move = (e: MouseEvent) => {
      if (!dragging.current) return
      setNavWidth(e.clientX - RAIL)
    }
    const up = () => {
      dragging.current = false
      setIsDragging(false)
    }
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
    return () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
    }
  }, [isDragging, setNavWidth])

  // Banners describe a condition of *this* project folder, so opening a
  // different one starts clean rather than inheriting the last one's trouble.
  const clearBanners = useUI((s) => s.clearBanners)
  useEffect(() => clearBanners(), [project.id, clearBanners])

  useEffect(() => {
    if (kindsQ.isError) report(kindsQ.error, 'Kind registry unavailable')
  }, [kindsQ.isError, kindsQ.error])

  useEffect(() => {
    if (nodesQ.isError) report(nodesQ.error, 'Could not list nodes')
  }, [nodesQ.isError, nodesQ.error])

  // Collapsed panes drop their column entirely rather than shrinking to zero,
  // so auto-placement can't slide the editor into an empty track.
  const style: CSSProperties = {
    gridTemplateColumns: [
      'var(--rail)',
      navCollapsed || mode !== 'library' ? null : `${navWidth}px`,
      '1fr',
      inspCollapsed || mode !== 'library' ? null : 'var(--insp)',
    ]
      .filter(Boolean)
      .join(' '),
  }

  return (
    <div className="app">
      <TitleBar project={project} chain={chain} selected={selected} kinds={kindIndex} />
      <Banners />

      <div className="workspace" style={style}>
        <ModeRail />

        {mode === 'library' ? (
          <>
            {!navCollapsed && (
              <Navigator
                nodes={nodes}
                byId={byId}
                pinned={pinned}
                groups={groups}
                kinds={kindIndex}
                loading={nodesQ.isPending}
                error={nodesQ.isError ? errorMessage(nodesQ.error) : null}
                readOnly={project.readOnly}
                corrupt={corruptQ.data ?? []}
                projectPath={project.path}
                onNewNode={(kind, parentId) => setNewNode({ kind, parentId })}
              />
            )}
            {!navCollapsed && (
              <div
                className={isDragging ? 'resizer is-dragging' : 'resizer'}
                style={{ left: RAIL + navWidth }}
                onMouseDown={(e) => {
                  e.preventDefault()
                  dragging.current = true
                  setIsDragging(true)
                }}
                role="separator"
                aria-orientation="vertical"
              />
            )}
            {/* The conflict card belongs above the editor rather than inside
                it: it is about the *file*, not about the node being edited, and
                it has to stay visible while the user scrolls the notes they are
                comparing it against. The wrapper is what keeps both of them in
                the single `1fr` track — as separate grid children the card
                would land in the inspector's column. */}
            <div className="editor-region">
              {(conflictsHere.length > 0 || conflictsElsewhere.length > 0) && (
                <div className="conflicts">
                  {conflictsHere.map((c) => (
                    <ConflictCard key={c.relPath} conflict={c} projectPath={project.path} />
                  ))}
                  <ConflictsElsewhere conflicts={conflictsElsewhere} onJump={jumpTo} />
                </div>
              )}
              <Editor
                selected={selected}
                chain={chain}
                kinds={kindIndex}
                readOnly={project.readOnly}
                onJump={jumpTo}
                hasNodes={nodes.length > 0}
                loading={nodesQ.isPending}
              />
            </div>
            {/* The compiled prompt sits under the influence stack, in the
                inspector's own column — the wrapper is what keeps the two in
                one grid track, exactly as `.editor-region` does for the
                conflict card. It is rendered from here rather than from inside
                the Inspector only until the stack itself lands; see
                `PromptBox`. */}
            {!inspCollapsed && (
              <div className="insp-region">
                <Inspector selected={selected} kinds={kindIndex} />
                <PromptBox project={project} subject={selected} onJump={jumpTo} />
              </div>
            )}
          </>
        ) : mode === 'settings' ? (
          <Settings />
        ) : (
          <MilestoneMode mode={mode} />
        )}
      </div>

      <StatusBar project={project} nodeCount={nodes.length} loading={nodesQ.isPending} />

      <CommandPalette nodes={nodes} kinds={kindIndex} onJump={jumpTo} onNewNode={openNewNode} />

      {newNode && (
        <NewNodeSheet
          initialKind={newNode.kind}
          initialParentId={newNode.parentId}
          nodes={nodes}
          kinds={kindsQ.data ?? []}
          onClose={() => setNewNode(null)}
          onCreated={(id) => {
            setNewNode(null)
            jumpTo(id)
          }}
        />
      )}
    </div>
  )
}
