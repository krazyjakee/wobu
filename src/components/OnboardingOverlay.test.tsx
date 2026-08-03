import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LEGAL_VERSION, useOnboarding } from '../store/onboarding'
import { useUI } from '../store/ui'
import { OnboardingOverlay } from './OnboardingOverlay'

const h = vi.hoisted(() => ({ invoke: vi.fn(), openDialog: vi.fn(), close: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: h.openDialog }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: h.close }),
}))

interface Record_ {
  legalAcceptedAt: string | null
  legalVersion: string | null
  completedAt: string | null
}

const FRESH: Record_ = { legalAcceptedAt: null, legalVersion: null, completedAt: null }
const SETTLED: Record_ = {
  legalAcceptedAt: '2026-08-01T09:00:00Z',
  legalVersion: LEGAL_VERSION,
  completedAt: '2026-08-01T09:04:00Z',
}

let stored: Record_
/** `null` for a project-less launcher; a summary once one is open. */
let project: unknown
let health: unknown

function show() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
  })
  render(
    <QueryClientProvider client={client}>
      <OnboardingOverlay />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  stored = { ...FRESH }
  project = null
  health = { state: 'unconfigured', detail: 'No image backend is selected for this project.' }
  h.invoke.mockReset()
  h.close.mockReset()
  h.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'onboarding_state') return Promise.resolve(stored)
    if (command === 'onboarding_accept_legal') {
      stored = {
        ...stored,
        legalAcceptedAt: '2026-08-04T10:00:00Z',
        legalVersion: String(args?.version),
      }
      return Promise.resolve(stored)
    }
    if (command === 'onboarding_finish') {
      stored = { ...stored, completedAt: '2026-08-04T10:01:00Z' }
      return Promise.resolve(stored)
    }
    if (command === 'project_current') return Promise.resolve(project)
    if (command === 'status_bar_backend') {
      return Promise.resolve({
        image: { provider: 'gemini', label: 'Gemini', model: 'x', contextTokens: null },
        text: { provider: 'anthropic', label: 'Anthropic', model: 'y', contextTokens: null },
        health,
      })
    }
    throw new Error(`unexpected command: ${command}`)
  })
  useOnboarding.setState({ record: null, open: false, step: 'legal', saving: false })
  useUI.setState({ mode: 'library' })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('the legal gate', () => {
  it('blocks the first run until both documents are accepted, and records the revision', async () => {
    show()

    await screen.findByRole('dialog', { name: 'Before you start' })
    // The gate is the one step with no way past it but agreement: no Skip, and
    // Escape must not stand in for one.
    expect(screen.queryByRole('button', { name: 'Skip for now' })).toBeNull()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.getByRole('dialog', { name: 'Before you start' })).toBeTruthy()
    expect(h.invoke).not.toHaveBeenCalledWith('onboarding_finish', expect.anything())

    // Both documents are readable here rather than only linked to.
    fireEvent.click(screen.getByRole('button', { name: 'Privacy policy' }))
    expect(screen.getByLabelText('Privacy policy').textContent).toContain('There is no telemetry')
    fireEvent.click(screen.getByRole('button', { name: 'Terms of use' }))
    expect(screen.getByLabelText('Terms of use').textContent).toContain('MIT licence')

    fireEvent.click(screen.getByRole('button', { name: 'I agree — continue' }))
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('onboarding_accept_legal', { version: LEGAL_VERSION }),
    )
    await screen.findByRole('dialog', { name: 'Welcome to wobu' })
  })

  it('does not advance when the agreement could not be written down', async () => {
    h.invoke.mockImplementation((command: string) => {
      if (command === 'onboarding_state') return Promise.resolve(FRESH)
      if (command === 'onboarding_accept_legal') return Promise.reject(new Error('read-only'))
      throw new Error(`unexpected command: ${command}`)
    })
    show()

    fireEvent.click(await screen.findByRole('button', { name: 'I agree — continue' }))
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/has not been recorded/)
    expect(screen.getByRole('dialog', { name: 'Before you start' })).toBeTruthy()
  })

  it('asks again when the documents changed, and asks only that', async () => {
    stored = { ...SETTLED, legalVersion: 'terms 1 January 2020; privacy 1 January 2020' }
    show()

    fireEvent.click(await screen.findByRole('button', { name: 'I agree — continue' }))
    // The tour was already finished, so agreeing again returns the reader to
    // the app rather than replaying four screens they have seen.
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(h.invoke).not.toHaveBeenCalledWith('onboarding_finish', expect.anything())
  })
})

describe('the tour', () => {
  it('stays away once it has been seen', async () => {
    stored = { ...SETTLED }
    show()

    await waitFor(() => expect(h.invoke).toHaveBeenCalledWith('onboarding_state', undefined))
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('records that it is over when skipped, and changes nothing else', async () => {
    stored = { ...SETTLED, completedAt: null }
    show()

    fireEvent.click(await screen.findByRole('button', { name: 'Skip for now' }))
    await waitFor(() => expect(h.invoke).toHaveBeenCalledWith('onboarding_finish', undefined))
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(useUI.getState().mode).toBe('library')
  })

  it('names only surfaces this build has', async () => {
    stored = { ...SETTLED, completedAt: null }
    show()

    const dialog = await screen.findByRole('dialog', { name: 'Welcome to wobu' })
    expect(dialog.textContent).toContain('Library')
    expect(dialog.textContent).toContain('Forge')
    // Board and the History tab were removed in #144. A tour that still
    // described them would be the most confusing possible first impression.
    expect(dialog.textContent).not.toContain('Board')
    expect(dialog.textContent).not.toContain('History')
  })

  it('sends a reader with no usable image backend to Providers instead of to Generate', async () => {
    stored = { ...SETTLED, completedAt: null }
    project = {
      id: 'ashfall-id',
      name: 'Ashfall',
      path: '/worlds/Ashfall.wobu',
      onNetworkShare: false,
      readOnly: false,
      lastOpenedAt: null,
    }
    show()

    await screen.findByRole('dialog', { name: 'Welcome to wobu' })
    fireEvent.click(screen.getByRole('button', { name: 'First concept' }))

    const finish = await screen.findByRole('button', { name: /Set up a provider/ })
    expect(screen.queryByRole('button', { name: 'Finish' })).toBeNull()

    fireEvent.click(finish)
    await waitFor(() => expect(useUI.getState().mode).toBe('settings'))
    expect(h.invoke).toHaveBeenCalledWith('onboarding_finish', undefined)
  })

  it('offers to finish once the project really can generate', async () => {
    stored = { ...SETTLED, completedAt: null }
    project = {
      id: 'ashfall-id',
      name: 'Ashfall',
      path: '/worlds/Ashfall.wobu',
      onNetworkShare: false,
      readOnly: false,
      lastOpenedAt: null,
    }
    health = { state: 'connected', externalQueue: null }
    show()

    await screen.findByRole('dialog', { name: 'Welcome to wobu' })
    fireEvent.click(screen.getByRole('button', { name: 'First concept' }))

    await screen.findByRole('button', { name: 'Finish' })
    expect(screen.queryByRole('button', { name: /Set up a provider/ })).toBeNull()
  })
})

describe('running it again', () => {
  it('reopens at the tour rather than at the gate, and never un-accepts', async () => {
    stored = { ...SETTLED }
    show()
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())

    useOnboarding.getState().restart()
    await screen.findByRole('dialog', { name: 'Welcome to wobu' })
    expect(screen.queryByRole('dialog', { name: 'Before you start' })).toBeNull()
    expect(h.invoke).not.toHaveBeenCalledWith('onboarding_accept_legal', expect.anything())
  })
})
