import { beforeEach, expect, it } from 'vitest'
import { DEFAULT_LIBRARY_VIEW } from './sceneLibraryModel'
import { readPreferences } from './sceneLibraryPreferences'
beforeEach(() => localStorage.clear())
it('migrates old saved filters without losing search, selection or unknown stable memberships', () => {
  const { act: _act, arc: _arc, tag: _tag, quest: _quest, ...old } = DEFAULT_LIBRARY_VIEW
  localStorage.setItem(
    'wobu:narrative-library:v1:/world',
    JSON.stringify({
      view: { ...old, query: 'beacon' },
      saved: [{ name: 'Old', view: { ...old, query: 'witness' } }],
      selected: 'scene',
      pins: ['scene'],
    }),
  )
  expect(readPreferences('/world')).toMatchObject({
    view: { query: 'beacon', act: '', arc: '', tag: '', quest: '' },
    saved: [{ name: 'Old', view: { query: 'witness', act: '', arc: '', tag: '' } }],
    selected: 'scene',
    revision: null,
  })
})
it('recovers malformed preferences and rejects unknown sort/filter values', () => {
  localStorage.setItem('wobu:narrative-library:v1:/world', '{broken')
  expect(readPreferences('/world').view).toEqual(DEFAULT_LIBRARY_VIEW)
  localStorage.setItem(
    'wobu:narrative-library:v1:/world',
    JSON.stringify({ view: { ...DEFAULT_LIBRARY_VIEW, sort: 'surprise' } }),
  )
  expect(readPreferences('/world').saved).toEqual([])
})
