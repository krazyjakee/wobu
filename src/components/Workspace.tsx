import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { ComponentProps, CSSProperties } from 'react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import type { Conflict, NodeKind, NodeSummary, ProjectSummary } from '../lib/api'
import { errorMessage } from '../lib/api'
import {
  useConflicts,
  useCorruptFiles,
  useKinds,
  useJobQueue,
  useNodes,
  usePresence,
  useProjectSync,
  useReportEditing,
  useStatusBarBackend,
} from '../lib/queries'
import { indexKinds } from '../lib/kinds'
import { PRESENCE_BANNER, editingText, editorsByNode, editorsOf, openedText } from '../lib/presence'
import { READ_ONLY_BANNER, READ_ONLY_TEXT } from '../lib/readOnly'
import { ancestorsOf, buildGroups, indexNodes } from '../lib/tree'
import { useUI, report, toast } from '../store/ui'
import { TitleBar } from './TitleBar'
import { ModeRail } from './ModeRail'
import { StatusBar } from './StatusBar'
import { Navigator } from './navigator/Navigator'
import { Editor } from './editor/Editor'
import { Inspector } from './Inspector'
import { CommandPalette } from './CommandPalette'
import { NewNodeSheet } from './NewNodeSheet'
import { StyleTransferSheet } from './StyleTransferSheet'
import { AssetsMode } from './AssetsMode'
import { ForgeMode } from './ForgeMode'
import { Settings } from './Settings'
import { Banners } from './Banners'
import { ConflictCard, ConflictsElsewhere } from './ConflictCard'
import { useKeyboard } from '../hooks/useKeyboard'

const RAIL = 52

