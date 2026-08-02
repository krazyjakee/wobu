import { useEffect, useState } from 'react'

/**
 * `value`, but only after it has stopped changing for `delay` ms.
 *
 * Used to keep the FTS query off the critical path of typing. The palette's
 * local name filter runs on the raw value and stays instant; only the round
 * trip to SQLite waits, so the list the user is reading updates immediately and
 * gains the notes matches a moment later.
 */
export function useDebounced<T>(value: T, delay: number): T {
  const [state, setState] = useState({ input: value, settled: value })

  if (!Object.is(state.input, value)) {
    setState((current) => ({
      input: value,
      settled: value === ('' as unknown as T) ? value : current.settled,
    }))
  }

  useEffect(() => {
    // An immediate reset to an empty value rather than a delayed one: clearing
    // the box should clear the results now, not after the debounce. Waiting
    // leaves stale hits on screen for a query the user has already abandoned.
    const id = window.setTimeout(
      () =>
        setState((current) =>
          Object.is(current.input, value) ? { ...current, settled: value } : current,
        ),
      delay,
    )
    return () => window.clearTimeout(id)
  }, [value, delay])

  return value === ('' as unknown as T) ? value : state.settled
}
