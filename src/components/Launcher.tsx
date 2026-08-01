import { useRef, useState } from 'react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { errorMessage, projectOpenCancel, type ProjectSummary, type ScanProgress } from '../lib/api'
import {
  useCreateProject,
  useForgetRecentProject,
  useOpenProject,
  useRecentProjects,
} from '../lib/queries'
import { useOpenProgress } from '../hooks/useOpenProgress'
import { report } from '../store/ui'
import { Icon } from './Icon'
import { Modal } from './Modal'
import { WindowControls } from './WindowControls'
import { ContextMenu } from './navigator/ContextMenu'

/**
 * What a slow open looks like.
 *
 * Rendered only while an open is in flight. The important case is not a large
 * world — it is a *stalled* one: an unresponsive NAS blocks a single read for
 * the mount's own timeout, minutes by default, and from outside that is
 * indistinguishable from a frozen app. A count that stops advancing tells the
 * user which it is, and Cancel gives them a way out that is not `kill`.
 */
function Scanning() {
  const progress = useOpenProgress(true)
  const [cancelling, setCancelling] = useState(false)

  async function cancel() {
    setCancelling(true)
    try {
      await projectOpenCancel()
    } catch (e) {
      report(e, 'Could not stop the scan')
      setCancelling(false)
    }
  }

  // No events yet: either the world is small enough that the scan is already
  // finishing, or the folder has not answered at all. Saying "reading" rather
  // than showing 0% avoids implying progress that has not been measured.
  const label = progress
    ? `Reading ${progress.done.toLocaleString()} of ${progress.total.toLocaleString()} files`
    : 'Reading the folder…'

  return (
    <div className="lch-scan" role="status" aria-live="polite">
      <div className="lch-scan-row">
        <span>{label}</span>
        <button className="btn-mini" onClick={() => void cancel()} disabled={cancelling}>
          {cancelling ? 'Stopping…' : 'Cancel'}
        </button>
      </div>
      <div className="lch-bar-track">
        <div
          className={progress ? 'lch-bar-fill' : 'lch-bar-fill is-indeterminate'}
          style={progress ? { width: `${pct(progress)}%` } : undefined}
        />
      </div>
      <p className="lch-scan-note">
        Only the first open reads every file. After this, Wobu re-reads just the ones that changed.
      </p>
    </div>
  )
}

function pct(p: ScanProgress): number {
  if (p.total === 0) return 100
  return Math.min(100, Math.round((p.done / p.total) * 100))
}

