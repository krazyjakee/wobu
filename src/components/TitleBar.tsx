import { useEffect, useRef, useState } from 'react'
import type { NodeSummary, ProjectSummary } from '../lib/api'
import { useCloseProject } from '../lib/queries'
import { labelFor, type KindIndex } from '../lib/kinds'
import { useUI, report } from '../store/ui'
import { Icon } from './Icon'
import { WindowControls } from './WindowControls'

export function TitleBar({
  project,
  chain,
  selected,
  kinds,
}: {
  project: ProjectSummary
  chain: NodeSummary[]
  selected: NodeSummary | null
  kinds: KindIndex
}) {
  const setPaletteOpen = useUI((s) => s.setPaletteOpen)
  const select = useUI((s) => s.select)
  const setMode = useUI((s) => s.setMode)
  const [menu, setMenu] = useState(false)
  const closeProject = useCloseProject()
  const wrap = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!menu) return
    const off = (e: MouseEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setMenu(false)
    }
    window.addEventListener('mousedown', off)
    return () => window.removeEventListener('mousedown', off)
  }, [menu])

  const kindLabel = selected ? labelFor(kinds.get(selected.kind), selected.kind) : null

  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="brand" data-tauri-drag-region>
        <span className="brand-mark" />
        wobu
      </div>

      <div ref={wrap} style={{ position: 'relative' }}>
        <button className="projpick" onClick={() => setMenu((v) => !v)}>
          {project.name}
          <Icon name="chev" size="sm" />
        </button>
        {menu && (
          <div className="ctx" style={{ position: 'absolute', top: 30, left: 0 }}>
            <div className="ctx-label">{project.path}</div>
            <div className="ctx-sep" />
            <button
              onClick={() => {
                setMenu(false)
                closeProject.mutate(undefined, {
                  onError: (e) => report(e),
                })
              }}
            >
              <Icon name="folder" size="sm" />
              Close project
            </button>
          </div>
        )}
      </div>

      {project.readOnly && (
        <span className="ro-badge" title="This project folder is not writable">
          <Icon name="lock" size="sm" />
          read-only
        </span>
      )}

      <nav className="crumbs" data-tauri-drag-region>
        {selected ? (
          <>
            <span>{kindLabel}</span>
            {chain.map((n) => (
              <span key={n.id} style={{ display: 'contents' }}>
                <span>/</span>
                <button onClick={() => select(n.id)}>{n.name}</button>
              </span>
            ))}
            <span>/</span>
            <span className="cur">{selected.name}</span>
          </>
        ) : (
          <span>no node selected</span>
        )}
      </nav>

      <div className="tspace" data-tauri-drag-region />

      <button className="omni" onClick={() => setPaletteOpen(true)}>
        <Icon name="search" size="sm" />
        Jump to…
        <kbd>{modKey()}K</kbd>
      </button>
      <button className="ibtn" onClick={() => setMode('settings')} title="Settings">
        <Icon name="settings" />
      </button>

      <WindowControls />
    </header>
  )
}

export function modKey(): string {
  const mac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)
  return mac ? '⌘' : 'Ctrl+'
}
