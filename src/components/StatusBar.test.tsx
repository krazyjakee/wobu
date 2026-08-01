import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { StatusBar, relativeTime } from './StatusBar'
import type {
  ProjectSummary,
  ProjectSyncStatus,
  QueueSnapshot,
  StatusBarBackend,
} from '../lib/api'
import { useUI } from '../store/ui'

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

const emptyQueue: QueueSnapshot = { jobs: [], queued: 0, running: 0, retrying: 0 }

function open(
  sync: ProjectSyncStatus | null,
  backend: StatusBarBackend | null = null,
  queue: QueueSnapshot = emptyQueue,
) {
  render(
    <StatusBar
      project={project}
      nodeCount={4}
      loading={false}
      peers={[]}
      sync={sync}
      backend={backend}
      queue={queue}
    />,
  )
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

describe('backend and queue facts', () => {
  const backend: StatusBarBackend = {
    image: { provider: 'comfyui', label: 'ComfyUI', model: 'flux-dev', contextTokens: null },
    text: {
      provider: 'anthropic',
      label: 'Anthropic',
      model: 'claude-sonnet-5',
      contextTokens: 1_000_000,
    },
    health: { state: 'connected', externalQueue: 2 },
  }

  it('shows probed health, both active models, live depth, and the last successful generation', () => {
    open(null, backend, {
      queued: 1,
      running: 1,
      retrying: 0,
      jobs: [
        {
          id: 'g1',
          kind: 'generate',
          label: 'Generate Kael',
          subjectId: 'kael',
          state: 'done',
          attempt: 1,
          elapsedMs: 4_200,
        },
        {
          id: 'e1',
          kind: 'enhance',
          label: 'Enhance Kael',
          subjectId: 'kael',
          state: 'running',
          attempt: 1,
          elapsedMs: 900,
        },
      ],
    })

    expect(screen.getByText('ComfyUI connected')).toBeTruthy()
    expect(screen.getByText('· flux-dev')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Open job queue, 2 jobs' })).toBeTruthy()
    expect(screen.getByText(/claude-sonnet-5 · 1m ctx/)).toBeTruthy()
    expect(screen.getByText('· ⏱ 4.2s')).toBeTruthy()
  })

  it('opens the real queue view instead of a dead status label', () => {
    useUI.setState({ mode: 'library' })
    open(null, backend)
    fireEvent.click(screen.getByRole('button', { name: 'Open job queue, 0 jobs' }))
    expect(useUI.getState().mode).toBe('forge')
  })

  it('does not call a configured address connected when its probe failed', () => {
    open(null, { ...backend, health: { state: 'unavailable', detail: 'connection refused' } })
    expect(screen.getByText('ComfyUI unavailable')).toHaveProperty('title', 'connection refused')
    expect(screen.queryByText('ComfyUI connected')).toBeNull()
  })
})