export function Launcher({ error }: { error: string | null }) {
  const recent = useRecentProjects()
  const openProject = useOpenProject()
  const forgetRecent = useForgetRecentProject()
  const [newOpen, setNewOpen] = useState(false)
  const [openFailure, setOpenFailure] = useState<{ id: string; message: string } | null>(null)

  const busy = openProject.isPending

  async function pickFolder() {
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: 'Open a Wobu project folder',
      })
      if (typeof picked !== 'string') return
      openProject.mutate(picked, {
        onError: (e) => report(e),
      })
    } catch (e) {
      report(e)
    }
  }

  function openRecent(project: ProjectSummary) {
    setOpenFailure(null)
    openProject.mutate(project.path, {
      onError: (reason) => {
        report(reason)
        setOpenFailure({ id: project.id, message: errorMessage(reason) })
      },
    })
  }

  function removeRecent(project: ProjectSummary) {
    forgetRecent.mutate(project.id, {
      onSuccess: () => setOpenFailure((failure) => (failure?.id === project.id ? null : failure)),
      onError: (reason) => report(reason, `Could not remove ${project.name} from Recent`),
    })
  }

  return (
    <div className="launcher">
      <div className="lch-bar" data-tauri-drag-region>
        <div className="brand" data-tauri-drag-region>
          <span className="brand-mark" />
          wobu
        </div>
        <div className="tspace" data-tauri-drag-region />
        <WindowControls />
      </div>

      <div className="lch-body">
        <div className="lch-inner">
          <div className="lch-head">
            <span className="brand-mark brand-lg" />
            <div>
              <h1>wobu</h1>
              <p>Author the hierarchy once. Every generation inherits it.</p>
            </div>
            <div className="lch-actions">
              <button className="btn" onClick={pickFolder} disabled={busy}>
                <Icon name="folder" size="sm" />
                Open folder…
              </button>
              <button className="btn btn-primary" onClick={() => setNewOpen(true)} disabled={busy}>
                <Icon name="plus" size="sm" />
                New project
              </button>
            </div>
          </div>

          <h2 className="lch-h">Recent</h2>
          <RecentGrid
            recent={recent.data}
            pending={recent.isPending}
            failed={recent.isError ? errorMessage(recent.error) : null}
            busy={busy}
            openFailure={openFailure}
            removingId={forgetRecent.isPending ? forgetRecent.variables : null}
            onOpen={openRecent}
            onRemove={removeRecent}
          />

          {busy && <Scanning />}

          <p className="lch-note">
            A project is a folder, not a database file. Point Wobu at a share and anyone who can see
            the path can open it — their keys, their machine, the same world.
          </p>

          {error && (
            <div className="lch-err">
              Could not read the current project: <code>{error}</code>
            </div>
          )}
        </div>
      </div>

      {newOpen && <NewProjectSheet onClose={() => setNewOpen(false)} />}
    </div>
  )
}

function RecentGrid({
  recent,
  pending,
  failed,
  busy,
  openFailure,
  removingId,
  onOpen,
  onRemove,
}: {
  recent: ProjectSummary[] | undefined
  pending: boolean
  failed: string | null
  busy: boolean
  openFailure: { id: string; message: string } | null
  removingId: string | null
  onOpen: (p: ProjectSummary) => void
  onRemove: (p: ProjectSummary) => void
}) {
  if (failed) {
    return <div className="lch-empty">Recent projects could not be read — {failed}</div>
  }
  if (pending) return <div className="lch-empty">Reading recent projects…</div>
  if (!recent || recent.length === 0) {
    return (
      <div className="lch-empty">
        No projects opened on this machine yet. <b>New project</b> writes a fresh folder;{' '}
        <b>Open folder…</b> points Wobu at one that already exists — including one on a share.
      </div>
    )
  }
  return (
    <div className="lch-grid">
      {recent.map((p) => (
        <RecentCard
          key={p.id}
          project={p}
          busy={busy}
          removing={removingId === p.id}
          failure={openFailure?.id === p.id ? openFailure.message : null}
          onOpen={() => onOpen(p)}
          onRemove={() => onRemove(p)}
        />
      ))}
    </div>
  )
}

