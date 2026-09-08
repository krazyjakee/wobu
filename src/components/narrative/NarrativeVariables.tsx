import { useMemo, useState } from 'react'
import { errorMessage, preconditionOf, type StateFile, type VariableDecl } from '../../lib/api'
import {
  useNarrativeState,
  useSaveNarrativeState,
  useSceneFiles,
  useScenes,
} from '../../lib/queries'
import { useWorldDrafts } from './worldDrafts'
import { useNarrativeWorld } from '../../lib/queries/narrativeWorld'
import { parseSafeInteger, hasUnsafeInteger } from './integerInput'
import { WORLD_COLLECTIONS } from './worldModel'

export function NarrativeVariables({
  projectKey,
  readOnly,
}: {
  projectKey: string
  readOnly: boolean
}) {
  const query = useNarrativeState()
  if (!query.data)
    return (
      <p role={query.isError ? 'alert' : 'status'}>
        {query.isError ? errorMessage(query.error) : 'Loading variables…'}
      </p>
    )
  return <VariablesEditor projectKey={projectKey} readOnly={readOnly} file={query.data} />
}
function VariablesEditor({
  projectKey,
  readOnly,
  file,
}: {
  projectKey: string
  readOnly: boolean
  file: StateFile
}) {
  const draft = useWorldDrafts((s) => s.variables[projectKey])
  const document = draft?.document ?? file.document
  const [selected, setSelected] = useState(0)
  const [message, setMessage] = useState('')
  const [confirmDelete, setConfirmDelete] = useState(false)
  const save = useSaveNarrativeState()
  const world = useNarrativeWorld()
  const scenes = useScenes()
  const ids = useMemo(() => (scenes.data?.scenes ?? []).map((scene) => scene.id), [scenes.data])
  const files = useSceneFiles(ids)
  const index = Math.min(selected, Math.max(0, document.variables.length - 1))
  const variable = document.variables[index]
  const unsafeInteger = hasUnsafeInteger(document)
  const disabled = readOnly || save.isPending
  const update = (variables: VariableDecl[]) => {
    if (disabled) return
    useWorldDrafts
      .getState()
      .putVariables(projectKey, { file: draft?.file ?? file, document: { ...document, variables } })
    setMessage('')
    setConfirmDelete(false)
  }
  const replace = (next: VariableDecl) =>
    update(document.variables.map((one, i) => (i === index ? next : one)))
  const users = variable
    ? files
        .filter((one) => one.data && referencesVariable(one.data.scene, variable.name))
        .map((one) => one.data!.scene.name)
    : []
  const worldUsers = variable
    ? WORLD_COLLECTIONS.flatMap(({ key }) =>
        (world.data?.document[key] ?? [])
          .filter((record) => referencesVariable(record, variable.name))
          .map((record) => record.name),
      )
    : []
  const allUsers = [...users, ...worldUsers]
  const submit = async () => {
    if (!draft || disabled || unsafeInteger) return
    try {
      await save.mutateAsync({
        document: draft.document,
        expected: preconditionOf(draft.file.stamp),
      })
      if (useWorldDrafts.getState().variables[projectKey] === draft)
        useWorldDrafts.getState().putVariables(projectKey, null)
      setMessage('Variables saved. Scene conditions now use these declarations.')
    } catch (error) {
      setMessage(`${errorMessage(error)} Your draft is kept.`)
    }
  }
  return (
    <>
      <div className="nrt-world-toolbar">
        <button
          className="btn is-primary"
          type="button"
          disabled={disabled || !draft || unsafeInteger}
          onClick={() => void submit()}
        >
          Save variables
        </button>
        <button
          className="btn"
          type="button"
          disabled={save.isPending || !draft}
          onClick={() => useWorldDrafts.getState().putVariables(projectKey, null)}
        >
          Discard variable changes
        </button>
        <span role="status">
          {message || (draft ? 'Unsaved variables — kept when changing views.' : 'Saved variables')}
        </span>
      </div>
      {unsafeInteger && (
        <p role="alert">
          A variable contains a number outside the editor’s exact integer range. Correct it before
          saving; the existing file is preserved.
        </p>
      )}
      {draft && draft.file.stamp?.hash !== file.stamp?.hash && (
        <p role="alert">Variables changed since this draft began. Saving checks for a conflict.</p>
      )}
      <p className="nrt-note">
        Declare the values used by conditions and effects. Host-owned values can be read by the
        story; only the game sets them.
      </p>
      <div className="nrt-world-body">
        <section className="nrt-world-list" aria-label="Declared variables">
          <button
            type="button"
            className="btn"
            disabled={disabled}
            onClick={() => {
              let count = 1
              while (document.variables.some((one) => one.name === `variable_${count}`)) count++
              update([
                ...document.variables,
                { name: `variable_${count}`, type: 'bool', default: false, owner: 'narrative' },
              ])
              setSelected(document.variables.length)
            }}
          >
            Add variable
          </button>
          {document.variables.map((one, i) => (
            <button
              type="button"
              className="btn"
              key={i}
              aria-pressed={index === i}
              onClick={() => {
                setSelected(i)
                setConfirmDelete(false)
              }}
            >
              {one.name || 'Unnamed variable'}
            </button>
          ))}
        </section>
        <section className="nrt-world-detail" aria-label="Selected variable">
          {variable ? (
            <>
              <fieldset disabled={disabled}>
                <legend>Variable</legend>
                <label>
                  Variable name
                  <input
                    value={variable.name}
                    onChange={(e) => replace({ ...variable, name: e.target.value })}
                  />
                </label>
                <p className="nrt-note">
                  Names use lowercase letters, digits and underscores. Renaming does not rewrite
                  existing conditions.
                </p>
                <label>
                  Value type
                  <select
                    value={
                      typeof variable.type === 'string'
                        ? 'bool'
                        : 'int' in variable.type
                          ? 'int'
                          : 'enum'
                    }
                    onChange={(e) =>
                      replace({
                        ...variable,
                        type:
                          e.target.value === 'bool'
                            ? 'bool'
                            : e.target.value === 'int'
                              ? { int: { min: 0, max: 100 } }
                              : { enum: { members: ['started', 'completed'] } },
                        default:
                          e.target.value === 'bool'
                            ? false
                            : e.target.value === 'int'
                              ? 0
                              : 'started',
                      })
                    }
                  >
                    <option value="bool">Boolean</option>
                    <option value="int">Bounded integer</option>
                    <option value="enum">Named values</option>
                  </select>
                </label>
                {typeof variable.type === 'object' && 'int' in variable.type && (
                  <>
                    <label>
                      Minimum
                      <input
                        type="number"
                        step="1"
                        value={variable.type.int.min}
                        onChange={(e) => {
                          const min = parseSafeInteger(e.target.value)
                          if (
                            min !== undefined &&
                            typeof variable.type === 'object' &&
                            'int' in variable.type
                          )
                            replace({ ...variable, type: { int: { ...variable.type.int, min } } })
                        }}
                      />
                    </label>
                    <label>
                      Maximum
                      <input
                        type="number"
                        step="1"
                        value={variable.type.int.max}
                        onChange={(e) => {
                          const max = parseSafeInteger(e.target.value)
                          if (
                            max !== undefined &&
                            typeof variable.type === 'object' &&
                            'int' in variable.type
                          )
                            replace({ ...variable, type: { int: { ...variable.type.int, max } } })
                        }}
                      />
                    </label>
                  </>
                )}
                {typeof variable.type === 'object' && 'enum' in variable.type && (
                  <label>
                    Named values (one per line)
                    <textarea
                      value={variable.type.enum.members.join('\n')}
                      onChange={(e) =>
                        replace({
                          ...variable,
                          type: { enum: { members: e.target.value.split('\n') } },
                        })
                      }
                    />
                  </label>
                )}
                <label>
                  Default value
                  {variable.type === 'bool' ? (
                    <select
                      value={String(variable.default)}
                      onChange={(e) => replace({ ...variable, default: e.target.value === 'true' })}
                    >
                      <option value="false">False</option>
                      <option value="true">True</option>
                    </select>
                  ) : (
                    <input
                      type={
                        typeof variable.type === 'object' && 'int' in variable.type
                          ? 'number'
                          : 'text'
                      }
                      value={String(variable.default)}
                      onChange={(e) => {
                        const value =
                          typeof variable.type === 'object' && 'int' in variable.type
                            ? parseSafeInteger(e.target.value)
                            : e.target.value
                        if (value !== undefined) replace({ ...variable, default: value })
                      }}
                    />
                  )}
                </label>
                <label>
                  Controlled by
                  <select
                    value={variable.owner ?? 'narrative'}
                    onChange={(e) =>
                      replace({ ...variable, owner: e.target.value as 'host' | 'narrative' })
                    }
                  >
                    <option value="narrative">Narrative</option>
                    <option value="host">Game host</option>
                  </select>
                </label>
                <label>
                  Description
                  <textarea
                    value={variable.description ?? ''}
                    onChange={(e) => replace({ ...variable, description: e.target.value })}
                  />
                </label>
              </fieldset>
              {allUsers.length > 0 && (
                <p>Referenced by scenes and world records: {allUsers.join(', ')}</p>
              )}
              <button
                className="btn"
                type="button"
                disabled={disabled}
                onClick={() => {
                  if (allUsers.length && !confirmDelete) {
                    setConfirmDelete(true)
                    return
                  }
                  update(document.variables.filter((_, i) => i !== index))
                }}
              >
                {confirmDelete ? 'Delete referenced variable' : 'Delete variable'}
              </button>
              {confirmDelete && (
                <p role="alert">
                  The scene and world references will remain and report the missing variable.
                </p>
              )}
            </>
          ) : (
            <p>Add a variable to use it in conditions and effects.</p>
          )}
        </section>
      </div>
    </>
  )
}
function referencesVariable(value: unknown, name: string): boolean {
  if (!value || typeof value !== 'object') return false
  if ('var' in value && value.var === name) return true
  return Object.values(value).some((child) => referencesVariable(child, name))
}
