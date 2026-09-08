import type { Scene } from '../../lib/api'
import { narrativeTextWritten } from '../../lib/api'

/** Seal only changed words. Copies retain their existing provenance and revision. */
export async function prepareScriptText(before: Scene, draft: Scene): Promise<Scene> {
  const next = structuredClone(draft)
  const variants = (before.beats ?? [])
    .flatMap((beat) => beat.dialogue ?? [])
    .flatMap((slot) => slot.variants ?? [])
  const byId = new Map(variants.map((variant) => [variant.id, variant.text]))
  const byRevision = new Map(variants.map((variant) => [variant.text.revision, variant.text]))
  for (const beat of next.beats ?? []) {
    for (const slot of beat.dialogue ?? []) {
      for (const variant of slot.variants ?? []) {
        const original = byId.get(variant.id) ?? byRevision.get(variant.text.revision)
        if (original?.body === variant.text.body) continue
        const text = await narrativeTextWritten(
          variant.text.body,
          variant.text.lifecycle?.policy === 'locked',
        )
        variant.text = {
          ...text,
          lifecycle: { ...text.lifecycle, freshness: original?.lifecycle?.freshness ?? 'current' },
        }
      }
    }
  }
  return next
}