function RecentCard({
  project,
  busy,
  removing,
  failure,
  onOpen,
  onRemove,
}: {
  project: ProjectSummary
  busy: boolean
  removing: boolean
  failure: string | null
  onOpen: () => void
  onRemove: () => void
}) {
  const [menu, setMenu] = useState(false)
  const moreButton = useRef<HTMLButtonElement>(null)
  const menuHelpId = `recent-project-menu-help-${project.id}`

  return (
    <div className="lch-card">
      <button className="lch-card-open" onClick={onOpen} disabled={busy || removing}>
        <span className="nm">
          {project.name}
          {project.readOnly && <span className="tag">read-only</span>}
          {!project.readOnly && project.onNetworkShare && <span className="tag">share</span>}
        </span>
        <span className="pth">{project.path}</span>
        <span className="when">{lastOpened(project.lastOpenedAt)}</span>
      </button>
      <button
        ref={moreButton}
        className="lch-card-more"
        aria-label={`More actions for ${project.name}`}
        aria-haspopup="menu"
        aria-expanded={menu}
        onClick={() => setMenu((value) => !value)}
        disabled={busy || removing}
      >
        ⋯
      </button>
      {menu && (
        <ContextMenu
          className="lch-card-menu"
          label={`Actions for ${project.name}`}
          restoreFocusRef={moreButton}
          onClose={() => setMenu(false)}
        >
          <button
            role="menuitem"
            aria-describedby={menuHelpId}
            onClick={() => {
              setMenu(false)
              onRemove()
            }}
          >
            Remove from Recent
          </button>
          <p id={menuHelpId} role="presentation">
            Removes this launcher entry only. Project files stay on disk.
          </p>
        </ContextMenu>
      )}
      {failure && (
        <div className="lch-open-failure" role="alert">
          <p>Could not open this project: {failure}</p>
          <div>
            <button className="btn-mini" onClick={onOpen} disabled={busy || removing}>
              Retry
            </button>
            <button className="btn-mini" onClick={onRemove} disabled={busy || removing}>
              {removing ? 'Removing…' : 'Remove from Recent'}
            </button>
          </div>
          <small>Removing changes this launcher list only. Project files stay on disk.</small>
        </div>
      )}
    </div>
  )
}

function lastOpened(iso: string | null): string {
  if (!iso) return 'never opened'
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return iso
  const mins = Math.round((Date.now() - t) / 60000)
  if (mins < 1) return 'just now'
  if (mins < 60) return `${mins} min ago`
  const hrs = Math.round(mins / 60)
  if (hrs < 24) return `${hrs} h ago`
  return new Date(t).toLocaleDateString()
}

function NewProjectSheet({ onClose }: { onClose: () => void }) {
  const createProject = useCreateProject()
  const [parentDir, setParentDir] = useState('')
  const [name, setName] = useState('')
  const [err, setErr] = useState<string | null>(null)

  async function pickParent() {
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: 'Where should the project folder go?',
      })
      if (typeof picked === 'string') setParentDir(picked)
    } catch (e) {
      setErr(errorMessage(e))
    }
  }

  function submit() {
    if (!parentDir.trim() || !name.trim()) {
      setErr('Pick a location and give the project a name.')
      return
    }
    setErr(null)
    createProject.mutate(
      { parentDir: parentDir.trim(), name: name.trim() },
      { onError: (e) => setErr(errorMessage(e)), onSuccess: onClose },
    )
  }

  return (
    <Modal
      titleId="new-project-title"
      descriptionId="new-project-description"
      onClose={onClose}
      busy={createProject.isPending}
      busyMessage={
        createProject.isPending
          ? 'Creating the project folder. This operation cannot be interrupted.'
          : undefined
      }
    >
      <h2 id="new-project-title">New project</h2>
      <p id="new-project-description">
        Wobu creates a self-contained folder — Markdown nodes, assets and generation history all
        inside it. Put it wherever it belongs, including a network share.
      </p>

      <div className="field">
        <label htmlFor="np-loc">Location</label>
        <div className="picker">
          <input
            id="np-loc"
            value={parentDir}
            placeholder="choose a parent folder…"
            onChange={(e) => setParentDir(e.target.value)}
          />
          <button className="btn" onClick={pickParent}>
            <Icon name="folder" size="sm" />
            Browse
          </button>
        </div>
      </div>

      <div className="field">
        <label htmlFor="np-name">Project name</label>
        <input
          id="np-name"
          value={name}
          placeholder="Ashfall"
          data-modal-initial-focus
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') submit()
          }}
        />
      </div>

      {err && <div className="sheet-err">{err}</div>}

      <div className="sheet-actions">
        <button className="btn btn-ghost" onClick={onClose} disabled={createProject.isPending}>
          Cancel
        </button>
        <button className="btn btn-primary" onClick={submit} disabled={createProject.isPending}>
          {createProject.isPending ? 'Creating…' : 'Create project'}
        </button>
      </div>
    </Modal>
  )
}
