import { describe, expect, it } from 'vitest'
import { NAME_LIMIT, TEXT_LIMIT, nameMatches, textMatches } from './search'
import { indexNodes } from './tree'
import { summary } from '../test/fixtures'

const world = [
  summary({ id: 'kael', name: 'Kael Vantris', summary: 'a scarred enforcer' }),
  summary({ id: 'kaelstone', name: 'Broken Kaelstone' }),
  summary({ id: 'oru', name: 'Sister Oru', summary: 'keeps the ember guild ledger' }),
  summary({ id: 'vashk', name: 'Vashk', kind: 'species' }),
]

const ids = (ns: { id: string }[]) => ns.map((n) => n.id)

describe('nameMatches', () => {
  it('returns everything, sorted, for an empty query', () => {
    // The palette opens with no query and has to show the world.
    expect(ids(nameMatches(world, ''))).toEqual(['kaelstone', 'kael', 'oru', 'vashk'])
  })

  it('puts an earlier position in the name first', () => {
    // Typing "kael" should offer Kael Vantris before Broken Kaelstone.
    expect(ids(nameMatches(world, 'kael'))).toEqual(['kael', 'kaelstone'])
  })

  it('matches the summary too, but ranks it below any name match', () => {
    const hits = nameMatches(world, 'ember')
    expect(ids(hits)).toEqual(['oru'])
  })

  it('is case-insensitive and ignores surrounding space', () => {
    expect(ids(nameMatches(world, '  VASHK '))).toEqual(['vashk'])
  })

  it('returns nothing when nothing matches', () => {
    expect(nameMatches(world, 'dragon')).toEqual([])
  })

  it('caps the list, because the palette is a menu and not a report', () => {
    const many = Array.from({ length: 100 }, (_, i) =>
      summary({ id: `n${i}`, name: `Node ${String(i).padStart(3, '0')}` }),
    )
    expect(nameMatches(many, 'node')).toHaveLength(NAME_LIMIT)
  })

  it('does not mutate the array it was given', () => {
    const before = ids(world)
    nameMatches(world, 'kael')
    expect(ids(world)).toEqual(before)
  })
})

describe('textMatches', () => {
  const byId = indexNodes(world)

  it('keeps the rank order the backend gave, rather than re-sorting', () => {
    // Rank is the one thing this side cannot recompute — FTS scored it against
    // notes text that was never loaded here.
    expect(ids(textMatches(byId, ['vashk', 'oru', 'kael'], []))).toEqual(['vashk', 'oru', 'kael'])
  })

  it('drops hits the name filter is already showing', () => {
    // Otherwise the same node appears twice, once under each heading.
    const shown = [world[0]!]
    expect(ids(textMatches(byId, ['kael', 'oru'], shown))).toEqual(['oru'])
  })

  it('skips an id that is not in the loaded list', () => {
    // A node created since the last refetch. There is nothing to render, and it
    // must not silently consume a slot in the limit either.
    expect(ids(textMatches(byId, ['ghost', 'oru'], []))).toEqual(['oru'])
  })

  it('does not count a skipped id against the limit', () => {
    const ftsIds = ['ghost', 'ghost', 'kael', 'oru']
    expect(ids(textMatches(byId, ftsIds, [], 2))).toEqual(['kael', 'oru'])
  })

  it('caps at the limit', () => {
    const many = Array.from({ length: 50 }, (_, i) => summary({ id: `n${i}` }))
    const map = indexNodes(many)
    expect(textMatches(map, ids(many), [])).toHaveLength(TEXT_LIMIT)
  })

  it('is empty when FTS returned nothing', () => {
    expect(textMatches(byId, [], [])).toEqual([])
  })

  it('is empty when everything FTS found is already shown', () => {
    expect(textMatches(byId, ['kael', 'oru'], [world[0]!, world[2]!])).toEqual([])
  })
})
