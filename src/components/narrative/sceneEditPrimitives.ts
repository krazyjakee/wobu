import type { Beat } from '../../lib/api'

export function duplicateBeat(beat: Beat, mintId: () => string): Beat {
  const copy = structuredClone(beat)
  copy.id = mintId()
  copy.title = `${beat.title} (copy)`
  for (const slot of copy.dialogue ?? []) {
    slot.id = mintId()
    for (const variant of slot.variants ?? []) {
      variant.id = mintId()
      variant.text.lifecycle = {
        ...variant.text.lifecycle,
        review: 'draft',
        policy: variant.text.lifecycle?.policy === 'locked' ? 'locked' : 'edited',
      }
    }
  }
  for (const exit of [...(copy.choices ?? []), ...(copy.outcomes ?? [])]) {
    exit.id = mintId()
  }
  return copy
}

export function beatHasLockedText(beat: Beat): boolean {
  return (beat.dialogue ?? []).some(
    (slot) =>
      slot.policy === 'locked' ||
      slot.variants?.some((variant) => variant.text.lifecycle?.policy === 'locked'),
  )
}
