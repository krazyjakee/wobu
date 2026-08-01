import { useEffect, useMemo, useRef, useState } from 'react'
import type { NodeSummary } from '../lib/api'
import { colorFor, labelFor, spriteFor, type KindIndex } from '../lib/kinds'
import { nameMatches, textMatches } from '../lib/search'
import { useNodeSearch, useUndo } from '../lib/queries'
import { useDebounced } from '../hooks/useDebounced'
import { useUI } from '../store/ui'
import { Icon } from './Icon'
import { Modal } from './Modal'

function NodeRow({
  node,
  kinds,
  on,
  onHover,
  onPick,
}: {
  node: NodeSummary
  kinds: KindIndex
  on: boolean
  onHover: () => void
  onPick: () => void
}) {
  const def = kinds.get(node.kind)
  return (
    <button className={on ? 'pal-row is-on' : 'pal-row'} onMouseEnter={onHover} onClick={onPick}>
      <Icon
        name={spriteFor(def, node.kind)}
        size="sm"
        style={{ color: colorFor(def, node.kind) }}
      />
      {node.name}
      <span className="sub">{node.summary || labelFor(def, node.kind)}</span>
    </button>
  )
}

interface Cmd {
  id: string
  label: string
  icon: string
  hint?: string
  run: () => void
}

