import { describe, expect, it } from 'vitest'
import type { NodeKind, NodeSummary } from '../../lib/api'
import { buildGroups } from '../../lib/tree'
import { kindDef, kindIndex, summary } from '../../test/fixtures'
import {
  FAVOURITES_BAND,
  RECENT_BAND,
  bucketBand,
  buildNavigatorRows,
  type NavigatorRowsInput,
} from './navigatorRows'

const kinds = kindIndex([
  kindDef('character', { label: 'Character', plural: 'Characters' }),
  kindDef('culture', { label: 'Culture', plural: 'Cultures' }),
])

const ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'

function world(count: number, kind: NodeKind = 'character'): NodeSummary[] {
  return Array.from({ length: count }, (_, index) =>
    summary({
      id: `${kind}-${index}`,
      name: `${ALPHABET[index % 26]}${index} ${kind}`,
      kind,
    }),
  )
}

function rows(over: Partial<NavigatorRowsInput> & { groups: NavigatorRowsInput['groups'] }) {
  return buildNavigatorRows({
    filter: '',
    closedGroups: {},
    collapsedNodes: {},
    bands: {},
    ...over,
  })
}

/** `type:key` per row, which is the whole shape of the list in one line. */
function shape(list: ReturnType<typeof rows>): string[] {
  return list.rows.map((row) => `${row.type}:${row.key}`)
}

describe('navigator sections', () => {
  const nodes = world(3)
  const groups = buildGroups(nodes, ['character'], kinds)

  it('draws favourites and recents above the tree, and neither when empty', () => {
    expect(shape(rows({ groups })).filter((key) => key.startsWith('band'))).toEqual([])

    const list = rows({
      groups,
      favourites: [nodes[1] as NodeSummary],
      recents: [nodes[2] as NodeSummary],
    })
    expect(shape(list).slice(0, 4)).toEqual([
      `band:${FAVOURITES_BAND}`,
      'node:favourites:character-1',
      `band:${RECENT_BAND}`,
      'node:recent:character-2',
    ])
  })

  it('keys a shortcut apart from the tree row for the same entity', () => {
    // Both rows are on screen at once, and React needs to tell them apart —
    // but so does everything that asks "which row is the node's place in the
    // world", from the scroll-into-view to the drag handles.
    const list = rows({ groups, favourites: [nodes[1] as NodeSummary] })
    const drawn = list.rows.filter(
      (row) => row.type === 'node' && row.tree.node.id === 'character-1',
    )
    expect(drawn.map((row) => row.type === 'node' && row.place)).toEqual(['favourites', 'tree'])
  })

  it('closes a section on request and keeps its heading', () => {
    const list = rows({
      groups,
      favourites: [nodes[1] as NodeSummary],
      bands: { [FAVOURITES_BAND]: false },
    })
    expect(shape(list).slice(0, 2)).toEqual([`band:${FAVOURITES_BAND}`, 'group:group:character'])
  })

  it('narrows a section with the filter and counts only the tree in `shown`', () => {
    const list = rows({ groups, filter: 'A0', favourites: nodes })
    expect(list.hasMatches).toBe(true)
    expect(list.shown).toBe(1)
    const band = list.rows[0]
    expect(band?.type === 'band' && band.count).toBe(1)
  })

  it('reports no matches when neither a section nor the tree has one', () => {
    const list = rows({ groups, filter: 'nothing here', favourites: nodes })
    expect(list.hasMatches).toBe(false)
    expect(list.rows).toEqual([])
  })
})

describe('navigator letter index', () => {
  const nodes = world(300)
  const groups = buildGroups(nodes, ['character'], kinds)

  it('replaces a long group with a handful of closed headings', () => {
    const list = rows({ groups })
    const bands = list.rows.filter((row) => row.type === 'band')
    expect(list.shown).toBe(0)
    expect(bands.length).toBeGreaterThan(1)
    // The point of the whole exercise: three hundred entities reachable from a
    // list short enough to read without scrolling.
    expect(list.rows.length).toBeLessThan(20)
    expect(list.rows[0]?.type).toBe('group')
  })

  it('opens one heading without opening the rest', () => {
    const first = rows({ groups }).rows.find((row) => row.type === 'band')
    expect(first?.type === 'band' && first.open).toBe(false)

    const list = rows({ groups, bands: { [first!.key]: true } })
    expect(list.shown).toBeGreaterThan(0)
    expect(list.shown).toBeLessThan(nodes.length)
    expect(list.rows.filter((row) => row.type === 'band' && row.open)).toHaveLength(1)
  })

  it('keys headings by kind, so two indexed groups do not open together', () => {
    const mixed = [...world(120), ...world(120, 'culture')]
    const both = buildGroups(mixed, ['character', 'culture'], kinds)
    const list = rows({ groups: both, bands: { [bucketBand('character', 'A')]: true } })
    const open = list.rows.filter((row) => row.type === 'band' && row.open)
    expect(open).toHaveLength(1)
    expect(open[0]?.key).toBe(bucketBand('character', 'A'))
  })

  it('stands aside while a filter is narrowing', () => {
    // Five matches behind an index of five letters is the index working
    // against the reader, so it is rebuilt on what survived the filter.
    const list = rows({ groups, filter: 'A0 ' })
    expect(list.rows.some((row) => row.type === 'band')).toBe(false)
    expect(list.shown).toBe(1)
  })

  it('keeps a nested branch inside its heading', () => {
    const parented = world(300)
    parented[1] = { ...(parented[1] as NodeSummary), parentId: 'character-0' }
    const nested = buildGroups(parented, ['character'], kinds)
    const list = rows({ groups: nested, bands: { [bucketBand('character', 'A')]: true } })
    const child = list.rows.find((row) => row.type === 'node' && row.tree.node.id === 'character-1')
    expect(child?.type === 'node' && child.tree.depth).toBe(1)
  })
})
