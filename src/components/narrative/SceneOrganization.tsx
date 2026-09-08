import { useRef, useState } from 'react'
import type { SceneFile } from '../../lib/api'
import { useNarrativeWorld } from '../../lib/queries/narrativeWorld'
import { useSceneEditSession } from './useSceneEditSession'
import { applySceneEdit } from './sceneEdits'
import { useNarrativeReveal } from './useNarrativeReveal'
import { MultiplePicker, RecordPicker } from './WorldFields'

/** Organization shares the same draft and original save guard as Flow and Script. */
export function SceneOrganization({
  file,
  projectKey,
  readOnly,
}: {
  file: SceneFile
  projectKey: string
  readOnly: boolean
}) {
  const root = useRef<HTMLElement>(null)
  useNarrativeReveal(root, projectKey, file.scene.id)
  const world = useNarrativeWorld()
  const session = useSceneEditSession(file, projectKey, readOnly)
  const [error, setError] = useState('')
  const update = (changes: { act_id?: string; arc_id?: string; tag_ids?: string[] }) => {
    const result = applySceneEdit(session.scene, { kind: 'patchScene', changes })
    if ('refused' in result) setError(result.refused)
    else {
      session.edit(result.scene)
      setError('')
    }
  }
  return (
    <section ref={root} aria-label="Scene organization" className="nrt-organization">
      <h3>Organization</h3>
      <p className="nrt-note">Saved with this scene. Manage shared names in World.</p>
      {world.error && (
        <p role="alert">Could not read organization records: {world.error.message}</p>
      )}
      <fieldset disabled={session.disabled || !world.data}>
        <div data-narrative-field="scene:act">
          <RecordPicker
            label="Act"
            value={session.scene.act_id ?? ''}
            options={world.data?.document.acts ?? []}
            onChange={(value) => update({ act_id: value || undefined })}
          />
        </div>
        <div data-narrative-field="scene:arc">
          <RecordPicker
            label="Arc"
            value={session.scene.arc_id ?? ''}
            options={world.data?.document.arcs ?? []}
            onChange={(value) => update({ arc_id: value || undefined })}
          />
        </div>
        <div data-narrative-field="scene:tags">
          <MultiplePicker
            label="Tags"
            values={session.scene.tag_ids ?? []}
            options={world.data?.document.tags ?? []}
            onChange={(values) => update({ tag_ids: values.length ? values : undefined })}
          />
        </div>
      </fieldset>
      {error && <p role="alert">{error}</p>}
    </section>
  )
}
