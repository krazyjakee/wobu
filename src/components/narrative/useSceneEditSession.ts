import { isProjectSession, projectSessionEpoch } from '../../lib/projectSession'
import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import {
  errorMessage,
  isWobuError,
  type ProjectSummary,
  type Scene,
  type SceneFile,
} from '../../lib/api'
import { useSaveScene } from '../../lib/queries/narrative'
import { qk } from '../../lib/queries/keys'
import { prepareScriptText } from './scriptText'
import { hasUnsafeInteger } from './integerInput'
import { sceneEditKey, useScriptDrafts, type SceneEditOptions } from './scriptDrafts'

/** Shared explicit-save lifecycle. Calling edit, undo or redo never invokes the backend. */
export function useSceneEditSession(file: SceneFile, projectKey: string, blocked = false) {
  const key = sceneEditKey(projectKey, file.scene.id)
  const draft = useScriptDrafts((state) => state.drafts[key])
  const client = useQueryClient()
  const mutation = useSaveScene(projectKey)
  const scene = draft?.scene ?? file.scene
  const unsafeInteger = hasUnsafeInteger(scene)
  const disabled = blocked || !!draft?.pending || unsafeInteger
  useEffect(() => {
    useScriptDrafts.getState().observe(key, file)
  }, [key, file])
  const currentProject = () => client.getQueryData<ProjectSummary | null>(qk.projectCurrent)?.path
  const save = async () => {
    const epoch = projectSessionEpoch()
    if (disabled || currentProject() !== projectKey) return
    const captured = useScriptDrafts.getState().beginSave(key)
    if (!captured) return
    try {
      const prepared = await prepareScriptText(captured.file.scene, captured.scene)
      if (!isProjectSession(epoch) || currentProject() !== projectKey)
        throw new Error('The project changed. Your scene draft is kept.')
      await mutation.mutateAsync({ file: captured.file, scene: prepared })
      if (!isProjectSession(epoch) || currentProject() !== projectKey)
        throw new Error(
          'The save finished in the previous project. Reopen it to inspect the saved source before discarding this draft.',
        )
      useScriptDrafts.getState().completeSave(key, captured.revision)
    } catch (error) {
      useScriptDrafts
        .getState()
        .failSave(
          key,
          captured.revision,
          errorMessage(error),
          isWobuError(error) ? error.conflictPath : null,
        )
    }
  }
  return {
    key,
    scene,
    draft,
    dirty: !!draft,
    disabled,
    unsafeInteger,
    edit: (next: Scene, options?: SceneEditOptions) =>
      !disabled && useScriptDrafts.getState().put(key, { file, scene: next }, options),
    save,
    discard: () => useScriptDrafts.getState().clear(key),
    undo: () => {
      if (!disabled) useScriptDrafts.getState().undo(key)
    },
    redo: () => {
      if (!disabled) useScriptDrafts.getState().redo(key)
    },
  }
}
