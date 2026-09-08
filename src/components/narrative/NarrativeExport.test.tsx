import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { NarrativeExport } from './NarrativeExport'
import { narrativeExport, narrativeExportCheck } from '../../lib/api/narrativeExport'
vi.mock('../../lib/api/narrativeExport', () => ({
  narrativeExport: vi.fn(),
  narrativeExportCheck: vi.fn(),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ revealItemInDir: vi.fn() }))
const ready = { diagnostics: [], payloadHash: 'abc123', scenes: 2, strings: 8, bytes: 1000 }
beforeEach(() => {
  vi.clearAllMocks()
})
describe('Narrative export', () => {
  it('requires validation and destination, then publishes exactly the checked identity', async () => {
    vi.mocked(narrativeExportCheck).mockResolvedValue(ready)
    vi.mocked(narrativeExport).mockResolvedValue({
      destination: '/exports/story',
      payloadHash: 'abc123',
      scenes: 2,
      strings: 8,
      bytes: 1000,
    })
    render(<NarrativeExport onClose={vi.fn()} />)
    expect(screen.getByRole('button', { name: 'Export package' })).toBeDisabled()
    fireEvent.change(screen.getByLabelText('Destination folder'), {
      target: { value: '/exports/story' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Check export' }))
    await screen.findByText('Ready: 2 scenes, 8 strings, 1000 bytes.')
    fireEvent.click(screen.getByRole('button', { name: 'Export package' }))
    await screen.findByText('Exported 2 scenes and 8 strings to /exports/story.')
    expect(narrativeExport).toHaveBeenCalledWith(
      { profile: 'development', commands: {}, debug: false },
      '/exports/story',
      'abc123',
    )
  })
  it('shows release blockers and invalidates validation on option changes', async () => {
    vi.mocked(narrativeExportCheck)
      .mockResolvedValueOnce(ready)
      .mockResolvedValueOnce({
        ...ready,
        payloadHash: null,
        diagnostics: [
          {
            scene: 'scene',
            site: {},
            severity: 'error',
            code: 'missing_text',
            message: 'Dialogue must be approved',
          },
        ],
      })
    render(<NarrativeExport onClose={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Check export' }))
    await screen.findByText(/Ready:/)
    fireEvent.change(screen.getByLabelText('Profile'), { target: { value: 'release' } })
    expect(screen.queryByText(/Ready:/)).toBeNull()
    expect(screen.getByRole('checkbox')).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: 'Check export' }))
    await screen.findByText(/Export blocked/)
    expect(screen.getByRole('list', { name: 'Export diagnostics' })).toHaveTextContent(
      'Dialogue must be approved',
    )
    expect(screen.getByRole('button', { name: 'Export package' })).toBeDisabled()
  })
  it('keeps failure visible and requires a new check after source drift or destination failure', async () => {
    vi.mocked(narrativeExportCheck).mockResolvedValue(ready)
    vi.mocked(narrativeExport).mockRejectedValue(
      new Error('Saved narrative changed after validation.'),
    )
    render(<NarrativeExport onClose={vi.fn()} />)
    fireEvent.change(screen.getByLabelText('Destination folder'), {
      target: { value: '/exports/story' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Check export' }))
    await screen.findByText(/Ready:/)
    fireEvent.click(screen.getByRole('button', { name: 'Export package' }))
    await waitFor(() =>
      expect(screen.getByRole('alert')).toHaveTextContent('Saved narrative changed'),
    )
    expect(screen.getByRole('button', { name: 'Export package' })).toBeDisabled()
    expect(screen.getByLabelText('Destination folder')).toHaveValue('/exports/story')
  })
})
