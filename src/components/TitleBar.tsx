import { useRef, useState } from 'react'
import type { NodeSummary, ProjectSummary } from '../lib/api'
import { useCloseProject } from '../lib/queries'
import { labelFor, type KindIndex } from '../lib/kinds'
import { useUI } from '../store/ui'
import { reportProjectCloseFailure } from '../lib/projectClose'
import { modKey } from '../lib/platform'
import { Icon } from './Icon'
import { IconButton, Tooltip } from './Tooltip'
import { WindowControls } from './WindowControls'
import { ContextMenu, MenuItem, MenuLabel, MenuSeparator } from './ContextMenu'
import { SharingSheet } from './SharingSheet'

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
  const [sharing, setSharing] = useState(false)
  const closeProject = useCloseProject()
  const projectMenuButton = useRef<HTMLButtonElement>(null)

  const kindLabel = selected ? labelFor(kinds.get(selected.kind), selected.kind) : null
  const requestProjectClose = () => {
    closeProject.mutate(undefined, {
      onError: (error) => reportProjectCloseFailure(error, requestProjectClose),
    })
  }

  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="brand" data-tauri-drag-region>
        <span className="brand-mark" />
        wobu
      </div>

      <div style={{ position: 'relative' }}>
        <button
          ref={projectMenuButton}
          className="projpick"
          aria-haspopup="menu"
          aria-expanded={menu}
          onClick={() => setMenu((v) => !v)}
        >
          {project.name}
          <Icon name="chev" size="sm" />
        </button>
        {menu && (
          <ContextMenu
            className="title-project-menu"
            label={`Project actions for ${project.name}`}
            restoreFocusRef={projectMenuButton}
            onClose={() => setMenu(false)}
          >
            <MenuLabel>{project.path}</MenuLabel>
            <MenuSeparator />
            <MenuItem icon={<Icon name="share" size="sm" />} onSelect={() => setSharing(true)}>
              Share this project…
            </MenuItem>
            <MenuItem
              icon={<Icon name="folder" size="sm" />}
              disabledReason={
                closeProject.isPending ? 'This project is already being closed.' : null
              }
              onSelect={requestProjectClose}
            >
              Close project
            </MenuItem>
          </ContextMenu>
        )}
      </div>

      {project.readOnly && (
        <Tooltip
          tip="Wobu cannot write to this folder, so nothing here can be created, renamed or saved. Check the folder's permissions, or copy the project somewhere you can write to."
          placement="bottom"
        >
          <span className="ro-badge" tabIndex={0}>
            <Icon name="lock" size="sm" />
            read-only
          </span>
        </Tooltip>
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
          <span>nothing selected</span>
        )}
      </nav>

      <div className="tspace" data-tauri-drag-region />

      <button className="omni" onClick={() => setPaletteOpen(true)}>
        <Icon name="search" size="sm" />
        Jump to…
        <kbd>{modKey()}K</kbd>
      </button>
      <IconButton
        className="ibtn"
        label="Settings"
        placement="bottom"
        onClick={() => setMode('settings')}
      >
        <Icon name="settings" />
      </IconButton>

      <WindowControls />

      {sharing && <SharingSheet project={project} onClose={() => setSharing(false)} />}
    </header>
  )
}
