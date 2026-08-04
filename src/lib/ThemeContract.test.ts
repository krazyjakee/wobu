import { describe, expect, it } from 'vitest'

/*
 * The palette's two obligations, checked instead of promised (#134).
 *
 * 1. The duplication contract. Three files hold copies of the brand hexes
 *    because they cannot read `tokens.css`: the SVG masters, the boot screen in
 *    `index.html`, and the installer gradient in `scripts/generate-icons.sh`.
 *    Nothing stops those copies drifting except this file, which reads all of
 *    them and says which one is wrong.
 *
 * 2. Contrast. A light theme nobody can read is worse than no light theme, and
 *    "looks fine on my monitor" is not a measurement. Every text role is
 *    checked against every surface in both themes, and the six influence
 *    colours are checked for separation under normal, protanopic, deuteranopic
 *    and tritanopic vision — they are how the Inspector says which layer a
 *    fragment came from.
 *
 * The stylesheet is read as text rather than mounted: jsdom does not implement
 * `color-mix()`, `getComputedStyle` on a custom property hands back the
 * unresolved token, and a check that needed a browser would not run in CI.
 */

/*
 * Reading the files.
 *
 * Not `?raw` imports: Vitest runs with CSS processing off, so a `.css` import —
 * raw, inline or otherwise — comes back as an empty string, and a contract test
 * that silently reads nothing is worse than no contract test. The `node:fs`
 * specifier is assembled at runtime because this repo carries no `@types/node`
 * and the literal would not compile; the cast is what that import would have
 * declared.
 */
const fs = (await import('node:' + 'fs')) as {
  readFileSync(path: string, encoding: 'utf8'): string
  readdirSync(path: string): string[]
}
const ROOT = (globalThis as { process?: { cwd(): string } }).process?.cwd() ?? '.'
const read = (path: string) => fs.readFileSync(`${ROOT}/${path}`, 'utf8')

const TOKENS = read('src/styles/tokens.css')
const INDEX_HTML = read('index.html')
const ICON_SVG = read('branding/wobu-icon.svg')
const MARK_SVG = read('branding/wobu-mark.svg')
const ICON_SCRIPT = read('scripts/generate-icons.sh')

/* ── reading the stylesheet ───────────────────────────────────────────────── */

const stripComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '')

function declarations(css: string, selector: string): Record<string, string> {
  const start = css.indexOf(selector)
  if (start < 0) throw new Error(`tokens.css has no ${selector} block`)
  const open = css.indexOf('{', start)
  const close = css.indexOf('}', open)
  const out: Record<string, string> = {}
  for (const line of css.slice(open + 1, close).split(';')) {
    const [name, ...rest] = line.split(':')
    if (!name?.trim().startsWith('--')) continue
    out[name.trim()] = rest.join(':').trim()
  }
  return out
}

/** Flatten `var(--x)` indirection so a role resolves to the hex it paints. */
function resolve(vars: Record<string, string>): Record<string, string> {
  const out: Record<string, string> = {}
  for (const key of Object.keys(vars)) {
    let value = vars[key] ?? ''
    for (let hop = 0; hop < 8; hop++) {
      const match = /^var\((--[a-z0-9-]+)\)$/.exec(value)
      if (!match) break
      value = vars[match[1] ?? ''] ?? value
    }
    out[key] = value
  }
  return out
}

const stripped = stripComments(TOKENS)
const root = declarations(stripped, ':root {')
const dark = resolve(root)
const light = resolve({ ...root, ...declarations(stripped, ":root[data-theme='light']") })
const THEMES = { dark, light }

/** A token's value, or a failure that names the token rather than `undefined`. */
function token(theme: Record<string, string>, name: string): string {
  const value = theme[name]
  if (!value) throw new Error(`tokens.css has no ${name}`)
  return value
}

/* ── colour maths ─────────────────────────────────────────────────────────── */

type Rgb = [number, number, number]

function rgb(hex: string): Rgb {
  const value = hex.trim().replace('#', '')
  const full = value.length === 3 ? [...value].map((c) => c + c).join('') : value
  if (!/^[0-9a-f]{6}$/i.test(full)) throw new Error(`not a hex colour: ${hex}`)
  return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16)) as Rgb
}

