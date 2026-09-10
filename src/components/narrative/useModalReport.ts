import { useEffect, useState } from 'react'
import { errorMessage } from '../../lib/api'

/**
 * Read a project-wide report once when a pane opens, and again when asked.
 *
 * Shared by the panes that ask a question costing a pass over the project —
 * retained deletions, repeated wording — because the awkward parts are the same
 * in each and were the same twice: not setting state after unmount, not starting
 * a second read while one is running, and replacing the error rather than
 * accumulating errors.
 *
 * `load` is called on mount and on `refresh`, and nothing else triggers it. These
 * reports are not on the path of a keystroke and must not be put there.
 */
export function useModalReport<T>(load: () => Promise<T>) {
  const [data, setData] = useState<T | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')

  useEffect(() => {
    let mounted = true
    void load()
      .then((result) => {
        if (mounted) setData(result)
      })
      .catch((reason: unknown) => {
        if (mounted) setError(errorMessage(reason))
      })
      .finally(() => {
        if (mounted) setLoading(false)
      })
    return () => {
      mounted = false
    }
    // Read once per mount. A `load` identity that changes per render would
    // otherwise re-read the whole project on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const refresh = async (busy: boolean) => {
    if (busy) return
    setLoading(true)
    setError('')
    try {
      setData(await load())
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setLoading(false)
    }
  }
  return { data, setData, loading, setLoading, error, setError, refresh }
}
