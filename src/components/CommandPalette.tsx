import { useEffect, useMemo, useRef, useState } from 'react'
import type { NodeSummary } from '../lib/api'
import { colorFor, labelFor, spriteFor, type KindIndex } from '../lib/kinds'
import { useUI } from '../store/ui'
import { Icon } from './Icon'

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
}: {
  nodes: NodeSummary[]
  kinds: KindIndex
  onJump: (id: string) => void
  onNewNode: () => void
}) {
  const open = useUI((s) => s.paletteOpen)
  const setOpen = useUI((s) => s.setPaletteOpen)
  const toggleNav = useUI((s) => s.toggleNav)
  const toggleInsp = useUI((s) => s.toggleInsp)

  const [q, setQ] = useState('')
  const [cursor, setCursor] = useState(0)
  const listRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (open) {
      setQ('')
      setCursor(0)
    }
  }, [open])

  const commands = useMemo<Cmd[]>(
    () => [
      {
        id: 'cmd:new',
        label: 'New node…',
        icon: 'plus',
        hint: 'create',
        run: () => onNewNode(),
      },
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
    [onNewNode, toggleNav, toggleInsp],
  )

  const needle = q.trim().toLowerCase()
  const matchedNodes = useMemo(() => {
    const list = needle
      ? nodes.filter(
          (n) =>
            n.name.toLowerCase().includes(needle) || n.summary.toLowerCase().includes(needle),
        )
      : nodes
    return [...list]
      .sort((a, b) => {
        if (needle) {
          const ai = a.name.toLowerCase().indexOf(needle)
          const bi = b.name.toLowerCase().indexOf(needle)
          if (ai !== bi) return (ai < 0 ? 99 : ai) - (bi < 0 ? 99 : bi)
        }
        return a.name.localeCompare(b.name)
      })
      .slice(0, 40)
  }, [nodes, needle])

  const matchedCmds = useMemo(
    () => (needle ? commands.filter((c) => c.label.toLowerCase().includes(needle)) : commands),
    [commands, needle],
  )

  const rows = useMemo(
    () => [
      ...matchedNodes.map((n) => ({ kind: 'node' as const, node: n })),
      ...matchedCmds.map((c) => ({ kind: 'cmd' as const, cmd: c })),
    ],
    [matchedNodes, matchedCmds],
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
    <div
      className="scrim scrim-top"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) setOpen(false)
      }}
    >
      <div className="pal" role="dialog" aria-label="Command palette">
        <div className="pal-in">
          <Icon name="search" />
          <input
            autoFocus
            spellCheck={false}
            value={q}
            placeholder="Jump to a node, or type a command…"
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                e.preventDefault()
                setOpen(false)
              } else if (e.key === 'ArrowDown') {
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
              {nodes.length === 0 ? 'This world has no nodes yet.' : 'No match.'}
            </div>
          )}

          {matchedNodes.length > 0 && <div className="pal-sec">Nodes</div>}
          {matchedNodes.map((n, i) => {
            const def = kinds.get(n.kind)
            return (
              <button
                key={n.id}
                className={cursor === i ? 'pal-row is-on' : 'pal-row'}
                onMouseEnter={() => setCursor(i)}
                onClick={() => pick(i)}
              >
                <Icon
                  name={spriteFor(def, n.kind)}
                  size="sm"
                  style={{ color: colorFor(def, n.kind) }}
                />
                {n.name}
                <span className="sub">{n.summary || labelFor(def, n.kind)}</span>
              </button>
            )
          })}

          {matchedCmds.length > 0 && <div className="pal-sec">Commands</div>}
          {matchedCmds.map((c, j) => {
            const i = matchedNodes.length + j
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
      </div>
    </div>
  )
}