function toLinear(channel: number): number {
  const c = channel / 255
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
}

function linearRgb(hex: string): Rgb {
  const [r, g, b] = rgb(hex)
  return [toLinear(r), toLinear(g), toLinear(b)]
}

function luminance(hex: string): number {
  const [r, g, b] = linearRgb(hex)
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

/** WCAG 2.x contrast ratio, rounded the way the issue comment quotes it. */
function contrast(a: string, b: string): number {
  const first = luminance(a)
  const second = luminance(b)
  const hi = Math.max(first, second)
  const lo = Math.min(first, second)
  return Math.round(((hi + 0.05) / (lo + 0.05)) * 100) / 100
}

function toLab(hex: string): [number, number, number] {
  const [r, g, b] = linearRgb(hex)
  const x = (r * 0.4124564 + g * 0.3575761 + b * 0.1804375) / 0.95047
  const y = r * 0.2126729 + g * 0.7151522 + b * 0.072175
  const z = (r * 0.0193339 + g * 0.119192 + b * 0.9503041) / 1.08883
  const f = (t: number) => (t > 216 / 24389 ? Math.cbrt(t) : (841 / 108) * t + 4 / 29)
  return [116 * f(y) - 16, 500 * (f(x) - f(y)), 200 * (f(y) - f(z))]
}

/**
 * CIEDE2000.
 *
 * Plain Euclidean distance in Lab overstates how different two saturated
 * colours look, which is exactly the mistake a palette check must not make: it
 * would pass a pair of dots nobody can tell apart.
 */
function deltaE(hexA: string, hexB: string): number {
  const [l1, a1, b1] = toLab(hexA)
  const [l2, a2, b2] = toLab(hexB)
  const rad = Math.PI / 180
  const deg = 180 / Math.PI
  const cBar = (Math.hypot(a1, b1) + Math.hypot(a2, b2)) / 2
  const g = 0.5 * (1 - Math.sqrt(cBar ** 7 / (cBar ** 7 + 25 ** 7)))
  const ap1 = (1 + g) * a1
  const ap2 = (1 + g) * a2
  const cp1 = Math.hypot(ap1, b1)
  const cp2 = Math.hypot(ap2, b2)
  const angle = (y: number, x: number) =>
    y === 0 && x === 0 ? 0 : (Math.atan2(y, x) * deg + 360) % 360
  const hp1 = angle(b1, ap1)
  const hp2 = angle(b2, ap2)
  const dLp = l2 - l1
  const dCp = cp2 - cp1
  let dhp = 0
  if (cp1 * cp2 !== 0) {
    dhp = hp2 - hp1
    if (dhp > 180) dhp -= 360
    else if (dhp < -180) dhp += 360
  }
  const dHp = 2 * Math.sqrt(cp1 * cp2) * Math.sin((dhp / 2) * rad)
  const lBar = (l1 + l2) / 2
  const cpBar = (cp1 + cp2) / 2
  let hBar = hp1 + hp2
  if (cp1 * cp2 !== 0) {
    hBar = (hp1 + hp2) / 2
    if (Math.abs(hp1 - hp2) > 180) hBar += hp1 + hp2 < 360 ? 180 : -180
  }
  const t =
    1 -
    0.17 * Math.cos((hBar - 30) * rad) +
    0.24 * Math.cos(2 * hBar * rad) +
    0.32 * Math.cos((3 * hBar + 6) * rad) -
    0.2 * Math.cos((4 * hBar - 63) * rad)
  const sl = 1 + (0.015 * (lBar - 50) ** 2) / Math.sqrt(20 + (lBar - 50) ** 2)
  const sc = 1 + 0.045 * cpBar
  const sh = 1 + 0.015 * cpBar * t
  const rt =
    -2 *
    Math.sqrt(cpBar ** 7 / (cpBar ** 7 + 25 ** 7)) *
    Math.sin(60 * Math.exp(-(((hBar - 275) / 25) ** 2)) * rad)
  return Math.sqrt(
    (dLp / sl) ** 2 + (dCp / sc) ** 2 + (dHp / sh) ** 2 + rt * (dCp / sc) * (dHp / sh),
  )
}

/**
 * Dichromat simulation (Viénot, Brettel & Mollon), in linear light.
 *
 * Roughly one man in twelve sees one of the first two. A palette that carries
 * meaning in six colours has to survive that, or the layer dots become six of
 * the same dot.
 */
const CVD_MATRICES = {
  protan: [
    [0.152286, 1.052583, -0.204868],
    [0.114503, 0.786281, 0.099216],
    [-0.003882, -0.048116, 1.051998],
  ],
  deutan: [
    [0.367322, 0.860646, -0.227968],
    [0.280085, 0.672501, 0.047413],
    [-0.01182, 0.04294, 0.968881],
  ],
  tritan: [
    [1.255528, -0.076749, -0.178779],
    [-0.078411, 0.930809, 0.147602],
    [0.004733, 0.691367, 0.3039],
  ],
} as const satisfies Record<string, readonly Rgb[]>

type Vision = 'normal' | keyof typeof CVD_MATRICES

function simulate(hex: string, vision: Vision): string {
  if (vision === 'normal') return hex
  const [r, g, b] = linearRgb(hex)
  const encode = (channel: number) => {
    const c = Math.min(1, Math.max(0, channel))
    const srgb = c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055
    return Math.round(srgb * 255)
      .toString(16)
      .padStart(2, '0')
  }
  const rows = CVD_MATRICES[vision]
  return '#' + rows.map((row) => encode(row[0] * r + row[1] * g + row[2] * b)).join('')
}

/* ── the duplication contract ─────────────────────────────────────────────── */

const BRAND = ['--brand-amber', '--brand-teal', '--brand-violet']
const TILE = ['--brand-tile-hi', '--brand-tile-lo']

/** Where each brand hex is copied, and therefore what a drift report must name. */
const COPIES = [
  { file: 'branding/wobu-icon.svg', source: ICON_SVG, tokens: [...BRAND, ...TILE] },
  { file: 'branding/wobu-mark.svg', source: MARK_SVG, tokens: BRAND },
  { file: 'index.html', source: INDEX_HTML, tokens: [...BRAND, ...TILE] },
]

describe('the duplication contract', () => {
  it.each(COPIES)('$file still carries the brand hexes from tokens.css', ({ source, tokens }) => {
    for (const name of tokens) {
      const hex = token(dark, name)
      expect(hex, `${name} should be a plain hex`).toMatch(/^#[0-9a-f]{6}$/i)
      expect(source.toLowerCase(), `${name} (${hex}) has drifted`).toContain(hex.toLowerCase())
    }
  })

  it('keeps the installer gradient on the icon tile', () => {
    const gradient = `gradient:${token(dark, '--brand-tile-hi')}-${token(dark, '--brand-tile-lo')}`
    expect(ICON_SCRIPT, `scripts/generate-icons.sh should compose ${gradient}`).toContain(gradient)
  })

  it('holds the brand hues still while the themes move', () => {
    // The mark is identity, not decoration: the same artwork in a light dock and
    // a dark one. If a brand hue ever needed to differ per theme, all three
    // copies above would silently be wrong for one of them.
    for (const name of [...BRAND, ...TILE]) expect(token(light, name)).toBe(token(dark, name))
  })

  it('gives the boot screen the same neutrals the bundle will paint', () => {
    // index.html cannot import tokens.css — it is the screen that covers the
    // wait for the bundle — so it declares its own copies. Anything that drifts
    // here shows up as a colour jump on every launch.
    const declared = new Set(
      [...INDEX_HTML.matchAll(/--wb-[a-z]+:\s*(#[0-9a-f]{3,8})/gi)].map((m) =>
        (m[1] ?? '').toLowerCase(),
      ),
    )
    for (const [name, theme] of Object.entries(THEMES)) {
      for (const key of ['--bg', '--bg-panel', '--border', '--text', '--text-faint']) {
        expect(declared, `${key} in ${name}`).toContain(token(theme, key).toLowerCase())
      }
    }
  })
})

/* ── contrast ─────────────────────────────────────────────────────────────── */

const SURFACES = ['--bg', '--bg-panel', '--bg-raised', '--bg-input']
/** Roles that are set as text on an app surface somewhere. WCAG AA is 4.5:1. */
const TEXT_ROLES = [
  '--text',
  '--text-dim',
  '--text-faint',
  '--accent',
  '--ai',
  '--ai-text',
  '--danger-text',
  '--ok',
  '--l-style',
  '--l-world',
  '--l-species',
  '--l-culture',
  '--l-place',
  '--l-subject',
]

describe.each(Object.entries(THEMES))('the %s theme', (_name, theme) => {
  it.each(TEXT_ROLES)('sets %s at AA on every surface', (role) => {
    for (const surface of SURFACES) {
      const ratio = contrast(token(theme, role), token(theme, surface))
      expect(ratio, `${role} on ${surface}`).toBeGreaterThanOrEqual(4.5)
    }
  })

  it('keeps the ink on a filled control legible', () => {
    expect(contrast(token(theme, '--on-accent'), token(theme, '--accent-fill'))).toBeGreaterThan(
      4.5,
    )
    expect(contrast(token(theme, '--on-danger'), token(theme, '--danger'))).toBeGreaterThan(4.5)
  })

  it('keeps the text over artwork legible, whichever theme is around it', () => {
    // Scrims and the 3D letterbox stay dark in both themes, because a concept
    // image does not change with the desktop — so the text on them must not
    // come from a role that does.
    for (const role of ['--on-scrim', '--on-scrim-dim', '--accent-on-scrim', '--ai-on-scrim']) {
      expect(contrast(token(theme, role), token(theme, '--media-bg')), role).toBeGreaterThan(4.5)
    }
  })

  it('draws a control boundary at 3:1', () => {
    // WCAG 1.4.11 is about the edge of a control, which is `--border-str`.
    // `--border` is the seam between two panels and is deliberately quieter: no
    // control depends on it alone to be found.
    for (const surface of SURFACES) {
      const ratio = contrast(token(theme, '--border-str'), token(theme, surface))
      expect(ratio, `--border-str on ${surface}`).toBeGreaterThanOrEqual(3)
    }
  })
})

/* ── influence layers ─────────────────────────────────────────────────────── */

const LAYERS = ['--l-style', '--l-world', '--l-species', '--l-culture', '--l-place', '--l-subject']

/**
 * The floor for "those are two colours, not one".
 *
 * ΔE00 of 2 is the just-noticeable difference between two flat patches side by
 * side; these are 6px dots, seen minutes apart, and named from memory.
 * Tritanopia gets a lower bar: it is orders of magnitude rarer, and buying
 * separation there costs it in the two kinds that are not.
 */
const SEPARATION: Record<Vision, number> = { normal: 12, protan: 8, deutan: 8, tritan: 6 }

describe.each(Object.entries(THEMES))('the %s influence layers', (_name, theme) => {
  it.each(Object.keys(SEPARATION) as Vision[])('stay apart under %s vision', (vision) => {
    for (let i = 0; i < LAYERS.length; i++) {
      for (let j = i + 1; j < LAYERS.length; j++) {
        const a = LAYERS[i] ?? ''
        const b = LAYERS[j] ?? ''
        const distance = deltaE(
          simulate(token(theme, a), vision),
          simulate(token(theme, b), vision),
        )
        expect(distance, `${a} vs ${b}`).toBeGreaterThanOrEqual(SEPARATION[vision])
      }
    }
  })
})

/* ── no colour outside the palette ────────────────────────────────────────── */

describe('the stylesheets', () => {
  it('name a token instead of a colour', () => {
    // A hex in a rule is a value that cannot follow the theme, and every one of
    // them was a light-mode bug before this issue. Pure black and white are
    // allowed as the far end of a `color-mix()`, where they mean "darker" and
    // "lighter" rather than a colour of their own.
    const offenders: string[] = []
    for (const file of fs.readdirSync(`${ROOT}/src/styles`)) {
      if (!file.endsWith('.css') || file === 'tokens.css') continue
      const css = stripComments(read(`src/styles/${file}`))
      for (const match of css.matchAll(/#[0-9a-f]{3,8}\b/gi)) {
        if (!/^#(000|fff|000000|ffffff)$/i.test(match[0])) offenders.push(`${file}: ${match[0]}`)
      }
    }
    expect(offenders).toEqual([])
  })
})
