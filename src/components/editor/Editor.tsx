import { useEffect, useRef, useState } from 'react'
import type { NodeSummary, QueueSnapshot } from '../../lib/api'
import { errorMessage } from '../../lib/api'
import { useInfluenceStack, useNode } from '../../lib/queries'
import { colorFor, labelFor, spriteFor, type KindIndex } from '../../lib/kinds'
import { useUI, EDITOR_TABS, type EditorTab } from '../../store/ui'
import { Icon } from '../Icon'
import { TipButton, Tooltip } from '../Tooltip'
import { modKey } from '../../lib/platform'
import { NotesPane } from './NotesPane'
import { RelationsPane } from './RelationsPane'
import { ConceptsPane } from './ConceptsPane'
import { ReferencesPane } from './ReferencesPane'
import { ThreePane } from './ThreePane'
import { useAutosaveNode, saveLabel } from '../../hooks/useAutosaveNode'
import { useActionShortcut } from '../../hooks/useKeyboard'
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
  const enhanceDisabled = !node || enhance.starting || (readOnly && !enhance.active)
  const triggerEnhance = () => {
    if (enhanceDisabled) return
    setTab('notes')
    if (!enhance.active) enhance.start()
  }
  useActionShortcut('enhance', !enhanceDisabled, triggerEnhance)
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
          <h3>{loading ? 'Reading the world…' : hasNodes ? 'Nothing selected' : 'Empty world'}</h3>
          <p>
            {hasNodes ? (
              <>
                Pick something in the navigator, or press <kbd>{modKey()}K</kbd> to jump.
              </>
            ) : (
              <>
                Nothing has been written yet. <b>New entity</b> in the navigator writes the first
                one. Every entity is a Markdown file in the project folder, and that file is the
                real thing — Wobu keeps no separate copy of it.
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
            <Tooltip tip="The notes, or something this inherits from, changed since the last enhance — so the description below is older than what it was written from.">
              <span className="badge" tabIndex={0}>
                stale
              </span>
            </Tooltip>
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
                  <TipButton
                    key={layer.nodeId}
                    className="chip"
                    style={{ color: c }}
                    onClick={() => onJump(layer.nodeId as string)}
                    tip={`Open ${layer.name}`}
                    placement="bottom"
                  >
                    <i />
                    <span style={{ color: 'var(--text-dim)' }}>{layer.name}</span>
                  </TipButton>
                )
              })
            )}
          </span>
        </div>

        <div className="ed-actions">
          <span className="col-tag-save">{saveLabel(autosave.status)}</span>
          {/* The read-only case is the one worth spelling out: Enhance is the
              primary action of this pane, and "greyed out" on a share is the
              question #129 was filed about. */}
          <TipButton
            className="btn btn-ai"
            disabledReason={
              !enhanceDisabled
                ? null
                : readOnly && !enhance.active
                  ? 'This project folder is read-only, and Enhance writes into it. Open a copy you can write to.'
                  : enhance.starting
                    ? 'It is already starting.'
                    : 'Open an entity first — Enhance rewrites the description of whatever you are looking at.'
            }
            tip={
              enhance.complete
                ? 'Review the finished description before anything is written'
                : enhance.stopped
                  ? 'Review the stopped local draft'
                  : 'Turn your notes, and everything this inherits, into a description you review before it is saved'
            }
            onClick={triggerEnhance}
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
          </TipButton>
        </div>
      </div>

      <div className="tabs">
        {EDITOR_TABS.map((t, i) => (
          <TipButton
            key={t}
            className={tab === t ? 'tab is-active' : 'tab'}
            aria-pressed={tab === t}
            onClick={() => setTab(t)}
            tip={`${TAB_LABEL[t]} · ${modKey()}${i + 1}`}
            placement="bottom"
          >
            {TAB_LABEL[t]}
          </TipButton>
        ))}
      </div>

      <div className="panes">
        {nodeQ.isPending && <div className="empty">Opening…</div>}
        {nodeQ.isError && (
          <div className="empty">
            <h3>Could not open this entity</h3>
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
            {tab === 'three' && <ThreePane node={node} queue={queue} readOnly={readOnly} />}
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
      aria-label="Entity name"
    />
  )
}
