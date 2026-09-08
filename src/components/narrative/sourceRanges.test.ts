import { describe, expect, it } from 'vitest'
import { sourceRange, syntaxOffset } from './sourceRanges'

describe('source ranges from typed diagnostic paths', () => {
  it('selects the exact flow-style value with quoted keys and Unicode before it', () => {
    const yaml =
      '# 🌋 notes\n"scene": {name: "港 🌟", beats: [{id: first, choices: [{to: {beat: missing}}]}]}\n'
    const range = sourceRange(yaml)(['scene', 'beats', 0, 'choices', 0, 'to', 'beat'])!
    expect(yaml.slice(range.start, range.end)).toBe('missing')
    expect(range.line).toBe(2)
    expect(range.start).toBe(yaml.indexOf('missing'))
  })
  it('uses list position to identify repeated IDs and excludes adjacent comments', () => {
    const yaml = 'scene:\n  beats:\n    - id: repeated\n    - id: repeated # duplicate\n'
    const range = sourceRange(yaml)(['scene', 'beats', 1, 'id'])!
    expect(range.start).toBe(yaml.lastIndexOf('repeated'))
    expect(yaml.slice(range.start, range.end)).toBe('repeated')
  })
  it('selects a block scalar without treating words inside prose as fields', () => {
    const yaml = 'scene:\n  summary: |\n    scene: not a key\n    星 🌟\n  name: Real\n'
    const range = sourceRange(yaml)(['scene', 'summary'])!
    expect(yaml.slice(range.start, range.end)).toBe('|\n    scene: not a key\n    星 🌟\n')
  })
  it('selects the authored alias without expanding recursive aliases or pointing at another branch', () => {
    const yaml =
      'scene:\n  entry: &gate {all: [*gate]}\n  beats: [{choices: [{requires: *gate}]}]\n'
    const range = sourceRange(yaml)(['scene', 'beats', 0, 'choices', 0, 'requires', 'all', 0])!
    expect(yaml.slice(range.start, range.end)).toBe('*gate')
    expect(range.start).toBe(yaml.lastIndexOf('*gate'))
  })
  it('locates payloads in legacy tagged enums without consuming matching payload keys', () => {
    const yaml = 'scene: {entry: !compare {var: missing, op: eq, value: !var other}}\n'
    const locate = sourceRange(yaml)
    const range = locate(['scene', 'entry', 'compare', 'var'])!
    expect(yaml.slice(range.start, range.end)).toBe('missing')
    const other = locate(['scene', 'entry', 'compare', 'value', 'var'])!
    expect(yaml.slice(other.start, other.end)).toBe('other')
  })
  it('converts Unicode scalar columns into textarea offsets including astral characters', () => {
    expect(syntaxOffset('header\n🌋é field', 2, 4)).toBe('header\n🌋é '.length)
  })
})
