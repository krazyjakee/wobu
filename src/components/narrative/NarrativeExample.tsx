import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { Modal } from '../Modal'
import { errorMessage, preconditionOf, type ProjectSummary } from '../../lib/api'
import { useCreateScene, useNarrativeState, useSaveNarrativeState } from '../../lib/queries'
import { qk } from '../../lib/queries/keys'
import { assertProjectSession, projectSessionEpoch } from '../../lib/projectSession'
import { useWorldDrafts } from './worldDrafts'
import { sceneEditKey, useScriptDrafts } from './scriptDrafts'
import { ashfallScene, ashfallState } from './ashfallExample'

export function NarrativeExample({
  projectKey,
  readOnly,
  onClose,
  onOpen,
}: {
  projectKey: string
  readOnly: boolean
  onClose: () => void
  onOpen: (sceneId: string) => void
}) {
  const client = useQueryClient()
  const state = useNarrativeState()
  const variableDraft = useWorldDrafts((value) => value.variables[projectKey])
  const saveState = useSaveNarrativeState(projectKey)
  const create = useCreateScene()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [declarationsSaved, setDeclarationsSaved] = useState(false)
  const add = async () => {
    if (busy || readOnly || !state.data || variableDraft) return
    const epoch = projectSessionEpoch()
    const current = () => {
      assertProjectSession(epoch)
      if (client.getQueryData<ProjectSummary>(qk.projectCurrent)?.path !== projectKey)
        throw new Error('The project changed. Reopen the original project before continuing.')
    }
    setBusy(true)
    setError('')
    try {
      current()
      const document = ashfallState(state.data.document)
      if (document.variables.length !== state.data.document.variables.length) {
        await saveState.mutateAsync({
          file: state.data,
          document,
          expected: preconditionOf(state.data.stamp),
        })
        current()
        setDeclarationsSaved(true)
      }
      current()
      const file = await create.mutateAsync('Ashfall council hearing')
      current()
      useScriptDrafts.getState().put(sceneEditKey(projectKey, file.scene.id), {
        file,
        scene: ashfallScene(file.scene.id),
      })
      onOpen(file.scene.id)
      onClose()
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Modal
      onClose={onClose}
      busy={busy}
      titleId="ashfall-example-title"
      descriptionId="ashfall-example-description"
    >
      <section className="nrt-export">
        <h2 id="ashfall-example-title">Ashfall council hearing</h2>
        <p id="ashfall-example-description">
          A handwritten example: present evidence, ask for a hearing, or challenge the council. All
          three routes meet at the council response.
        </p>
        <p>
          Create an editable scene draft and add three prefixed variables to this project. Save
          scene in Script to keep the dialogue. Creation and variable changes have separate undo
          steps.
        </p>
        <p>
          In Preview, register the command <code>ashfall_file_record</code> with no arguments. Try
          trust 20 or 40 and change <code>ashfall_knows_logbook</code>. The clerk can return{' '}
          <code>ashfall_record_filed: true</code>.
        </p>
        {state.isPending && <p role="status">Reading declared variables…</p>}
        {state.isError && <p role="alert">{errorMessage(state.error)}</p>}
        {variableDraft && (
          <p role="alert">Save or discard your variable draft before adding the example.</p>
        )}
        {declarationsSaved && (
          <p role="status">
            The example variables were saved. If scene creation fails, they remain in World
            variables and can be undone.
          </p>
        )}
        {error && (
          <p role="alert">
            {error} Any completed writes remain in the project. Check the scene library before
            retrying.
          </p>
        )}
        <button
          className="btn is-primary"
          disabled={busy || readOnly || !state.data || !!variableDraft}
          onClick={() => void add()}
        >
          {busy ? 'Creating…' : 'Create example draft'}
        </button>
        <button className="btn" disabled={busy} onClick={onClose}>
          Close
        </button>
      </section>
    </Modal>
  )
}
