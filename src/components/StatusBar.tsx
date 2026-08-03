import { useEffect, useMemo, useState } from 'react'
import { jobDepth } from '../lib/api'
import type {
  Peer,
  ProjectSummary,
  ProjectSyncStatus,
  QueueSnapshot,
  StatusBarBackend,
  SyncPeerStatus,
} from '../lib/api'
import { elapsedText, lastGeneration } from '../lib/jobs'
import { sessionsText, sessionsTitle } from '../lib/presence'
import { relativeTime } from '../lib/time'
import { useUI } from '../store/ui'
import { NotificationCentre } from './NotificationCentre'

/**
 * Honest by construction: every state here comes from a backend observation.
 * In particular, idle is not rendered as "synced", and a completed peer round
 * is not rendered as cloud availability.
 */
export function StatusBar({
  project,
  nodeCount,
  loading,
  peers,
  sync,
  backend,
  queue,
}: {
  project: ProjectSummary
  nodeCount: number
  loading: boolean
  peers: Peer[]
  sync: ProjectSyncStatus | null
  backend: StatusBarBackend | null
  queue: QueueSnapshot
}) {
  const navCollapsed = useUI((s) => s.navCollapsed)
  const inspCollapsed = useUI((s) => s.inspCollapsed)
  const setMode = useUI((s) => s.setMode)
  // Absent rather than "1 session" when nobody else is here. A count that never
  // changes is a count nobody reads, and the point of this one is that it moved.
  const sessions = sessionsText(peers)
  const last = useMemo(() => lastConverged(sync?.peers ?? []), [sync])
  const now = useRelativeClock(last !== null)
  const generation = lastGeneration(queue)

  return (
    <footer className="status">
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

      {sync && (
        <>
          <span className="sep" />
          <span
            className={`dot ${sync.state === 'syncing' ? 'dot-ok' : sync.state === 'connecting' ? 'dot-warn' : ''}`}
            role="img"
            aria-label={`Sync ${sync.state}`}
          />
          <span>sync · {sync.state}</span>
          {connectedText(sync.peers) && <span>· {connectedText(sync.peers)}</span>}
          {last && (
            <span>
              · last synced with {last.alias} · {relativeTime(last.at, now)}
            </span>
          )}
          <span className="sync-caveat">
            · peer edits arrive only while both people run Wobu · no seed node
          </span>
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
      {backend ? (
        <>
          <span title={healthTitle(backend)}>{healthText(backend)}</span>
          {backend.image && <span>· {backend.image.model}</span>}
        </>
      ) : (
        <span>checking backend…</span>
      )}
      <button
        className="status-link"
        onClick={() => setMode('forge')}
        aria-label={`Open job queue, ${jobDepth(queue)} jobs`}
      >
        · queue {jobDepth(queue)}
      </button>
      {backend && (
        <span>
          · {backend.text.model}
          {backend.text.contextTokens && ` · ${contextText(backend.text.contextTokens)} ctx`}
        </span>
      )}
      {generation && <span title={generation.label}>· ⏱ {elapsedText(generation.elapsedMs)}</span>}
      {/* The status bar is the one surface present in every mode, which is what
          makes it the right home for a record that has to outlive the pane the
          failure happened in (#142). */}
      <NotificationCentre />
    </footer>
  )
}

function healthText(status: StatusBarBackend): string {
  if (!status.image) return 'image backend not selected'
  switch (status.health.state) {
    case 'connected':
      return `${status.image.label} connected`
    case 'unavailable':
      return `${status.image.label} unavailable`
    case 'unconfigured':
      return `${status.image.label} not configured`
    case 'unsupported':
      return `${status.image.label} unsupported`
  }
}

function healthTitle(status: StatusBarBackend): string {
  if (status.health.state !== 'connected') return status.health.detail
  if (status.health.externalQueue === null) return 'The selected image model answered its probe.'
  return `The backend answered its health probe and reports ${status.health.externalQueue} external jobs.`
}

function contextText(tokens: number): string {
  if (tokens >= 1_000_000) {
    const millions = tokens / 1_000_000
    return `${millions.toFixed(millions < 1.1 ? 0 : 1)}m`
  }
  return `${Math.round(tokens / 1_000)}k`
}

function connectedText(peers: SyncPeerStatus[]): string | null {
  const connected = peers.filter((peer) => peer.connected)
  if (connected.length === 0) return null
  if (connected.length === 1) return `${connected[0]?.alias ?? '1 peer'} connected`
  return `${connected.length} peers connected`
}

function lastConverged(peers: SyncPeerStatus[]): { alias: string; at: number } | null {
  let latest: { alias: string; at: number } | null = null
  for (const peer of peers) {
    if (!peer.lastConvergedAt) continue
    const at = Date.parse(peer.lastConvergedAt)
    if (!Number.isFinite(at)) continue
    if (!latest || at > latest.at) latest = { alias: peer.alias, at }
  }
  return latest
}

function useRelativeClock(active: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!active) return
    const timer = window.setInterval(() => setNow(Date.now()), 30_000)
    return () => window.clearInterval(timer)
  }, [active])
  return now
}
