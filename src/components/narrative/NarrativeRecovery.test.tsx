import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { NarrativeRecovery } from './NarrativeRecovery'
const api = vi.hoisted(() => ({ list: vi.fn(), restore: vi.fn() }))
vi.mock('../../lib/api/narrativeRecovery', () => ({
  narrativeRecoveryList: api.list,
  narrativeRecoveryRestore: api.restore,
}))
const item = {
  id: 'deletion',
  name: 'Council hearing',
  target: 'narrative/scenes/council.yaml',
  hash: 'original',
  restored: false,
}
function mount(readOnly = false) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const invalidate = vi.spyOn(queryClient, 'invalidateQueries')
  render(
    <QueryClientProvider client={queryClient}>
      <NarrativeRecovery readOnly={readOnly} onClose={vi.fn()} />
    </QueryClientProvider>,
  )
  return invalidate
}
beforeEach(() => {
  vi.resetAllMocks()
  api.list.mockResolvedValue([item])
})
describe('narrative recovery', () => {
  it('shows named retained paths, restores explicitly and refreshes source/conflict queries', async () => {
    api.list.mockResolvedValueOnce([item]).mockResolvedValue([{ ...item, restored: true }])
    api.restore.mockResolvedValue({ status: 'saved', id: item.id })
    const invalidate = mount()
    expect(await screen.findByText(item.name)).toBeInTheDocument()
    expect(screen.getByText(item.target)).toBeInTheDocument()
    expect(api.restore).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Restore Council hearing' }))
    expect(
      await screen.findByText(`Original content restored to ${item.target}.`),
    ).toBeInTheDocument()
    expect(screen.getByText('Restoration requested')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Retry restore Council hearing' })).toBeEnabled()
    expect(api.restore).toHaveBeenCalledWith(item.id)
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['narrative_scenes'] })
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['conflicts'] })
  })
  it('reports the retained competing copy without claiming the newer file was replaced', async () => {
    api.list.mockResolvedValueOnce([item]).mockResolvedValue([{ ...item, restored: true }])
    api.restore.mockResolvedValue({
      status: 'conflict',
      id: item.id,
      conflictPath: 'narrative/scenes/council.conflict-peer.yaml',
    })
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Restore Council hearing' }))
    expect(await screen.findByText(/The newer current file was kept/)).toHaveTextContent(
      'council.conflict-peer.yaml',
    )
    expect(screen.getByText('Restoration requested')).toBeInTheDocument()
  })
  it('keeps read-only recovery available for inspection and permits retry after a list error', async () => {
    api.list.mockRejectedValueOnce({ message: 'Share disconnected' }).mockResolvedValue([item])
    mount(true)
    expect(await screen.findByRole('alert')).toHaveTextContent('Share disconnected')
    fireEvent.click(screen.getByRole('button', { name: 'Refresh history' }))
    expect(await screen.findByText(item.name)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Restore Council hearing' })).toBeDisabled()
    expect(api.restore).not.toHaveBeenCalled()
  })
  it('keeps a failed restore available and does not claim a restoration marker', async () => {
    api.restore.mockRejectedValue({ message: 'Read-only share' })
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Restore Council hearing' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Read-only share')
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Restore Council hearing' })).toBeEnabled(),
    )
    expect(screen.getByText('Original version retained')).toBeInTheDocument()
  })
})
