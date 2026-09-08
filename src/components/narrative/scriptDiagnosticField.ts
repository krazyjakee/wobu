import type { NarrativeTarget } from '../../store/ui'

/** Existing form markers use stable IDs; never interpolate them into a CSS selector. */
export function narrativeField(target: NarrativeTarget): string {
  const route = target.choiceId
    ? `choice:${target.choiceId}`
    : target.outcomeId
      ? `outcome:${target.outcomeId}`
      : null
  if (route) return `${route}:${target.field ?? 'condition'}`
  if (target.variantId) return `variant:${target.variantId}`
  if (target.lineId) return `slot:${target.lineId}`
  if (
    target.field &&
    ['entry', 'participants', 'name', 'act', 'arc', 'tags'].includes(target.field)
  )
    return `scene:${target.field}`
  return target.beatId ? `beat:${target.beatId}` : 'scene:name'
}
