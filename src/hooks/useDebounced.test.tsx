import { act, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { useDebounced } from './useDebounced'

afterEach(() => vi.useRealTimers())

describe('useDebounced', () => {
  it('clears immediately without reviving the previous value when typing resumes', () => {
    vi.useFakeTimers()
    const { result, rerender } = renderHook(({ value }) => useDebounced(value, 140), {
      initialProps: { value: 'old query' },
    })

    rerender({ value: '' })
    expect(result.current).toBe('')

    rerender({ value: 'new query' })
    expect(result.current).toBe('')
    act(() => vi.advanceTimersByTime(140))
    expect(result.current).toBe('new query')
  })
})
