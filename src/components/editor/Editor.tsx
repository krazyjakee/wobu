import { useEffect, useRef, useState } from 'react'
import type { NodeSummary, QueueSnapshot } from '../../lib/api'
import { errorMessage } from '../../lib/api'
import { useInfluenceStack, useNode } from '../../lib/queries'
import { colorFor, labelFor, spriteFor, type KindIndex } from '../../lib/kinds'
import { useUI, EDITOR_TABS, type EditorTab } from '../../store/ui'
import { Icon } from '../Icon'
import { modKey } from '../TitleBar'
import { NotesPane } from './NotesPane'
import { RelationsPane } from './RelationsPane'
import { ConceptsPane } from './ConceptsPane'
import { ReferencesPane } from './ReferencesPane'
import { ThreePane } from './ThreePane'
import { useAutosaveNode, saveLabel } from '../../hooks/useAutosaveNode'
import { useEnhanceSession } from './useEnhanceSession'

const TAB_LABEL: Record<EditorTab, string> = {
  notes: 'Notes',
  refs: 'References',
  concepts: 'Concepts',
  three: '3D',
  relations: 'Relations',
}

export function Editor({
  selected,
  kinds,
  readOnly,
  onJump,
  hasNodes,
  loading,
  queue = EMPTY_QUEUE,
}: {
  selected: NodeSummary | null
  kinds: KindIndex
  readOnly: boolean
  onJump: (id: string) => void
  hasNodes: boolean
  loading: boolean
  queue?: QueueSnapshot
}) {
  const tab = useUI((s) => s.tab)
  const setTab = useUI((s) => s.setTab)
  const nodeQ = useNode(selected?.id ?? null)
  const influenceQ = useInfluenceStack(selected?.id ?? null)
  const node = nodeQ.data
  const autosave = useAutosaveNode(node, { readOnly })
  const enhance = useEnhanceSession(node?.id ?? selected?.id ?? null, queue)
  // The two project roots influence everything, but do not locate an entity in
  // the world's hierarchy. The breadcrumb is the ancestry/culture/place spine
  // of the resolved stack; using the resolver (rather than reading only the
  // subject's direct links) also includes implicit parent chains in the order
  // the compiler actually sees them.
  const influenceChain =
    influenceQ.data?.layers.filter(
      (layer) =>
        layer.nodeId !== null &&
        (layer.layer === 'ancestry' || layer.layer === 'culture' || layer.layer === 'place'),
    ) ?? []

  if (!selected) {
    return (
      <main className="editor">
        <div className="empty" style={{ margin: 'auto' }}>
          <Icon name="library" size="xl" />
          <h3>{loading ? 'Reading the world…' : hasNodes ? 'No node selected' : 'Empty world'}</h3>
          <p>
            {hasNodes ? (
              <>
                Pick something in the navigator, or press <kbd>{modKey()}K</kbd> to jump.
              </>
            ) : (
              <>
                Nothing has been written yet. <b>New entity</b> in the navigator creates the first
                Markdown file — that file is the source of truth, not a database row.
              </>
            )}
          </p>
        </div>
      </main>
    )
  }

  const def = kinds.get(selected.kind)
  const color = colorFor(def, selected.kind)

  return (
    <main className="editor">
      <div className="ed-head">
        <div className="ed-title">
          <span
            className="ed-kind"
            style={{ color, background: `color-mix(in srgb, ${color} 15%, transparent)` }}
          >
            <Icon name={spriteFor(def, selected.kind)} />
          </span>
          <NameField
            key={selected.id}
            name={node?.name ?? selected.name}
            readOnly={readOnly || !node}
            onCommit={(name) => {
              if (node && name && name !== node.name) autosave.queue({ name })
            }}
          />
          <span className="badge">{labelFor(def, selected.kind)}</span>
          {selected.descriptionState === 'stale' && (
            <span className="badge" title="notes changed since the last enhance">
              stale
            </span>
          )}
        </div>

        <div className="ed-sub">
          <span className="ed-inherit">inherits</span>
          <span className="chain">
            {influenceQ.isPending ? null : influenceChain.length === 0 ? (
              <span>nothing</span>
            ) : (
              influenceChain.map((layer) => {
                const kind = layer.kind
                const c = kind ? colorFor(kinds.get(kind), kind) : 'var(--text-faint)'
                return (
                  <button
                    key={layer.nodeId}
                    className="chip"
                    style={{ color: c }}
                    onClick={() => onJump(layer.nodeId as string)}
                    title={`Open ${layer.name}`}
                  >
                    <i />
                    <span style={{ color: 'var(--text-dim)' }}>{layer.name}</span>
                  </button>
                )
              })
            )}
          </span>
        </div>

        <div className="ed-actions">
          <span className="col-tag-save">{saveLabel(autosave.status)}</span>
          <button
            className="btn btn-ai"
            disabled={!node || enhance.starting || (readOnly && !enhance.active)}
            title={
              readOnly
                ? 'This share is read-only, and Enhance writes to the node'
                : enhance.complete
                  ? 'Review the finished description before anything is written'
                  : enhance.stopped
                    ? 'Review the stopped local draft'
                    : 'Turn notes and influences into reviewed canonical sections'
            }
            onClick={() => {
              setTab('notes')
              if (!enhance.active) enhance.start()
            }}
          >
            <Icon name="spark" size="sm" />
            {enhance.starting
              ? 'Starting…'
              : enhance.running
                ? 'Enhancing…'
                : enhance.complete
                  ? 'Review Enhance'
                  : enhance.stopped || enhance.failure || enhance.candidate
                    ? 'Review draft'
                    : 'Enhance'}
          </button>
        </div>
      </div>

      <div className="tabs">
        {EDITOR_TABS.map((t, i) => (
          <button
            key={t}
            className={tab === t ? 'tab is-active' : 'tab'}
            onClick={() => setTab(t)}
            title={`${modKey()}${i + 1}`}
          >
            {TAB_LABEL[t]}
          </button>
        ))}
      </div>

      <div className="panes">
        {nodeQ.isPending && <div className="empty">Loading node…</div>}
        {nodeQ.isError && (
          <div className="empty">
            <h3>Could not open this node</h3>
            <p>{errorMessage(nodeQ.error)}</p>
          </div>
        )}
        {node && (
          <div className="pane">
            {tab === 'notes' && (
              <NotesPane
                node={node}
                def={def}
                readOnly={readOnly}
                autosave={autosave}
                enhance={enhance}
              />
            )}
            {tab === 'refs' && (
              <ReferencesPane key={node.id} node={node} readOnly={readOnly} autosave={autosave} />
            )}
            {tab === 'concepts' && (
              <ConceptsPane node={node} queue={queue} kinds={kinds} readOnly={readOnly} />
            )}
            {tab === 'three' && <ThreePane node={node} />}
            {tab === 'relations' && (
              <RelationsPane
                node={node}
                def={def}
                kinds={kinds}
                readOnly={readOnly}
                onJump={onJump}
              />
            )}
          </div>
        )}
      </div>
    </main>
  )
}

const EMPTY_QUEUE: QueueSnapshot = { jobs: [], queued: 0, running: 0, retrying: 0 }

function NameField({
  name,
  readOnly,
  onCommit,
}: {
  name: string
  readOnly: boolean
  onCommit: (name: string) => void
}) {
  const [value, setValue] = useState(name)
  const editing = useRef(false)
  useEffect(() => {
    if (!editing.current) setValue(name)
  }, [name])

  return (
    <input
      className="ed-name"
      value={value}
      readOnly={readOnly}
      spellCheck={false}
      size={Math.max(6, value.length)}
      onFocus={() => (editing.current = true)}
      onChange={(e) => setValue(e.target.value)}
      onBlur={() => {
        editing.current = false
        const v = value.trim()
        if (v) onCommit(v)
        else setValue(name)
      }}
      onKeyDown={(e) => {
        if (e.key === 'Enter') e.currentTarget.blur()
        if (e.key === 'Escape') {
          setValue(name)
          editing.current = false
          e.currentTarget.blur()
        }
      }}
      aria-label="Node name"
    />
  )
}
