import { useEffect, useRef, useState } from 'react'
import type { NodeSummary } from '../../lib/api'
import { errorMessage } from '../../lib/api'
import { useNode } from '../../lib/queries'
import { colorFor, labelFor, spriteFor, type KindIndex } from '../../lib/kinds'
import { useUI, EDITOR_TABS, type EditorTab } from '../../store/ui'
import { Icon } from '../Icon'
import { modKey } from '../TitleBar'
import { NotesPane } from './NotesPane'
import { TabEmpty } from './TabEmpty'
import { useAutosaveNode, saveLabel } from '../../hooks/useAutosaveNode'

const TAB_LABEL: Record<EditorTab, string> = {
  notes: 'Notes',
  refs: 'References',
  concepts: 'Concepts',
  three: '3D',
  relations: 'Relations',
}

export function Editor({
  selected,
  chain,
  kinds,
  readOnly,
  onJump,
  hasNodes,
  loading,
}: {
  selected: NodeSummary | null
  chain: NodeSummary[]
  kinds: KindIndex
  readOnly: boolean
  onJump: (id: string) => void
  hasNodes: boolean
  loading: boolean
}) {
  const tab = useUI((s) => s.tab)
  const setTab = useUI((s) => s.setTab)
  const nodeQ = useNode(selected?.id ?? null)
  const node = nodeQ.data
  const autosave = useAutosaveNode(node, { readOnly })

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
            {chain.length === 0 ? (
              <span>nothing yet — links land with the Influence Engine in M5</span>
            ) : (
              chain.map((n) => {
                const c = colorFor(kinds.get(n.kind), n.kind)
                return (
                  <button
                    key={n.id}
                    className="chip"
                    style={{ color: c }}
                    onClick={() => onJump(n.id)}
                    title={n.summary || n.name}
                  >
                    <i />
                    <span style={{ color: 'var(--text-dim)' }}>{n.name}</span>
                  </button>
                )
              })
            )}
          </span>
        </div>

        <div className="ed-actions">
          <span className="col-tag-save">{saveLabel(autosave.status)}</span>
          {/* Enhance is M4 and Generate is M5, and neither is wired up — the
              button is disabled for that reason first. Both write to the node
              like any other edit (docs/07-file-shares.md), so when they land
              they stay disabled on a read-only folder: keep `readOnly` in the
              condition below and add Generate's button beside this one with the
              same test. The banner is already on screen and explains it, so
              nothing here needs its own sentence about the share. */}
          <button
            className="btn btn-ai"
            disabled
            title={
              readOnly
                ? 'This share is read-only, and Enhance writes to the node'
                : 'Enhance arrives in M4 — no LLM provider is wired up yet'
            }
          >
            <Icon name="spark" size="sm" />
            Enhance
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
              <NotesPane node={node} def={def} readOnly={readOnly} autosave={autosave} />
            )}
            {tab === 'refs' && (
              <TabEmpty
                icon="image"
                title="References"
                milestone="M3 — References"
                body="Image import, content-addressed storage, thumbnails, and per-image role and weight. Until that exists there is nothing here — and nothing pretending to be here."
              />
            )}
            {tab === 'concepts' && (
              <TabEmpty
                icon="spark"
                title="Concepts"
                milestone="M5 — Influence Engine + first images"
                body="Generated art for this entity, with its prompt, seed and influence snapshot. It needs the compiler and an image backend, both of which arrive together."
              />
            )}
            {tab === 'three' && (
              <TabEmpty
                icon="cube"
                title="3D"
                milestone="M7 — Concept 3D"
                body="Turnaround sheets, image-to-mesh, and an inline three.js viewer. Deliberately last: it depends on consistent multi-view output."
              />
            )}
            {tab === 'relations' && (
              <TabEmpty
                icon="link"
                title="Relations"
                milestone="M5 — Influence Engine + first images"
                body="Outgoing links and backlinks — “3 characters inherit from this”. Link editing lands with the engine that walks them."
              />
            )}
          </div>
        )}
      </div>
    </main>
  )
}

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
