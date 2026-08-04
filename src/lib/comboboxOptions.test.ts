import { describe, expect, it } from 'vitest'
import { filterPrepared, foldText, prepareOptions, sortPrepared, titleKey } from './comboboxOptions'

const labels = (entries: { option: { label: string } }[]) => entries.map((e) => e.option.label)
const sorted = (names: string[]) =>
  labels(sortPrepared(prepareOptions(names.map((label) => ({ label })))))
const filtered = (names: string[], query: string) =>
  labels(filterPrepared(prepareOptions(names.map((label) => ({ label }))), query))

describe('folding', () => {
  it('strips case and diacritics so accented titles sort with their letter', () => {
    expect(foldText('Élan Vitál')).toBe('elan vital')
    expect(foldText('ÅNGSTRÖM')).toBe('angstrom')
  })

  it('drops a leading article, and only when it is a whole word', () => {
    expect(titleKey('The Ashen Gate')).toBe('ashen gate')
    expect(titleKey('A Sword')).toBe('sword')
    expect(titleKey('An Oath')).toBe('oath')
    // "Theodore" and "Answer" begin with the same letters and are not articles.
    expect(titleKey('Theodore')).toBe('theodore')
    expect(titleKey('Answer')).toBe('answer')
  })
})

describe('title sorting', () => {
  it('files a title under its first real word', () => {
    expect(sorted(['The Ashen Gate', 'Baker', 'An Oath', 'Zephyr'])).toEqual([
      'The Ashen Gate',
      'Baker',
      'An Oath',
      'Zephyr',
    ])
  })

  it('sorts accented titles among their unaccented neighbours', () => {
    expect(sorted(['Zephyr', 'Élan', 'Emberly', 'edda'])).toEqual([
      'edda',
      'Élan',
      'Emberly',
      'Zephyr',
    ])
  })

  it('gives colliding sort keys one fixed order rather than the input order', () => {
    expect(sorted(['The Gate', 'Gate'])).toEqual(sorted(['Gate', 'The Gate']))
  })

  it('keeps identical labels in the order they arrived', () => {
    const options = [
      { label: 'Kael', keywords: 'first' },
      { label: 'Kael', keywords: 'second' },
    ]
    expect(sortPrepared(prepareOptions(options)).map((e) => e.option.keywords)).toEqual([
      'first',
      'second',
    ])
  })
})

describe('filtering', () => {
  it('returns everything for an empty query', () => {
    expect(filtered(['Kael', 'Mira'], '   ')).toEqual(['Kael', 'Mira'])
  })

  it('ranks prefix matches above substring matches', () => {
    expect(filtered(['Broken Kaelstone', 'Kael', 'Unkaelable'], 'kae')).toEqual([
      'Kael',
      'Broken Kaelstone',
      'Unkaelable',
    ])
  })

  it('treats a title whose article was stripped as a prefix match', () => {
    // "The Ashen Gate" does not start with "ash" — its title, once the article
    // is dropped, does. That has to beat a plain substring hit.
    expect(filtered(['Flashing Blade', 'The Ashen Gate'], 'ash')).toEqual([
      'The Ashen Gate',
      'Flashing Blade',
    ])
  })

  it('ignores case and diacritics on both sides', () => {
    expect(filtered(['Élan'], 'ELAN')).toEqual(['Élan'])
    expect(filtered(['Elan'], 'élan')).toEqual(['Elan'])
  })

  it('keeps the incoming order within a tier', () => {
    expect(filtered(['Kaelstone', 'Kaelen', 'Kael'], 'kael')).toEqual([
      'Kaelstone',
      'Kaelen',
      'Kael',
    ])
  })

  it('matches hidden keywords last, and drops what matches nothing', () => {
    const options = [
      { label: 'Harbour', keywords: 'setting place' },
      { label: 'Settle', keywords: '' },
      { label: 'Kael', keywords: 'character' },
    ]
    expect(labels(filterPrepared(prepareOptions(options), 'sett'))).toEqual(['Settle', 'Harbour'])
  })

  it('filters a few thousand options in one pass over the prepared list', () => {
    const options = Array.from({ length: 4000 }, (_, i) => ({ label: `Node ${i}` }))
    const prepared = prepareOptions(options)
    const started = performance.now()
    for (let i = 0; i < 20; i += 1) filterPrepared(prepared, 'node 12')
    // Twenty keystrokes over four thousand rows. The bound is loose on purpose —
    // it is here to catch a per-keystroke re-fold or an accidental O(n²), not to
    // measure the machine.
    expect(performance.now() - started).toBeLessThan(1000)
    expect(filterPrepared(prepared, 'node 123')).toHaveLength(11)
  })
})
