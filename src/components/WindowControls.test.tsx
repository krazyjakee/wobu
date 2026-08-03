import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { WindowControls } from './WindowControls'

const h = vi.hoisted(() => ({
  closeWindow: vi.fn(() => Promise.resolve()),
  destroyWindow: vi.fn(() => Promise.resolve()),
  minimizeWindow: vi.fn(() => Promise.resolve()),
  toggleMaximizeWindow: vi.fn(() => Promise.resolve()),
  isMaximized: vi.fn(() => Promise.resolve(false)),
}))

vi.mock('../lib/window', () => ({
  closeWindow: h.closeWindow,
  destroyWindow: h.destroyWindow,
  minimizeWindow: h.minimizeWindow,
  toggleMaximizeWindow: h.toggleMaximizeWindow,
  isMaximized: h.isMaximized,
  onResized: () => Promise.resolve(() => {}),
}))

beforeEach(() => {
  Object.values(h).forEach((fn) => fn.mockClear())
  h.isMaximized.mockResolvedValue(false)
})

describe('the frameless window controls', () => {
  it('asks to close rather than closing, so the exit gate still runs', () => {
    // `decorations: false` means this button *is* the titlebar's close control,
    // and the temptation is to make it do the closing. It must not: `close()`
    // raises `CloseRequested`, which is what `useSafeWindowClose` intercepts to
    // settle editor writes and warn about jobs in flight. `destroy()` skips all
    // of it, and is only ever correct on the far side of that gate — so seeing
    // it here would be an unsaved paragraph one click away.
    // See `docs/15-exit-policy.md`.
    render(<WindowControls />)
    fireEvent.click(screen.getByRole('button', { name: 'Close' }))

    expect(h.closeWindow).toHaveBeenCalledOnce()
    expect(h.destroyWindow).not.toHaveBeenCalled()
  })

  it('minimises and maximises without going near the close path', () => {
    render(<WindowControls />)
    fireEvent.click(screen.getByRole('button', { name: 'Minimise' }))
    fireEvent.click(screen.getByRole('button', { name: 'Maximise' }))

    expect(h.minimizeWindow).toHaveBeenCalledOnce()
    expect(h.toggleMaximizeWindow).toHaveBeenCalledOnce()
    expect(h.closeWindow).not.toHaveBeenCalled()
    expect(h.destroyWindow).not.toHaveBeenCalled()
  })

  it('names the restore control for what it does once the window is maximised', async () => {
    h.isMaximized.mockResolvedValue(true)
    render(<WindowControls />)

    await waitFor(() => expect(screen.getByRole('button', { name: 'Restore' })).toBeInTheDocument())
    expect(screen.queryByRole('button', { name: 'Maximise' })).toBeNull()
  })
})
