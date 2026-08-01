import { describe, expect, it } from 'vitest'
import {
  colorFor,
  indexKinds,
  labelFor,
  layerColor,
  layerLabel,
  pluralFor,
  spriteFor,
} from './kinds'
import type { InfluenceLayer } from './api'
import { kindDef } from '../test/fixtures'

/*
 * Everything here is about one property: the backend owns the kind registry and
 * hands the frontend two opaque strings, `icon` and `color`. Neither is
 * validated on the Rust side, so every function below has to turn arbitrary
 * text into something that renders, and never into a blank square.
 */

describe('spriteFor', () => {
  it('passes through a name that is already a sprite', () => {
    expect(spriteFor(kindDef('species', { icon: 'creature' }), 'species')).toBe('creature')
  })

  it('normalises case, whitespace and an i- prefix', () => {
    expect(spriteFor(kindDef('species', { icon: '  I-Creature ' }), 'species')).toBe('creature')
  })

  it('resolves an alias', () => {
    expect(spriteFor(kindDef('character', { icon: 'person' }), 'character')).toBe('character')
  })

  it('falls back to the kind when the icon is unknown', () => {
    expect(spriteFor(kindDef('vehicle', { icon: 'nonsense-from-a-plugin' }), 'vehicle')).toBe(
      'vehicle',
    )
  })

  it('falls back to the kind when there is no def at all', () => {
    expect(spriteFor(undefined, 'environment')).toBe('env')
  })

  it('lands on a generic sprite for a kind it has never heard of', () => {
    // The last line of defence: an unrecognised kind still gets a row with an
    // icon, rather than an empty box the user cannot identify.
    expect(spriteFor(undefined, 'not_a_kind' as never)).toBe('cube')
  })
})

describe('colorFor', () => {
  it('passes a hex value through untouched', () => {
    expect(colorFor(kindDef('species', { color: '#ff8800' }), 'species')).toBe('#ff8800')
  })

  it.each(['rgb(1 2 3)', 'hsl(200 50% 40%)', 'var(--custom)'])('passes %s through', (c) => {
    expect(colorFor(kindDef('species', { color: c }), 'species')).toBe(c)
  })

  it('turns a bare layer name into a token with a fallback', () => {
    // The fallback is the point: a `--l-*` token this build does not define
    // would otherwise resolve to nothing and paint the row invisible.
    expect(colorFor(kindDef('species', { color: 'species' }), 'species')).toBe(
      'var(--l-species, var(--l-species))',
    )
  })

  it('converts underscores in a bare name to the token spelling', () => {
    expect(colorFor(kindDef('style_guide', { color: 'style_guide' }), 'style_guide')).toBe(
      'var(--l-style-guide, var(--l-style))',
    )
  })

  it('uses the layer when no colour was given', () => {
    expect(colorFor(kindDef('prop', { color: '  ', layer: 'culture' }), 'prop')).toBe(
      'var(--l-culture)',
    )
  })

  it('falls back by kind when there is no def', () => {
    expect(colorFor(undefined, 'world_bible')).toBe('var(--l-world)')
    expect(colorFor(undefined, 'environment')).toBe('var(--l-place)')
    expect(colorFor(undefined, 'vehicle')).toBe('var(--l-subject)')
  })
})

const layers: InfluenceLayer[] = [
  'style',
  'world',
  'ancestry',
  'culture',
  'place',
  'subject',
  'shot',
]

describe('layerColor', () => {
  it('covers every layer — the switch has no default, so a new one is a type error', () => {
    for (const l of layers) expect(layerColor(l)).toMatch(/^var\(--/)
    expect(new Set(layers.map(layerColor)).size).toBe(layers.length)
  })
})

describe('layerLabel', () => {
  it('names every layer distinctly, since it is what carries attribution without colour', () => {
    // The compiled prompt box credits each span to a layer in words as well as
    // in tint, for readers who cannot tell two of these colours apart. Two
    // layers sharing a label would make that attribution unreadable.
    for (const l of layers) expect(layerLabel(l)).toMatch(/^[A-Z]/)
    expect(new Set(layers.map(layerLabel)).size).toBe(layers.length)
  })

  it('spells them the way wobu-core does, so two surfaces cannot disagree', () => {
    expect(layerLabel('ancestry')).toBe('Ancestry')
    expect(layerLabel('shot')).toBe('Shot')
  })
})

describe('labelFor / pluralFor', () => {
  it('prefers the registry', () => {
    const def = kindDef('style_guide', { label: 'Style Guide', plural: 'Style Guides' })
    expect(labelFor(def, 'style_guide')).toBe('Style Guide')
    expect(pluralFor(def, 'style_guide')).toBe('Style Guides')
  })

  it('makes something readable out of the kind alone', () => {
    expect(labelFor(undefined, 'world_bible')).toBe('world bible')
    expect(pluralFor(undefined, 'world_bible')).toBe('world bibles')
  })

  it('pluralises a registry label when only the plural is missing', () => {
    const def = { ...kindDef('species', { label: 'Species' }), plural: undefined as never }
    expect(pluralFor(def, 'species')).toBe('Speciess')
  })
})

describe('indexKinds', () => {
  it('handles undefined, which is what a pending query hands it', () => {
    expect(indexKinds(undefined).size).toBe(0)
  })

  it('keys by kind', () => {
    const m = indexKinds([kindDef('species'), kindDef('character')])
    expect(m.get('species')?.kind).toBe('species')
    expect(m.size).toBe(2)
  })
})
