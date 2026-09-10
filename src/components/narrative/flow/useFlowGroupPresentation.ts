import { useEffect, useRef } from 'react'
import type { FlowPresentation } from './useFlowPresentation'
import { useFlowLevelApi } from './flowStore'

/**
 * Canvas and outline hydrate and persist the same cosmetic group state.
 *
 * `regrouped` is the arc's case (#187). When the level is being carved up by
 * quest or by quest state, the boxes on screen are not the arrangement groups
 * the sidecar holds: their ids belong to World quests, and their closed set is
 * a way of *reading* the arc rather than a change to it. So while a derived
 * grouping is on, this stands down entirely — it does not hydrate, because the
 * stored closed set names other groups, and it does not persist, because
 * writing "open" for every arrangement group the writer is not currently
 * looking at would trade their whole collapsed arrangement for a glance at the
 * quests. Switching back re-runs the hydration and restores it.
 */
export function useFlowGroupPresentation(
  presentation: FlowPresentation | undefined,
  readOnly: boolean,
  regrouped = false,
) {
  const store = useFlowLevelApi()
  const presentationRef = useRef(presentation)
  useEffect(() => {
    presentationRef.current = presentation
  }, [presentation])
  const regroupedRef = useRef(regrouped)
  useEffect(() => {
    regroupedRef.current = regrouped
  }, [regrouped])
  const hydratedGroups = useRef(false)
  const storedGroups = presentation?.layout.groups
  useEffect(() => {
    if (!storedGroups || regrouped) return
    hydratedGroups.current = true
    store.getState().setClosedGroups(
      Object.values(storedGroups)
        .filter((g) => g.collapsed)
        .map((g) => g.id),
    )
    hydratedGroups.current = false
  }, [storedGroups, store, regrouped])
  useEffect(
    () =>
      store.subscribe((state, previous) => {
        const current = presentationRef.current
        if (
          !current ||
          readOnly ||
          regroupedRef.current ||
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
