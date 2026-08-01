import { render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { StatusBar, relativeTime } from './StatusBar'
import type { ProjectSummary, ProjectSyncStatus } from '../lib/api'

const project: ProjectSummary = {
  id: 'p1',
  name: 'Ashfall',
  path: '/worlds/ashfall',
  onNetworkShare: false,
  readOnly: false,
  lastOpenedAt: null,
}

function status(over: Partial<ProjectSyncStatus> = {}): ProjectSyncStatus {
  return {
    project: 'p1',
    state: 'idle',
    peers: [],
    ...over,
  }
}

function open(sync: ProjectSyncStatus | null) {
  render(<StatusBar project={project} nodeCount={4} loading={false} peers={[]} sync={sync} />)
}

afterEach(() => vi.useRealTimers())

describe('peer sync in the status bar', () => {
  it('names the last genuinely converged peer and says how long ago it happened', () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-08-01T12:12:30Z'))
    open(
      status({
        peers: [
          {
            endpointId: 'peer-1',
            alias: 'Nadia',
            connected: false,
            lastConvergedAt: '2026-08-01T12:00:00Z',
          },
        ],
      }),
    )

    expect(screen.getByText('· last synced with Nadia · 12 minutes ago')).toBeTruthy()
    expect(screen.getByText(/peer edits arrive only while both people run Wobu/)).toBeTruthy()
    expect(screen.getByText(/no seed node/)).toBeTruthy()
  })

  it('shows live state and connected peers without turning them into a cloud guarantee', () => {
    open(
      status({
        state: 'syncing',
        peers: [
          {
            endpointId: 'peer-1',
            alias: 'amber-heron',
            connected: true,
            lastConvergedAt: null,
          },
        ],
      }),
    )

    expect(screen.getByText('sync · syncing')).toBeTruthy()
    expect(screen.getByText('· amber-heron connected')).toBeTruthy()
    expect(screen.queryByText(/last synced/)).toBeNull()
  })

  it('does not show a sync claim for a project this installation has not shared', () => {
    open(null)
    expect(screen.queryByText(/sync ·/)).toBeNull()
    expect(screen.queryByText(/no seed node/)).toBeNull()
  })
})

describe('relative sync time', () => {
  it('does not render future clock skew as a negative duration', () => {
    expect(relativeTime(Date.parse('2026-08-01T12:01:00Z'), Date.parse('2026-08-01T12:00:00Z')))
      .toBe('just now')
  })
})
