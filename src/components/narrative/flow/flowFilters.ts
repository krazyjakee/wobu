import type { NarrativeFilter } from '../../../store/ui'
import type { FlowElement } from './model'

export function isFlowElementMuted(
  element: FlowElement | undefined,
  blocking: boolean,
  participant: string | null,
  statuses: Record<NarrativeFilter, boolean>,
): boolean {
  /*
   * A filter may never hide a release-blocking diagnostic.
   *
   * #187 says this in one line and it is the whole reason `blocking` is on the
   * node: filtering to "needs review" while a scene has no way into it would
   * otherwise dim the one box that stops the story shipping, and the writer
   * would be looking at a clean canvas that is not clean.
   */
  if (blocking) return false
  if (participant !== null) {
    const people =
      element?.kind === 'beat' || element?.kind === 'scene' ? element.participants : null
    if (!people?.includes(participant)) return true
  }
  const wanted = (Object.keys(statuses) as NarrativeFilter[]).filter((key) => statuses[key])
  if (wanted.length === 0) return false
  // Work dimensions are independent: a beat may need wording and review at once.
  if (element?.counts) {
    return !wanted.some(
      (filter) =>
        element.counts![filter] > 0 ||
        (filter === 'needsText' && element.kind === 'beat' && element.lines === 0),
    )
  }
  // Demonstration/legacy display elements may carry only the summary chip.
  return !element?.status || !wanted.includes(element.status as NarrativeFilter)
}