export function CommandPalette({
  nodes,
  kinds,
  onJump,
  onNewNode,
  readOnly,
}: {
  nodes: NodeSummary[]
  kinds: KindIndex
  onJump: (id: string) => void
  onNewNode: () => void
  readOnly: boolean
}) {
  const open = useUI((s) => s.paletteOpen)
  const setOpen = useUI((s) => s.setPaletteOpen)
  const toggleNav = useUI((s) => s.toggleNav)
  const toggleInsp = useUI((s) => s.toggleInsp)

  const { undo, redo, nextUndo, nextRedo } = useUndo()

  const [q, setQ] = useState('')
  const [cursor, setCursor] = useState(0)
  const listRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (open) {
      setQ('')
      setCursor(0)
    }
  }, [open])

  /*
   * Undo and redo appear only when there is something to undo, and say what
   * that is. A palette row that is always present and sometimes does nothing
   * teaches the user to distrust the whole list, and "Undo" on its own asks
   * them to remember what they last did — which is precisely what someone
   * reaching for undo has already failed to do.
   *
   * On a read-only folder the three that write are absent rather than disabled,
   * by the same rule: the banner has already said why, and a greyed row in a
   * list you are typing into is a second explanation nobody asked for.
   */
  const commands = useMemo<Cmd[]>(
    () => [
      ...(readOnly
        ? []
        : [
            {
              id: 'cmd:new',
              label: 'New node…',
              icon: 'plus',
              hint: 'create',
              run: () => onNewNode(),
            },
          ]),
      ...(!readOnly && nextUndo
        ? [
            {
              id: 'cmd:undo',
              label: `Undo ${nextUndo.label}`,
              icon: 'refresh',
              hint: '⌘Z',
              run: () => void undo(),
            },
          ]
        : []),
      ...(!readOnly && nextRedo
        ? [
            {
              id: 'cmd:redo',
              label: `Redo ${nextRedo.label}`,
              icon: 'refresh',
              hint: '⇧⌘Z',
              run: () => void redo(),
            },
          ]
        : []),
      {
        id: 'cmd:nav',
        label: 'Toggle navigator',
        icon: 'library',
        hint: '[',
        run: () => toggleNav(),
      },
      {
        id: 'cmd:insp',
        label: 'Toggle inspector',
        icon: 'layers',
        hint: ']',
        run: () => toggleInsp(),
      },
    ],
    [onNewNode, readOnly, toggleNav, toggleInsp, undo, redo, nextUndo, nextRedo],
  )

  const needle = q.trim().toLowerCase()

  /*
   * Two searches, deliberately.
   *
   * The local one runs on every keystroke over the already-loaded summaries, so
   * typing a node's name is instant and never waits on a round trip. The FTS
   * one is debounced and reaches into notes and descriptions, which are not in
   * memory and cannot be searched here at all.
   *
   * They are shown as separate groups rather than merged into one ranked list.
   * A node whose name does not contain what you typed, sitting among ones that
   * do, reads as a bug — the heading is what explains it, and it is honest
   * about which half of the search found the row.
   */
  const matchedNodes = useMemo(() => nameMatches(nodes, q), [nodes, q])

  const search = useNodeSearch(useDebounced(q, 140))

  const byId = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes])

  /** FTS hits that the name filter did not already show, in rank order. */
  const matchedText = useMemo(
    () => (needle ? textMatches(byId, search.data ?? [], matchedNodes) : []),
    [search.data, matchedNodes, byId, needle],
  )

  const matchedCmds = useMemo(
    () => (needle ? commands.filter((c) => c.label.toLowerCase().includes(needle)) : commands),
    [commands, needle],
  )

  const rows = useMemo(
    () => [
      ...matchedNodes.map((n) => ({ kind: 'node' as const, node: n })),
      ...matchedText.map((n) => ({ kind: 'node' as const, node: n })),
      ...matchedCmds.map((c) => ({ kind: 'cmd' as const, cmd: c })),
    ],
    [matchedNodes, matchedText, matchedCmds],
  )

  useEffect(() => {
    setCursor((c) => (rows.length === 0 ? 0 : Math.min(c, rows.length - 1)))
  }, [rows.length])

  useEffect(() => {
    listRef.current?.querySelector('.is-on')?.scrollIntoView({ block: 'nearest' })
  }, [cursor])

  if (!open) return null

  const pick = (i: number) => {
    const row = rows[i]
    if (!row) return
    setOpen(false)
    if (row.kind === 'node') onJump(row.node.id)
    else row.cmd.run()
  }

  return (
    <Modal
      className="pal"
      scrimClassName="scrim-top"
      titleId="command-palette-title"
      descriptionId="command-palette-description"
      onClose={() => setOpen(false)}
    >
      <h2 className="modal-sr-only" id="command-palette-title">
        Command palette
      </h2>
      <p className="modal-sr-only" id="command-palette-description">
        Search for a node or choose a workspace command.
      </p>
      <div className="pal-in">
        <Icon name="search" />
        <input
          data-modal-initial-focus
          spellCheck={false}
          value={q}
          placeholder="Jump to a node, or type a command…"
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') {
              e.preventDefault()
              setCursor((c) => (rows.length ? (c + 1) % rows.length : 0))
            } else if (e.key === 'ArrowUp') {
              e.preventDefault()
              setCursor((c) => (rows.length ? (c - 1 + rows.length) % rows.length : 0))
            } else if (e.key === 'Enter') {
              e.preventDefault()
              pick(cursor)
            }
          }}
        />
        <kbd>esc</kbd>
      </div>

      <div className="pal-list" ref={listRef}>
        {rows.length === 0 && (
          <div className="pal-none">
            {nodes.length === 0
              ? 'This world has no nodes yet.'
              : search.isFetching
                ? 'Searching notes…'
                : 'No match.'}
          </div>
        )}

        {matchedNodes.length > 0 && <div className="pal-sec">Nodes</div>}
        {matchedNodes.map((n, i) => (
          <NodeRow
            key={n.id}
            node={n}
            kinds={kinds}
            on={cursor === i}
            onHover={() => setCursor(i)}
            onPick={() => pick(i)}
          />
        ))}

        {/* Named rather than merged above: these matched something the user
              cannot see on the row, so the heading is the explanation. */}
        {matchedText.length > 0 && <div className="pal-sec">In notes and descriptions</div>}
        {matchedText.map((n, j) => {
          const i = matchedNodes.length + j
          return (
            <NodeRow
              key={n.id}
              node={n}
              kinds={kinds}
              on={cursor === i}
              onHover={() => setCursor(i)}
              onPick={() => pick(i)}
            />
          )
        })}

        {matchedCmds.length > 0 && <div className="pal-sec">Commands</div>}
        {matchedCmds.map((c, j) => {
          const i = matchedNodes.length + matchedText.length + j
          return (
            <button
              key={c.id}
              className={cursor === i ? 'pal-row is-on' : 'pal-row'}
              onMouseEnter={() => setCursor(i)}
              onClick={() => pick(i)}
            >
              <Icon name={c.icon} size="sm" />
              {c.label}
              {c.hint && <span className="sub">{c.hint}</span>}
            </button>
          )
        })}
      </div>

      <div className="pal-foot">
        <kbd>↑↓</kbd> navigate <kbd>↵</kbd> open <kbd>esc</kbd> dismiss
      </div>
    </Modal>
  )
}
