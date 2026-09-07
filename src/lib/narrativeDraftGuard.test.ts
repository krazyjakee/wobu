import { expect, it } from 'vitest'
import { editorWrites, EditorWritesBlocked } from './editorWrites'
import { useScriptDrafts } from '../components/narrative/scriptDrafts'
import type { SceneFile } from './api'

it('keeps close blocked for an unmounted script draft until it is saved or discarded', async () => {
  const file: SceneFile = {
    scene: { id: 'scene', name: 'Council' },
    slug: 'council',
    rel: 'scene.yaml',
    stamp: null,
  }
  useScriptDrafts
    .getState()
    .put('/project:scene', { file, scene: { ...file.scene, name: 'Changed' } })
  await expect(editorWrites.flushAll()).rejects.toBeInstanceOf(EditorWritesBlocked)
  useScriptDrafts.getState().clear('/project:scene')
  await expect(editorWrites.flushAll()).resolves.toBeUndefined()
})
