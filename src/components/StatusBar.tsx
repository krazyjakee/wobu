import type { ProjectSummary } from '../lib/api'
import { useUI } from '../store/ui'

/**
 * Honest by construction: everything here is something M1 actually knows.
 * Backend health, image model and queue depth arrive with M5.
 */
export function StatusBar({
  project,
  nodeCount,
  loading,
}: {
  project: ProjectSummary
  nodeCount: number
  loading: boolean
}) {
  const navCollapsed = useUI((s) => s.navCollapsed)
  const inspCollapsed = useUI((s) => s.inspCollapsed)

  return (
    <footer className="status">
      <span className="dot dot-ok" />
      <b>{project.name}</b>
      <span className="sep" />
      <code title={project.path}>{project.path}</code>
      {project.onNetworkShare && (
        <>
          <span className="sep" />
          <span>network share · 5s poll</span>
        </>
      )}
      {project.readOnly && (
        <>
          <span className="sep" />
          <span>read-only</span>
        </>
      )}

      <div className="sspace" />

      <span>{loading ? 'reading world…' : `${nodeCount} ${nodeCount === 1 ? 'node' : 'nodes'}`}</span>
      <span className="sep" />
      <span>
        {navCollapsed ? '[ navigator hidden' : '[ navigator'} ·{' '}
        {inspCollapsed ? 'inspector hidden ]' : 'inspector ]'}
      </span>
      <span className="sep" />
      <span>M1 · no AI backends configured</span>
    </footer>
  )
}
