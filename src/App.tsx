import { useEffect } from 'react'
import { useCurrentProject, useShareListener, useWorldChangedListener } from './lib/queries'
import { errorMessage, isTauri } from './lib/api'
import { useUndoStack } from './lib/undo'
import { Launcher } from './components/Launcher'
import { Workspace } from './components/Workspace'
import { Toasts } from './components/Toasts'
import { Icon } from './components/Icon'
import { useUiScale } from './hooks/useUiScale'

export function App() {
  useWorldChangedListener()
  useShareListener()
  // Applied at the root so it covers the launcher and the overlays too, not
  // just the workspace.
  useUiScale()
  const current = useCurrentProject()

  // The undo stack belongs to one project's world, so it is tied to whichever
  // project is open rather than cleared at the close call site. Doing it here
  // covers every way the project can change — closed, swapped, or restored on a
  // reload that never went through `useOpenProject` at all.
  const projectId = current.data?.id ?? null
  useEffect(() => useUndoStack.getState().setProject(projectId), [projectId])

  // Outside the Tauri webview there is no backend at all — say so plainly
  // rather than spinning forever.
  if (!isTauri()) {
    return (
      <>
        <div className="launcher">
          <div className="lch-bar">
            <div className="brand">
              <span className="brand-mark" />
              wobu
            </div>
          </div>
          <div className="lch-body">
            <div className="lch-inner">
              <div className="empty">
                <Icon name="lock" size="xl" />
                <h3>No backend</h3>
                <p>
                  This page is being served by Vite on its own. Wobu&apos;s commands live in the
                  Rust core, so nothing can be loaded here. Launch the desktop shell instead.
                </p>
              </div>
            </div>
          </div>
        </div>
        <Toasts />
      </>
    )
  }

  if (current.isPending) {
    return (
      <>
        <div className="launcher">
          <div className="lch-bar">
            <div className="brand">
              <span className="brand-mark" />
              wobu
            </div>
          </div>
          <div className="lch-body" />
        </div>
        <Toasts />
      </>
    )
  }

  const project = current.data ?? null

  return (
    <>
      {project ? (
        <Workspace project={project} />
      ) : (
        <Launcher error={current.isError ? errorMessage(current.error) : null} />
      )}
      <Toasts />
    </>
  )
}
