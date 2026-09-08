import { useEffect, useRef } from 'react'
import type { FlowPresentation } from './useFlowPresentation'
import { useFlowLevelApi } from './flowStore'

/** Canvas and outline hydrate and persist the same cosmetic group state. */
export function useFlowGroupPresentation(
  presentation: FlowPresentation | undefined,
  readOnly: boolean,
) {
  const store = useFlowLevelApi()
  const presentationRef = useRef(presentation)
  useEffect(() => {
    presentationRef.current = presentation
  }, [presentation])
  const hydratedGroups = useRef(false)
  const storedGroups = presentation?.layout.groups
  useEffect(() => {
    if (!storedGroups) return
    hydratedGroups.current = true
    store.getState().setClosedGroups(
      Object.values(storedGroups)
        .filter((g) => g.collapsed)
        .map((g) => g.id),
    )
    hydratedGroups.current = false
  }, [storedGroups, store])
  useEffect(
    () =>
      store.subscribe((state, previous) => {
        const current = presentationRef.current
        if (
          !current ||
          readOnly ||
          hydratedGroups.current ||
          state.closedGroups === previous.closedGroups
        )
          return
        const closed = new Set(state.closedGroups)
        const groups = { ...current.layout.groups }
        let changed = false
        for (const [id, group] of Object.entries(groups)) {
          if (!!group.collapsed !== closed.has(id)) {
            groups[id] = {
              ...group,
              collapsed: closed.has(id),
              updatedAt: new Date().toISOString(),
            }
            changed = true
          }
        }
        if (changed) current.onChange({ ...current.layout, groups })
      }),
    [store, readOnly],
  )
}
