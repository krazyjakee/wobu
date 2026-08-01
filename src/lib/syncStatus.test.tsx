import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useProjectSync } from './queries'
import type { ProjectSyncStatus, SyncStatus } from './api'

const h = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: ProjectSyncStatus }) => void>(),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, handler: (event: { payload: ProjectSyncStatus }) => void) => {
    h.listeners.set(name, handler)
    return Promise.resolve(() => h.listeners.delete(name))
  },
}))

function snapshot(state: ProjectSyncStatus['state']): ProjectSyncStatus {
  return { project: 'p1', state, peers: [] }
}

function Probe() {
  const sync = useProjectSync('p1')
  return <span>{sync?.state ?? 'none'}</span>
}

function open(initial: ProjectSyncStatus) {
  const answer: SyncStatus = {
    running: true,
    alias: 'local-peer',
    endpointId: 'local-id',
    persistent: true,
    shares: [],
    projects: [initial],
  }
  h.invoke.mockResolvedValue(answer)
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={qc}>
      <Probe />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  h.invoke.mockReset()
  h.listeners.clear()
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('sync status events', () => {
  it('catches up through sync_status when the listener mounted late', async () => {
    open(snapshot('offline'))
    expect(await screen.findByText('offline')).toBeTruthy()
    expect(h.invoke).toHaveBeenCalledWith('sync_status', undefined)
  })

  it('replaces the project snapshot from both state and peer events', async () => {
    open(snapshot('idle'))
    await screen.findByText('idle')
    await waitFor(() => expect(h.listeners.size).toBe(2))

    act(() => h.listeners.get('sync:state')?.({ payload: snapshot('connecting') }))
    expect(await screen.findByText('connecting')).toBeTruthy()

    act(() => h.listeners.get('sync:peer')?.({ payload: snapshot('syncing') }))
    expect(await screen.findByText('syncing')).toBeTruthy()
  })
})
