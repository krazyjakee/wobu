import { libraryRows } from './sceneLibrary.fixture'
import { describe, expect, it } from 'vitest'
import { DEFAULT_LIBRARY_VIEW, findScenes } from './sceneLibraryModel'

describe('scene discovery', () => {
  it('finds intent and exact dialogue variants without depending on their order', () => {
    const [result] = findScenes(libraryRows, { ...DEFAULT_LIBRARY_VIEW, query: 'attack' })
    expect(result?.matches).toMatchObject([
      { sceneId: 'council', beatId: 'evidence', lineId: 'line', variantId: 'high' },
      { sceneId: 'council', beatId: 'evidence', lineId: 'line', variantId: 'low' },
    ])
    expect(
      findScenes(libraryRows, { ...DEFAULT_LIBRARY_VIEW, query: 'convince' })[0]?.matches[0]
        ?.beatId,
    ).toBe('evidence')
  })
  it('excludes generated drafts until explicitly included', () => {
    expect(findScenes(libraryRows, { ...DEFAULT_LIBRARY_VIEW, query: 'secret' })).toHaveLength(0)
    expect(
      findScenes(libraryRows, { ...DEFAULT_LIBRARY_VIEW, query: 'secret', includeDrafts: true })[0]
        ?.matches[0]?.draft,
    ).toBe(true)
  })
  it('combines independent lifecycle fields on the same variant', () => {
    expect(
      findScenes(libraryRows, {
        ...DEFAULT_LIBRARY_VIEW,
        participant: 'mira',
        policy: 'locked',
        review: 'approved',
        freshness: 'out_of_date',
      }),
    ).toHaveLength(1)
    expect(
      findScenes(libraryRows, { ...DEFAULT_LIBRARY_VIEW, policy: 'locked', review: 'draft' }),
    ).toHaveLength(0)
    expect(findScenes(libraryRows, { ...DEFAULT_LIBRARY_VIEW, participant: 'other' })).toHaveLength(
      0,
    )
  })
  it('keeps unread files discoverable by title without claiming text coverage', () => {
    const rows = [{ summary: libraryRows[0]!.summary, error: 'Conflict' }]
    expect(findScenes(rows, DEFAULT_LIBRARY_VIEW)).toHaveLength(1)
    expect(findScenes(rows, { ...DEFAULT_LIBRARY_VIEW, missing: true })).toHaveLength(0)
  })
})

describe('quest membership', () => {
  const quests = [
    { id: 'attack', name: 'Investigate the attack', scene_ids: ['council', 'council'] },
    { id: 'trust', name: 'Earn council trust', scene_ids: ['council'] },
    { id: 'rescue', name: 'Rescue the crew', scene_ids: [] },
  ]
  it('finds a shared scene under either quest without duplicating rows or memberships', () => {
    for (const quest of ['attack', 'trust']) {
      const result = findScenes(libraryRows, { ...DEFAULT_LIBRARY_VIEW, quest }, quests)
      expect(result).toHaveLength(1)
      expect(result[0]?.quests.map((entry) => entry.id)).toEqual(['attack', 'trust'])
    }
    expect(findScenes(libraryRows, DEFAULT_LIBRARY_VIEW, quests)).toHaveLength(1)
    expect(findScenes(libraryRows, { ...DEFAULT_LIBRARY_VIEW, quest: 'rescue' }, quests)).toEqual(
      [],
    )
  })
  it('combines quest with participant and text search while preserving source order', () => {
    const view = { ...DEFAULT_LIBRARY_VIEW, quest: 'attack', participant: 'mira', query: 'attack' }
    expect(findScenes(libraryRows, view, quests)).toHaveLength(1)
    expect(findScenes(libraryRows, { ...view, participant: 'other' }, quests)).toEqual([])
    expect(quests[0]?.scene_ids).toEqual(['council', 'council'])
  })
})
