import { beforeEach, expect, it } from 'vitest'
import { resetNarrativeDraftGuards } from '../../lib/narrativeDraftGuard'
import { sceneEditKey, useScriptDrafts } from './scriptDrafts'
import { sessionFile } from './sceneSession.test-support'
beforeEach(() => {
  useScriptDrafts.setState({ drafts: {} })
  resetNarrativeDraftGuards()
})
const key = sceneEditKey('/project', 'scene')
it('retains original authorization and both edits after incoming source and failed save', () => {
  const store = useScriptDrafts.getState()
  const base = sessionFile()
  store.put(key, { file: base, scene: { ...base.scene, name: 'Local name' } })
  const incoming = {
    ...base,
    stamp: { ...base.stamp!, hash: 'peer' },
    scene: { ...base.scene, name: 'Peer name' },
  }
  store.observe(key, incoming)
  store.put(key, {
    file: incoming,
    scene: { ...useScriptDrafts.getState().drafts[key]!.scene, summary: 'Flow change' },
  })
  const captured = store.beginSave(key)!
  expect(store.put(key, { file: incoming, scene: incoming.scene })).toBe(false)
  store.clear(key)
  store.undo(key)
  expect(useScriptDrafts.getState().drafts[key]!.pending).toBe(true)
  store.failSave(key, captured.revision, 'Conflict', 'retained.yaml')
  expect(useScriptDrafts.getState().drafts[key]).toMatchObject({
    file: base,
    incoming,
    scene: { name: 'Local name', summary: 'Flow change' },
    conflictPath: 'retained.yaml',
    pending: false,
  })
})
it('coalesces typing, keeps structural operations separate, and bounds undo without discarding the draft', () => {
  const store = useScriptDrafts.getState(),
    file = sessionFile()
  for (let i = 0; i < 10; i++)
    store.put(
      key,
      { file, scene: { ...file.scene, name: `Typing ${i}` } },
      { coalesceKey: 'name', now: i },
    )
  expect(useScriptDrafts.getState().drafts[key]!.past).toHaveLength(1)
  for (let i = 0; i < 120; i++)
    store.put(key, { file, scene: { ...file.scene, summary: `Step ${i}` } })
  expect(useScriptDrafts.getState().drafts[key]).toMatchObject({
    scene: { summary: 'Step 119' },
    historyTruncated: true,
  })
  expect(useScriptDrafts.getState().drafts[key]!.past).toHaveLength(100)
  store.undo(key)
  store.redo(key)
  expect(useScriptDrafts.getState().drafts[key]!.scene.summary).toBe('Step 119')
  store.clear(key)
  expect(useScriptDrafts.getState().drafts[key]).toBeUndefined()
})
it('refuses stale completion to clear a newer session revision or another project', () => {
  const store = useScriptDrafts.getState(),
    file = sessionFile()
  store.put(key, { file, scene: { ...file.scene, name: 'First' } })
  const captured = store.beginSave(key)!
  store.failSave(key, captured.revision, 'Retry')
  store.put(key, { file, scene: { ...file.scene, name: 'Newer' } })
  store.put(sceneEditKey('/other', 'scene'), { file, scene: { ...file.scene, name: 'Other' } })
  store.completeSave(key, captured.revision)
  expect(Object.values(useScriptDrafts.getState().drafts).map((draft) => draft.scene.name)).toEqual(
    ['Newer', 'Other'],
  )
})

it('bounds even one oversized undo checkpoint while retaining the complete current source', () => {
  const store = useScriptDrafts.getState()
  const file = sessionFile()
  file.scene.summary = 'x'.repeat(3 * 1024 * 1024)
  store.put(key, { file, scene: { ...file.scene, name: 'Current' } })
  expect(useScriptDrafts.getState().drafts[key]).toMatchObject({
    historyTruncated: true,
    past: [],
    scene: { name: 'Current', summary: file.scene.summary },
  })
})
