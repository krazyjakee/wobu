import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { NodeKind } from '../lib/api'
import { spriteFor } from '../lib/kinds'
import { kindDef } from '../test/fixtures'
import { Icon, type IconSize } from './Icon'
import { IconSprite } from './IconSprite'

/**
 * The icon system's two contracts (#128).
 *
 * Both of these were broken, and neither was visible from a render test: the
 * icons *appeared*, they were simply cropped and drawn at three different
 * weights, and two pairs of them were the same picture under different names.
 */

const SIZES: IconSize[] = ['sm', 'md', 'xl']
const KINDS: NodeKind[] = [
  'style_guide',
  'world_bible',
  'species',
  'culture',
  'setting',
  'character',
  'creature',
  'prop',
  'environment',
  'vehicle',
]

function spriteIds(): Map<string, string> {
  const { container } = render(<IconSprite />)
  const drawings = new Map<string, string>()
  for (const group of container.querySelectorAll('g[id]')) {
    drawings.set(group.id, group.innerHTML.replace(/\s+/g, ' ').trim())
  }
  return drawings
}

describe('an icon in its box', () => {
  it.each(SIZES)('maps the 24-unit grid onto the %s box', (size) => {
    // Without this the sprite's own coordinates were CSS pixels, so a 24-unit
    // glyph was drawn at 150% into a 16px viewport and cropped to its top-left
    // corner — "too large for their containers and most are visually cut off".
    const { container } = render(<Icon name="world" size={size} />)
    expect(container.querySelector('svg')).toHaveAttribute('viewBox', '0 0 24 24')
  })

  it('draws the same stroke weight at every size', () => {
    // `stroke-width` is in user units, so one number across three box sizes is
    // three different weights on screen. These are the numbers that make the
    // *rendered* weight identical: box × width ÷ 24 is constant.
    const rendered = SIZES.map((size) => {
      const { container } = render(<Icon name="world" size={size} />)
      const svg = container.querySelector('svg') as SVGElement
      const width = parseFloat(svg.style.strokeWidth)
      const box = { sm: 14, md: 16, xl: 38 }[size]
      return Math.round(((width * box) / 24) * 100) / 100
    })
    expect(new Set(rendered).size).toBe(1)
  })

  it('lets a caller override the box without the stroke fighting the class', () => {
    const { container } = render(<Icon name="chev" style={{ color: 'red' }} />)
    const svg = container.querySelector('svg') as SVGElement
    expect(svg.style.strokeWidth).not.toBe('')
    expect(svg.style.color).toBe('red')
  })

  it('stays out of the accessible tree, because the control around it is named', () => {
    const { container } = render(<Icon name="x" />)
    expect(container.querySelector('svg')).toHaveAttribute('aria-hidden')
  })
})

describe('the sprite', () => {
  it('gives every concept its own drawing', () => {
    // `i-assets` was byte-identical to `i-image` and `i-prop` to `i-cube`, so
    // the Assets mode was a single image asset and an entity kind the frontend
    // had never heard of was a prop.
    const drawings = spriteIds()
    const seen = new Map<string, string>()
    const clashes: string[] = []
    for (const [id, drawing] of drawings) {
      const first = seen.get(drawing)
      if (first) clashes.push(`${id} draws the same shape as ${first}`)
      else seen.set(drawing, id)
    }
    expect(clashes).toEqual([])
  })

  it('holds every glyph the kind registry can ask for', () => {
    // `lib/kinds.ts` resolves the backend's opaque icon string to an id here.
    // A kind whose sprite is missing renders an empty <use> and no error.
    const drawings = spriteIds()
    const asked = new Set<string>()
    for (const kind of KINDS) {
      asked.add(spriteFor(undefined, kind))
      for (const alias of ['map-pin', 'globe', 'paw', 'palette', 'box', 'car', 'tree', 'nonsense'])
        asked.add(spriteFor(kindDef(kind, { icon: alias }), kind))
    }
    for (const name of asked) expect(drawings.has(`i-${name}`), `i-${name} is missing`).toBe(true)
  })

  it('leaves colour and weight to the stylesheet', () => {
    // A glyph that sets its own `stroke-width` cannot follow the size it is
    // drawn at, and one that sets a colour cannot follow the theme — which is
    // the same failure `ThemeContract.test.ts` guards in the stylesheets.
    const { container } = render(<IconSprite />)
    const declared: string[] = []
    for (const group of container.querySelectorAll('g[id]')) {
      expect(group.id.startsWith('i-'), `${group.id} should be named i-…`).toBe(true)
      for (const shape of group.querySelectorAll('*')) {
        for (const name of ['fill', 'stroke', 'stroke-width', 'style', 'color', 'opacity']) {
          if (shape.hasAttribute(name)) declared.push(`${group.id} sets ${name}`)
        }
      }
    }
    expect(declared).toEqual([])
  })
})
