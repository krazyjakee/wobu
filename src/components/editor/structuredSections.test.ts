import { describe, expect, it } from 'vitest'
import type { SectionDef } from '../../lib/api'
import { orderedSectionDefinitions, sectionValuesEqual } from './structuredSections'

const definitions: SectionDef[] = [
  { key: 'silhouette', label: 'Silhouette', valueKind: 'text' },
  { key: 'palette', label: 'Palette', valueKind: 'list' },
]

describe('structured section primitives', () => {
  it('keeps registry order and appends each extension section once in encounter order', () => {
    expect(
      orderedSectionDefinitions(
        definitions,
        {
          legacy_note: { type: 'text', value: 'Old canon' },
          palette: { type: 'list', value: ['#101820'] },
        },
        {
          provider_tags: { type: 'list', value: ['scarred'] },
          legacy_note: { type: 'text', value: 'Proposed canon' },
        },
      ),
    ).toEqual([
      ...definitions,
      { key: 'legacy_note', label: 'legacy note', valueKind: 'text' },
      { key: 'provider_tags', label: 'provider tags', valueKind: 'list' },
    ])
  })

  it('compares missing, text, and ordered list values structurally', () => {
    expect(sectionValuesEqual(undefined, undefined)).toBe(true)
    expect(
      sectionValuesEqual(
        { type: 'text', value: 'Forward-canted' },
        { type: 'text', value: 'Forward-canted' },
      ),
    ).toBe(true)
    expect(
      sectionValuesEqual(
        { type: 'list', value: ['ash', 'ember'] },
        { type: 'list', value: ['ember', 'ash'] },
      ),
    ).toBe(false)
    expect(
      sectionValuesEqual({ type: 'text', value: 'ash' }, { type: 'list', value: ['ash'] }),
    ).toBe(false)
  })
})
