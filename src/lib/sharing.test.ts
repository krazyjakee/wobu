import { beforeEach, describe, expect, it, vi } from 'vitest'
import { syncAccept, syncAcceptCancel, syncShare, syncStatus, syncUnshare } from './api'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))

beforeEach(() => {
  h.invoke.mockReset()
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('typed sharing command boundary', () => {
  it('wires status, share, probe, clone, cancel, and unshare with exact arguments', async () => {
    h.invoke.mockResolvedValue(null)

    await syncStatus()
    await syncShare()
    await syncAccept('wobuproject-ticket')
    await syncAccept('wobuproject-ticket', '/worlds')
    await syncAcceptCancel()
    await syncUnshare('project-id')

    // `call` forwards its optional `args` straight through, so a command with
    // no payload still reaches `invoke` as a second, undefined argument. That
    // is invisible to Tauri but not to a recorded call list.
    expect(h.invoke.mock.calls).toEqual([
      ['sync_status', undefined],
      ['sync_share', undefined],
      ['sync_accept', { token: 'wobuproject-ticket', destination: null, cancel: false }],
      ['sync_accept', { token: 'wobuproject-ticket', destination: '/worlds', cancel: false }],
      ['sync_accept', { token: null, destination: null, cancel: true }],
      ['sync_unshare', { project: 'project-id' }],
    ])
  })
})
