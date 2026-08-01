import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ProjectSummary } from '../lib/api'
import { WikiExportSection } from './WikiExportSection'

const h = vi.hoisted(() => ({ invoke: vi.fn(), save: vi.fn(), reveal: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: h.save }))
vi.mock('@tauri-apps/plugin-opener', () => ({ revealItemInDir: h.reveal }))

const project: ProjectSummary = {
  id: 'world-id',
  name: 'The Glass / Sea',
  path: '/worlds/glass-sea',
  onNetworkShare: false,
  readOnly: true,
  lastOpenedAt: null,
}

beforeEach(() => {
  h.invoke.mockReset()
  h.save.mockReset()
  h.reveal.mockReset()
  h.save.mockResolvedValue('/exports/glass-sea-wiki')
  h.reveal.mockResolvedValue(undefined)
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('static wiki export', () => {
  it('exports read-only projects to a newly chosen folder and reveals the result', async () => {
    h.invoke.mockResolvedValue({
      destination: '/exports/glass-sea-wiki',
      nodeCount: 12,
      imageCount: 7,
      missingImages: 2,
    })
    render(<WikiExportSection project={project} />)

    fireEvent.click(screen.getByRole('button', { name: 'Export static wiki…' }))

    await waitFor(() =>
      expect(h.save).toHaveBeenCalledWith({
        title: 'Export static world wiki',
        defaultPath: 'The-Glass-Sea-wiki',
      }),
    )
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('project_export_wiki', {
        destination: '/exports/glass-sea-wiki',
      }),
    )
    expect(await screen.findByText('Exported 12 nodes and 7 images.')).toBeInTheDocument()
    expect(screen.getByText('2 missing images were replaced with placeholders.')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Reveal exported folder' }))
    await waitFor(() => expect(h.reveal).toHaveBeenCalledWith('/exports/glass-sea-wiki'))
  })

  it('leaves a refused existing destination visible as an actionable error', async () => {
    h.invoke.mockRejectedValue({
      code: 'project.already_exists',
      message: '/exports/glass-sea-wiki already exists',
      retryable: false,
    })
    render(<WikiExportSection project={project} />)

    fireEvent.click(screen.getByRole('button', { name: 'Export static wiki…' }))

    expect(await screen.findByRole('alert')).toHaveTextContent(
      '/exports/glass-sea-wiki already exists',
    )
    expect(screen.queryByRole('button', { name: 'Reveal exported folder' })).not.toBeInTheDocument()
  })
})
