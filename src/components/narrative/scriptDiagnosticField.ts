import type { NarrativeDiagnostic } from '../../lib/api'

/** Stable element IDs are used for navigation; diagnostics never parse display text. */
export function diagnosticField(diagnostic: NarrativeDiagnostic): string {
  if (diagnostic.choiceId)
    return `choice:${diagnostic.choiceId}:${diagnostic.destination ? 'destination' : 'condition'}`
  if (diagnostic.outcomeId)
    return `outcome:${diagnostic.outcomeId}:${diagnostic.destination ? 'destination' : 'condition'}`
  if (diagnostic.variantId) return `variant:${diagnostic.variantId}`
  if (diagnostic.slotId) return `slot:${diagnostic.slotId}`
  if (diagnostic.kind === 'entry') return 'scene:entry'
  if (diagnostic.kind === 'participant') return 'scene:participants'
  return diagnostic.beatId ? `beat:${diagnostic.beatId}` : 'scene:name'
}
