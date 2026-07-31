import { useState } from 'react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { errorMessage, type ProjectSummary } from '../lib/api'
import { useCreateProject, useOpenProject, useRecentProjects } from '../lib/queries'
import { toast } from '../store/ui'
import { Icon } from './Icon'
import { WindowControls } from './WindowControls'

export function Launcher({ error }: { error: string | null }) {
  const recent = useRecentProjects()
  const openProject = useOpenProject()
  const [newOpen, setNewOpen] = useState(false)

  const busy = openProject.isPending

  async function pickFolder() {
    try {
      const picked = await openDialog({ directory: true, multiple: false, title: 'Open a Wobu project folder' })
      if (typeof picked !== 'string') return
      openProject.mutate(picked, {
        onError: (e) => toast(errorMessage(e), 'error'),
      })
    } catch (e) {
      toast(errorMessage(e), 'error')
    }
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
            onOpen={(p) =>
              openProject.mutate(p.path, { onError: (e) => toast(errorMessage(e), 'error') })
            }
          />

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
  onOpen,
}: {
  recent: ProjectSummary[] | undefined
  pending: boolean
  failed: string | null
  busy: boolean
  onOpen: (p: ProjectSummary) => void
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
        <button key={p.id} className="lch-card" onClick={() => onOpen(p)} disabled={busy}>
          <span className="nm">
            {p.name}
            {p.readOnly && <span className="tag">read-only</span>}
            {!p.readOnly && p.onNetworkShare && <span className="tag">share</span>}
          </span>
          <span className="pth">{p.path}</span>
          <span className="when">{lastOpened(p.lastOpenedAt)}</span>
        </button>
      ))}
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
    <div
      className="scrim"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div className="sheet" role="dialog" aria-label="New project">
        <h2>New project</h2>
        <p>
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
            autoFocus
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submit()
            }}
          />
        </div>

        {err && <div className="sheet-err">{err}</div>}

        <div className="sheet-actions">
          <button className="btn btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={submit} disabled={createProject.isPending}>
            {createProject.isPending ? 'Creating…' : 'Create project'}
          </button>
        </div>
      </div>
    </div>
  )
}
