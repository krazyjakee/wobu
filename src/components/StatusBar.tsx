import type { Peer, ProjectSummary } from '../lib/api'
import { sessionsText, sessionsTitle } from '../lib/presence'
import { useUI } from '../store/ui'

/**
 * Honest by construction: everything here is something M1 actually knows.
 * Backend health, image model and queue depth arrive with M5.
 */
export function StatusBar({
  project,
  nodeCount,
  loading,
  peers,
}: {
  project: ProjectSummary
  nodeCount: number
  loading: boolean
  peers: Peer[]
}) {
  const navCollapsed = useUI((s) => s.navCollapsed)
  const inspCollapsed = useUI((s) => s.inspCollapsed)
  // Absent rather than "1 session" when nobody else is here. A count that never
  // changes is a count nobody reads, and the point of this one is that it moved.
  const sessions = sessionsText(peers)

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

      {sessions && (
        <>
          <span className="presence" role="img" aria-label="Other sessions in this project" />
          <span title={sessionsTitle(peers)}>{sessions}</span>
          <span className="sep" />
        </>
      )}
      <span>
        {loading ? 'reading world…' : `${nodeCount} ${nodeCount === 1 ? 'node' : 'nodes'}`}
      </span>
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
