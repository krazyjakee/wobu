import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { toast, useUI } from '../store/ui'
import { Toasts } from './Toasts'

beforeEach(() => {
  useUI.setState({ toasts: [] })
})

afterEach(() => {
  vi.useRealTimers()
})

function enqueue(text: string, kind: 'info' | 'error' = 'info') {
  let id = 0
  act(() => {
    id = toast(text, kind)
  })
  return id
}

describe('toast announcements', () => {
  it('announces information politely and failures as alerts', () => {
    render(<Toasts />)

    enqueue('Saved')
    enqueue('Save failed', 'error')

    expect(screen.getByRole('status')).toHaveAttribute('aria-live', 'polite')
    expect(screen.getByRole('status')).toHaveTextContent('Saved')
    expect(screen.getByRole('alert')).toHaveAttribute('aria-live', 'assertive')
    expect(screen.getByRole('alert')).toHaveTextContent('Save failed')
  })

  it('keeps an existing live-region node stable when another toast is enqueued', () => {
    render(<Toasts />)
    enqueue('First')
    const firstRegion = screen.getByRole('status')

    enqueue('Second')

    expect(screen.getAllByRole('status')).toHaveLength(2)
    expect(screen.getByText('First').closest('[role="status"]')).toBe(firstRegion)
  })

  it('updates only the intended live-region row', () => {
    render(<Toasts />)
    const id = enqueue('Uploading')
    enqueue('Unrelated')
    const updatedRegion = screen.getByText('Uploading').closest('[role="status"]')
    const untouchedRegion = screen.getByText('Unrelated').closest('[role="status"]')

    act(() => useUI.getState().updateToast(id, { text: 'Upload complete' }))

    expect(screen.getByText('Upload complete').closest('[role="status"]')).toBe(updatedRegion)
    expect(screen.getByText('Unrelated').closest('[role="status"]')).toBe(untouchedRegion)
  })
})

describe('toast interaction and expiry', () => {
  it('dismisses from the keyboard with Escape', () => {
    render(<Toasts />)
    enqueue('Saved')
    const dismiss = screen.getByRole('button', { name: 'Dismiss message: Saved' })

    dismiss.focus()
    fireEvent.keyDown(dismiss, { key: 'Escape' })

    expect(screen.queryByText('Saved')).not.toBeInTheDocument()
  })

  it('expires an untouched informational toast', () => {
    vi.useFakeTimers()
    render(<Toasts />)
    enqueue('Saved')

    act(() => vi.advanceTimersByTime(4_199))
    expect(screen.getByText('Saved')).toBeInTheDocument()
    act(() => vi.advanceTimersByTime(1))
    expect(screen.queryByText('Saved')).not.toBeInTheDocument()
  })

  it('starts a fresh expiry period when a toast is updated', () => {
    vi.useFakeTimers()
    render(<Toasts />)
    const id = enqueue('Uploading')

    act(() => vi.advanceTimersByTime(4_000))
    act(() => useUI.getState().updateToast(id, { text: 'Processing upload' }))
    act(() => vi.advanceTimersByTime(4_199))
    expect(screen.getByText('Processing upload')).toBeInTheDocument()
    act(() => vi.advanceTimersByTime(1))
    expect(screen.queryByText('Processing upload')).not.toBeInTheDocument()
  })

  it('pauses expiry while focus remains inside the toast', () => {
    vi.useFakeTimers()
    render(<Toasts />)
    enqueue('Saved')
    const dismiss = screen.getByRole('button', { name: 'Dismiss message: Saved' })

    act(() => vi.advanceTimersByTime(4_000))
    fireEvent.focusIn(dismiss)
    act(() => vi.advanceTimersByTime(5_000))
    expect(screen.getByText('Saved')).toBeInTheDocument()

    fireEvent.focusOut(dismiss, { relatedTarget: null })
    act(() => vi.advanceTimersByTime(199))
    expect(screen.getByText('Saved')).toBeInTheDocument()
    act(() => vi.advanceTimersByTime(1))
    expect(screen.queryByText('Saved')).not.toBeInTheDocument()
  })

  it('pauses expiry while the toast is hovered', () => {
    vi.useFakeTimers()
    render(<Toasts />)
    enqueue('Copied')
    const region = screen.getByRole('status')

    act(() => vi.advanceTimersByTime(4_000))
    fireEvent.pointerEnter(region)
    act(() => vi.advanceTimersByTime(1_000))
    expect(screen.getByText('Copied')).toBeInTheDocument()

    fireEvent.pointerLeave(region)
    act(() => vi.advanceTimersByTime(200))
    expect(screen.queryByText('Copied')).not.toBeInTheDocument()
  })

  it('keeps an action available until it runs, then dismisses the toast', () => {
    vi.useFakeTimers()
    const retry = vi.fn()
    render(<Toasts />)
    act(() => {
      toast('Upload failed', 'error', { action: { label: 'Retry', run: retry } })
    })

    act(() => vi.advanceTimersByTime(60_000))
    expect(screen.getByText('Upload failed')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }))

    expect(retry).toHaveBeenCalledOnce()
    expect(screen.queryByText('Upload failed')).not.toBeInTheDocument()
  })

  it('retains and reveals durable error detail', () => {
    vi.useFakeTimers()
    render(<Toasts />)
    act(() => {
      toast('Save failed', 'error', { detail: 'EACCES: story.wobu' })
    })

    act(() => vi.advanceTimersByTime(60_000))
    fireEvent.click(screen.getByRole('button', { name: 'Details' }))
    expect(screen.getByText('EACCES: story.wobu')).toBeInTheDocument()
  })
})
