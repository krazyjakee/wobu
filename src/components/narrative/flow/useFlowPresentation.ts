import { useCallback, useEffect, useRef, useState } from 'react'
import type { GraphKey, Layout } from '../../../lib/api'
import { graphKeyId } from '../../../lib/api'
import { useSaveLayout, useSceneLayout } from '../../../lib/queries'

export interface FlowPresentation {
  layout: Layout
  onChange: (layout: Layout) => void
  saveFailed?: boolean
}

/** A presentation draft survives a refused cosmetic save; source never enters
 * this queue. Serialize drags so delayed acknowledgements cannot roll back UI. */
export function useFlowPresentation(graph: GraphKey) {
  const stored = useSceneLayout(graph)
  const mutation = useSaveLayout()
  const [draft, setDraft] = useState<Layout | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const latest = useRef<Layout | null>(null)
  const running = useRef(false)
  const mounted = useRef(true)
  const key = graphKeyId(graph)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      latest.current = null
    }
  }, [])
  const [seenKey, setSeenKey] = useState(key)
  if (seenKey !== key) {
    setSeenKey(key)
    setDraft(null)
  }
  useEffect(() => {
    latest.current = null
  }, [key])
  const save = mutation.mutateAsync
  const onChange = useCallback(
    (layout: Layout) => {
      const next = { ...layout, schemaVersion: 2 }
      setFailure(null)
      setDraft(next)
      latest.current = next
      if (running.current) return
      running.current = true
      void (async () => {
        try {
          while (latest.current) {
            const pending = latest.current
            latest.current = null
            try {
              const outcome = await save(pending)
              if (outcome.outcome === 'written' && !latest.current && mounted.current) {
                setDraft(null)
                setFailure(null)
              }
            } catch (error) {
              if (mounted.current) setFailure(String(error))
            }
          }
        } finally {
          running.current = false
        }
      })()
    },
    [save],
  )
  const layout = draft ?? stored.data?.layout
  return {
    stored,
    outcome: failure ? { outcome: 'unwritable' as const, reason: failure } : mutation.data,
    presentation: layout
      ? {
          layout,
          onChange,
          saveFailed:
            !!draft &&
            (!!failure || (mutation.data != null && mutation.data.outcome !== 'written')),
        }
      : undefined,
  }
}