export function Workspace({ project }: { project: ProjectSummary }) {
  const readOnly = project.readOnly
  const kindsQ = useKinds()
  const nodesQ = useNodes(true)
  const corruptQ = useCorruptFiles(true)
  const conflictsQ = useConflicts(true)
  const { peers, ready: presenceReady } = usePresence(true)
  const sync = useProjectSync(project.id)
  const backend = useStatusBarBackend(project.id).data ?? null
  const queue = useJobQueue()

  const mode = useUI((s) => s.mode)
  const navWidth = useUI((s) => s.navWidth)
  const setNavWidth = useUI((s) => s.setNavWidth)
  const navCollapsed = useUI((s) => s.navCollapsed)
  const inspCollapsed = useUI((s) => s.inspCollapsed)
  const selectedId = useUI((s) => s.selectedId)
  const select = useUI((s) => s.select)
  const openAncestors = useUI((s) => s.openAncestors)

  const [newNode, setNewNode] = useState<{ kind: NodeKind | null; parentId: string | null } | null>(
    null,
  )
  const [transferSource, setTransferSource] = useState<string | null>(null)

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
    () =>
      buildGroups(
        nodes.filter((n) => !singletonKinds.has(n.kind)),
        kindOrder,
        kindIndex,
      ),
    [nodes, singletonKinds, kindOrder, kindIndex],
  )

  const conflicts = useMemo(() => conflictsQ.data ?? [], [conflictsQ.data])

  const selected = selectedId ? (byId.get(selectedId) ?? null) : null
  const chain = useMemo(() => (selected ? ancestorsOf(selected.id, byId) : []), [selected, byId])

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

  // Everyone else's Wobu is told which node we have open, so their navigator can
  // put a dot on it. Advisory in both directions: nothing sent here reserves a
  // node, and nothing that comes back disables anything below.
  useReportEditing(selectedId)

  const peerEditors = useMemo(() => editorsByNode(peers), [peers])
  const editorsHere = useMemo(() => editorsOf(selectedId, peers), [selectedId, peers])

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

  // The one gate every route to the New-node sheet passes through — the
  // navigator's button and group headers, the palette, ⌘N. Disabling the
  // controls is what the user sees; this is what makes it true.
  const startNewNode = useCallback(
    (kind: NodeKind | null, parentId: string | null) => {
      if (readOnly) return
      setNewNode({ kind, parentId })
    },
    [readOnly],
  )

  const openNewNode = useCallback(() => {
    const sel = useUI.getState().selectedId
    const s = sel ? byId.get(sel) : undefined
    startNewNode(s && !singletonKinds.has(s.kind) ? s.kind : null, s?.parentId ?? null)
  }, [byId, singletonKinds, startNewNode])

  const startStyleTransfer = useCallback(async () => {
    if (readOnly) return
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: 'Choose a Wobu project to import from',
      })
      if (typeof picked === 'string') setTransferSource(picked)
    } catch (error) {
      report(error, 'Could not choose a source project')
    }
  }, [readOnly])

  useKeyboard({ onNewNode: openNewNode, readOnly })

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
  //
  // Read-only is raised in the same breath, and this is the only place it is
  // said: it was settled by `paths::is_writable()` at open and holds for the
  // whole session, so it belongs to the project rather than to a node or a
  // control. Keyed by code, so even if this ran again it would collapse to one.
  const clearBanners = useUI((s) => s.clearBanners)
  useEffect(() => {
    clearBanners()
    if (readOnly) {
      useUI
        .getState()
        .raiseBanner({ code: READ_ONLY_BANNER, text: READ_ONLY_TEXT, retryable: false })
    }
  }, [project.id, readOnly, clearBanners])

  // Both presence effects below are declared *after* that one on purpose:
  // effects run in source order, so opening a project clears the banners first
  // and this raises against the clean slate rather than into one about to be
  // wiped.

  // "Nadia has this project open." — once per project open, on the first answer
  // that comes back rather than on every poll, and silent when nobody else is
  // here. Announcing an empty folder every time is how the one message that
  // matters gets dismissed unread.
  const greeted = useRef<string | null>(null)
  useEffect(() => {
    if (!presenceReady || greeted.current === project.id) return
    greeted.current = project.id
    const text = openedText(peers)
    if (text) toast(text)
  }, [project.id, presenceReady, peers])

  // The passive banner, raised once per *event* — this node becoming one that
  // somebody else has open — rather than once per poll. Keyed by code exactly as
  // the read-only banner is (#19), and the remembered key is what lets a
  // dismissal stick until the situation itself changes.
  //
  // Informational, and that is the whole design: it names what happens if you
  // both save. Nothing under it is disabled, and nothing may be.
  const bannerKey = useRef<string | null>(null)
  useEffect(() => {
    const key = editorsHere.length
      ? `${project.id}|${selectedId}|${editorsHere.map((p) => p.sessionId).join(',')}`
      : null
    if (bannerKey.current === key) return
    bannerKey.current = key
    if (!key) {
      useUI.getState().clearBanner(PRESENCE_BANNER)
      return
    }
    useUI.getState().raiseBanner({
      code: PRESENCE_BANNER,
      text: editingText(editorsHere, selected?.name ?? 'this node'),
      retryable: false,
    })
  }, [project.id, selectedId, editorsHere, selected?.name])

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
  const navigatorProps: ComponentProps<typeof Navigator> = {
    nodes,
    byId,
    pinned,
    groups,
    kinds: kindIndex,
    loading: nodesQ.isPending,
    error: nodesQ.isError ? errorMessage(nodesQ.error) : null,
    readOnly,
    corrupt: corruptQ.data ?? [],
    editedElsewhere: peerEditors,
    projectPath: project.path,
    onNewNode: startNewNode,
    onStyleTransfer: () => void startStyleTransfer(),
  }

  return (
    <div className="app">
      <TitleBar project={project} chain={chain} selected={selected} kinds={kindIndex} />
      <Banners />

      <div className="workspace" style={style}>
        <ModeRail />

        {mode === 'library' ? (
          <>
            {!navCollapsed && <Navigator {...navigatorProps} />}
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
                kinds={kindIndex}
                readOnly={readOnly}
                onJump={jumpTo}
                hasNodes={nodes.length > 0}
                loading={nodesQ.isPending}
                queue={queue}
              />
            </div>
            {/* Inspector returns the stack and its compiled prompt as siblings;
                this wrapper keeps both in the inspector grid track. */}
            {!inspCollapsed && (
              <div className="insp-region">
                <Inspector
                  project={project}
                  selected={selected}
                  kinds={kindIndex}
                  onJump={jumpTo}
                />
              </div>
            )}
          </>
        ) : mode === 'assets' ? (
          <AssetsMode nodes={nodes} kinds={kindIndex} readOnly={readOnly} onJump={jumpTo} />
        ) : mode === 'forge' ? (
          <ForgeMode
            project={project}
            nodes={nodes}
            selected={selected}
            kinds={kindIndex}
            queue={queue}
            onJump={jumpTo}
          />
        ) : (
          <Settings project={project} />
        )}
      </div>

      <StatusBar
        project={project}
        nodeCount={nodes.length}
        loading={nodesQ.isPending}
        peers={peers}
        sync={sync}
        backend={backend}
        queue={queue}
      />

      <CommandPalette
        nodes={nodes}
        kinds={kindIndex}
        onJump={jumpTo}
        onNewNode={openNewNode}
        readOnly={readOnly}
      />

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

      {transferSource && (
        <StyleTransferSheet
          sourcePath={transferSource}
          kinds={kindIndex}
          onClose={() => setTransferSource(null)}
          onImported={(id) => {
            setTransferSource(null)
            jumpTo(id)
          }}
        />
      )}
    </div>
  )
}
