import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ProjectSummary } from '../lib/api'
import { SharingSheet } from './SharingSheet'

const h = vi.hoisted(() => ({ invoke: vi.fn(), writeText: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))

const project: ProjectSummary = {
  id: 'project-id',
  name: 'Ashfall',
  path: '/worlds/Ashfall.wobu',
  onNetworkShare: false,
  readOnly: false,
  lastOpenedAt: null,
}

beforeEach(() => {
  h.invoke.mockReset()
  h.writeText.mockReset()
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText: h.writeText },
  })
  h.invoke.mockImplementation((command: string) => {
    if (command === 'sync_status') {
      return Promise.resolve({
        running: true,
        alias: 'amber-heron',
        endpointId: 'endpoint',
        persistent: true,
        shares: [],
        projects: [],
      })
    }
    if (command === 'sync_share') {
      return Promise.resolve({
        project: project.id,
        token: 'wobuproject-secret',
        relayed: false,
        alias: 'amber-heron',
      })
    }
    if (command === 'sync_unshare') return Promise.resolve()
    throw new Error(`unexpected command ${command}`)
  })
  h.writeText.mockResolvedValue(undefined)
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('project sharing sheet', () => {
  it('warns about credentials and revocation, then displays and copies the ticket', async () => {
    render(<SharingSheet project={project} onClose={vi.fn()} />)
    expect(screen.getByText(/ticket is an access credential/i)).toBeTruthy()
    expect(screen.getByText(/one ticket cannot be revoked/i)).toBeTruthy()

    fireEvent.click(await screen.findByRole('button', { name: 'Share and copy ticket' }))
    expect(await screen.findByLabelText('Ticket')).toHaveValue('wobuproject-secret')
    expect(h.writeText).toHaveBeenCalledWith('wobuproject-secret')
    expect(screen.getByText(/same local network/i)).toBeTruthy()
  })

  it('shows an existing join and confirms that unshare revokes all tickets', async () => {
    h.invoke.mockImplementation((command: string) => {
      if (command === 'sync_status') {
        return Promise.resolve({
          running: true,
          alias: 'amber-heron',
          endpointId: 'endpoint',
          persistent: true,
          shares: [{ project: project.id, root: project.path, peers: 1, open: true }],
          projects: [
            {
              project: project.id,
              state: 'idle',
              peers: [
                {
                  endpointId: 'peer',
                  alias: 'silver-plover',
                  connected: false,
                  lastConvergedAt: null,
                },
              ],
            },
          ],
        })
      }
      if (command === 'sync_unshare') return Promise.resolve()
      throw new Error(`unexpected command ${command}`)
    })

    render(<SharingSheet project={project} onClose={vi.fn()} />)
    expect(await screen.findByText(/Joined with silver-plover/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Stop sharing…' }))
    expect(screen.getByRole('alertdialog')).toHaveTextContent(/revokes every ticket/i)
    fireEvent.click(screen.getByRole('button', { name: 'Stop sharing' }))
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('sync_unshare', { project: project.id }),
    )
  })
})
