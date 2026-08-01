import type { InfluenceLayer, KindDef, NodeKind } from './api'

/**
 * The kind registry is owned by Rust (`kind_registry`). These maps only cover
 * the two things the backend hands us as opaque strings — an icon name and a
 * colour — so the frontend can resolve them to a sprite id and a CSS colour.
 * Nothing here invents a kind the backend did not send.
 */

/** Sprite ids that exist in <IconSprite>. */
const SPRITES = new Set([
  'style',
  'world',
  'species',
  'culture',
  'place',
  'character',
  'creature',
  'prop',
  'env',
  'vehicle',
  'cube',
  'image',
  'layers',
])

/** Fallbacks by kind, used when the backend's icon name isn't a known sprite. */
const KIND_SPRITE: Record<NodeKind, string> = {
  style_guide: 'style',
  world_bible: 'world',
  species: 'species',
  culture: 'culture',
  setting: 'place',
  character: 'character',
  creature: 'creature',
  prop: 'prop',
  environment: 'env',
  vehicle: 'vehicle',
}

/** Aliases for icon names the backend might reasonably use. */
const ALIAS: Record<string, string> = {
  'style-guide': 'style',
  styleguide: 'style',
  palette: 'style',
  globe: 'world',
  'world-bible': 'world',
  book: 'world',
  dna: 'species',
  users: 'culture',
  people: 'culture',
  group: 'culture',
  map: 'place',
  'map-pin': 'place',
  pin: 'place',
  setting: 'place',
  location: 'place',
  user: 'character',
  person: 'character',
  paw: 'creature',
  beast: 'creature',
  monster: 'creature',
  box: 'prop',
  package: 'prop',
  tree: 'env',
  environment: 'env',
  mountain: 'env',
  landscape: 'env',
  car: 'vehicle',
  ship: 'vehicle',
}

export function spriteFor(def: KindDef | undefined, kind: NodeKind): string {
  const raw = (def?.icon ?? '').trim().toLowerCase().replace(/^i-/, '')
  if (SPRITES.has(raw)) return raw
  const aliased = ALIAS[raw]
  if (aliased) return aliased
  return KIND_SPRITE[kind] ?? 'cube'
}

/**
 * The backend's `color` may be a hex value, a full CSS colour, or a bare layer
 * name (`species`) matching the `--l-*` tokens in docs/03-ui-layout.md.
 */
export function colorFor(def: KindDef | undefined, kind: NodeKind): string {
  const raw = (def?.color ?? '').trim()
  if (raw) {
    if (/^(#|rgb|hsl|var\()/i.test(raw)) return raw
    if (/^[a-z_-]+$/i.test(raw)) return `var(--l-${raw.replace(/_/g, '-')}, ${fallbackColor(kind)})`
    return raw
  }
  if (def?.layer) return layerColor(def.layer)
  return fallbackColor(kind)
}

/** Influence layer → the `--l-*` token from docs/03-ui-layout.md. */
export function layerColor(layer: InfluenceLayer): string {
  switch (layer) {
    case 'style':
      return 'var(--l-style)'
    case 'world':
      return 'var(--l-world)'
    case 'ancestry':
      return 'var(--l-species)'
    case 'culture':
      return 'var(--l-culture)'
    case 'place':
      return 'var(--l-place)'
    case 'subject':
      return 'var(--l-subject)'
    case 'shot':
      return 'var(--accent)'
  }
}

/**
 * Influence layer → the name it is called by.
 *
 * Spelled the same as `wobu-core`'s `Layer::label`, because the prompt box
 * attributes a span to a layer in words as well as in colour and the two sides
 * disagreeing would mean a fragment credited to "Ancestry" on one surface and
 * "Species" on another.
 */
export function layerLabel(layer: InfluenceLayer): string {
  switch (layer) {
    case 'style':
      return 'Style'
    case 'world':
      return 'World'
    case 'ancestry':
      return 'Ancestry'
    case 'culture':
      return 'Culture'
    case 'place':
      return 'Place'
    case 'subject':
      return 'Subject'
    case 'shot':
      return 'Shot'
  }
}

function fallbackColor(kind: NodeKind): string {
  switch (kind) {
    case 'style_guide':
      return 'var(--l-style)'
    case 'world_bible':
      return 'var(--l-world)'
    case 'species':
      return 'var(--l-species)'
    case 'culture':
      return 'var(--l-culture)'
    case 'setting':
    case 'environment':
      return 'var(--l-place)'
    default:
      return 'var(--l-subject)'
  }
}

/** Human label without the registry (only used before `kind_registry` lands). */
export function labelFor(def: KindDef | undefined, kind: NodeKind): string {
  return def?.label ?? kind.replace(/_/g, ' ')
}

export function pluralFor(def: KindDef | undefined, kind: NodeKind): string {
  return def?.plural ?? `${labelFor(def, kind)}s`
}

export type KindIndex = Map<NodeKind, KindDef>

export function indexKinds(defs: KindDef[] | undefined): KindIndex {
  const m: KindIndex = new Map()
  for (const d of defs ?? []) m.set(d.kind, d)
  return m
}
