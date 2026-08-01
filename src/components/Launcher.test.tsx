import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ProjectSummary } from '../lib/api'
import { Launcher } from './Launcher'

const h = vi.hoisted(() => ({ invoke: vi.fn(), openDialog: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: h.openDialog }))
vi.mock('../hooks/useOpenProgress', () => ({ useOpenProgress: () => null }))
vi.mock('./WindowControls', () => ({ WindowControls: () => null }))

const ashfall: ProjectSummary = {
  id: 'ashfall-id',
  name: 'Ashfall',
  path: '/worlds/Ashfall.wobu',
  onNetworkShare: false,
  readOnly: false,
  lastOpenedAt: '2026-08-01T12:00:00Z',
}

let recent: ProjectSummary[]
let forget: Promise<void>
let resolveForget: (() => void) | null
let openError: Error | null

function showLauncher() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  })
  render(
    <QueryClientProvider client={client}>
      <Launcher error={null} />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  recent = [ashfall]
  resolveForget = null
  openError = null
  forget = new Promise((resolve) => {
    resolveForget = resolve
  })
  h.invoke.mockReset()
  h.openDialog.mockReset()
  h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'project_recent') return Promise.resolve(recent)
    if (command === 'project_open') {
      return openError ? Promise.reject(openError) : Promise.resolve(ashfall)
    }
    if (command === 'project_recent_forget') {
      recent = recent.filter((project) => project.id !== args?.id)
      return forget
    }
    throw new Error(`unexpected command: ${command}`)
  })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('launcher recent projects', () => {
  it('removes a card optimistically while making the launcher-only scope explicit', async () => {
    showLauncher()
    await screen.findByRole('button', { name: /^Ashfall/ })

    fireEvent.click(screen.getByRole('button', { name: 'More actions for Ashfall' }))
    const menu = screen.getByRole('menu')
    expect(within(menu).getByText(/Project files stay on disk/)).toBeTruthy()
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Remove from Recent' }))

    await waitFor(() => expect(screen.queryByRole('button', { name: /^Ashfall/ })).toBeNull())
    expect(h.invoke).toHaveBeenCalledWith('project_recent_forget', { id: ashfall.id })

    await act(async () => resolveForget?.())
  })

  it('offers retry and launcher-only removal after a recent project cannot open', async () => {
    openError = new Error('The project folder is unavailable.')
    showLauncher()

    fireEvent.click(await screen.findByRole('button', { name: /^Ashfall/ }))
    const alert = await screen.findByRole('alert')
    expect(within(alert).getByText(/project folder is unavailable/)).toBeTruthy()
    expect(within(alert).getByText(/changes this launcher list only/i)).toBeTruthy()

    fireEvent.click(within(alert).getByRole('button', { name: 'Retry' }))
    await waitFor(() => {
      expect(h.invoke.mock.calls.filter(([command]) => command === 'project_open')).toHaveLength(2)
    })

    const retriedAlert = await screen.findByRole('alert')
    fireEvent.click(within(retriedAlert).getByRole('button', { name: 'Remove from Recent' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: /^Ashfall/ })).toBeNull())
    expect(h.invoke).toHaveBeenCalledWith('project_recent_forget', { id: ashfall.id })

    await act(async () => resolveForget?.())
  })
})
