import type { SectionDef, SectionValue } from '../../lib/api'

export type SectionMap = Record<string, SectionValue>

/** Registry order first, followed by provider/file extensions in encounter order. */
export function orderedSectionDefinitions(
  definitions: SectionDef[],
  ...sectionMaps: SectionMap[]
): SectionDef[] {
  const ordered = [...definitions]
  const seen = new Set(definitions.map((definition) => definition.key))
  for (const sections of sectionMaps) {
    for (const [key, value] of Object.entries(sections)) {
      if (seen.has(key)) continue
      seen.add(key)
      ordered.push({ key, label: key.replace(/_/g, ' '), valueKind: value.type })
    }
  }
  return ordered
}

/**
 * Structural equality for both editable canon and read-only Enhance review.
 *
 * Only ordering and value comparison are shared. Their markup intentionally
 * stays consumer-owned: DescriptionEditor exposes autosaving form controls,
 * while EnhanceReview presents a read-only diff with explicit accept choices.
 */
export function sectionValuesEqual(
  left: SectionValue | undefined,
  right: SectionValue | undefined,
): boolean {
  if (!left || !right) return left === right
  if (left.type !== right.type) return false
  if (left.type === 'text' && right.type === 'text') return left.value === right.value
  if (left.type === 'list' && right.type === 'list') {
    return (
      left.value.length === right.value.length &&
      left.value.every((item, index) => item === right.value[index])
    )
  }
  return false
}
