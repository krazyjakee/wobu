import { assertProjectSession, projectSessionEpoch } from '../../lib/projectSession'
import { useUI } from '../../store/ui'
import { worldRecordEntities } from './narrativeBacklinkModel'
import { useEffect, useRef, useState } from 'react'
import type { WorldFile, WorldDocument } from '../../lib/api/narrativeWorld'
import { errorMessage } from '../../lib/api'
import { useNodes, useNarrativeState, useScenes } from '../../lib/queries'
import { useNarrativeWorld, useSaveNarrativeWorld } from '../../lib/queries/narrativeWorld'
import { useWorldDrafts } from './worldDrafts'
import {
  createWorldRecord,
  recordUsers,
  requiredWorldFields,
  WORLD_COLLECTIONS,
  type WorldCollection,
  type WorldItem,
} from './worldModel'
import { WorldFields } from './WorldFields'
import { NarrativeVariables } from './NarrativeVariables'
import './world.css'

export function NarrativeWorldPane(props: { projectKey: string; readOnly: boolean }) {
  const requested = useUI((state) => state.narrativeWorldTarget)
  const sequence = requested?.projectKey === props.projectKey ? requested.seq : 0
  return <WorldPaneSession key={`${props.projectKey}:${sequence}`} {...props} />
}
function WorldPaneSession({ projectKey, readOnly }: { projectKey: string; readOnly: boolean }) {
  const requested = useUI((state) => state.narrativeWorldTarget)
  const target = requested?.projectKey === projectKey ? requested : null
  const [section, setSection] = useState<WorldCollection | 'variables'>(
    target?.collection ?? 'facts',
  )
  const query = useNarrativeWorld()
  return (
    <main className="nrt-world" aria-label="Narrative world state">
      <nav className="nrt-world-tabs" aria-label="World record categories">
        {WORLD_COLLECTIONS.map(({ key, label }) => (
          <button
            type="button"
            className="btn"
            aria-pressed={section === key}
            key={key}
            onClick={() => setSection(key)}
          >
            {label}
          </button>
        ))}
        <button
          type="button"
          className="btn"
          aria-pressed={section === 'variables'}
          onClick={() => setSection('variables')}
        >
          Variables
        </button>
      </nav>
      {section === 'variables' ? (
        <NarrativeVariables projectKey={projectKey} readOnly={readOnly} />
      ) : query.data ? (
        <WorldEditor
          projectKey={projectKey}
          readOnly={readOnly}
          file={query.data}
          collection={section}
          onCollection={setSection}
        />
      ) : (
        <p role={query.isError ? 'alert' : 'status'}>
          {query.isError ? errorMessage(query.error) : 'Loading narrative world…'}
        </p>
      )}
    </main>
  )
}
function WorldEditor({
  projectKey,
  readOnly,
  file,
  collection,
  onCollection,
}: {
  projectKey: string
  readOnly: boolean
  file: WorldFile
  collection: WorldCollection
  onCollection: (collection: WorldCollection) => void
}) {
  const draft = useWorldDrafts((s) => s.world[projectKey])
  const document = draft?.document ?? file.document
  const requested = useUI((state) => state.narrativeWorldTarget)
  const target = requested?.projectKey === projectKey ? requested : null
  const [selection, setSelection] = useState<string | null>(target?.recordId ?? null)
  const detail = useRef<HTMLElement>(null)
  const revealed = useRef<number | null>(null)
  const [filter, setFilter] = useState('')
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null)
  const [message, setMessage] = useState('')
  const save = useSaveNarrativeWorld()
  const nodes = useNodes(true)
  const scenes = useScenes()
  const variables = useNarrativeState()
  const required = requiredWorldFields(document)
  const items = document[collection] ?? []
  const item = items.find((one) => one.id === selection) ?? items[0]
  const missingTarget =
    target?.collection === collection && !items.some((record) => record.id === target.recordId)
  const users = item ? recordUsers(document, item.id) : []
  useEffect(() => {
    if (target && target.collection === collection && target.seq !== revealed.current) {
      const field = detail.current?.querySelector<HTMLInputElement>('input')
      if (field && !field.matches(':disabled')) field.focus()
      else detail.current?.focus()
      revealed.current = target.seq
    }
  }, [target, item?.id, collection])
  const disabled = readOnly || save.isPending
  const update = (next: WorldDocument) => {
    if (disabled) return
    useWorldDrafts.getState().putWorld(projectKey, { file: draft?.file ?? file, document: next })
    setMessage('')
    setConfirmDelete(null)
  }
  const replace = (next: WorldItem) =>
    update({ ...document, [collection]: items.map((one) => (one.id === next.id ? next : one)) })
  const submit = async () => {
    const epoch = projectSessionEpoch()
    if (!draft || disabled || required.length) return
    try {
      await save.mutateAsync({ file: draft.file, document: draft.document })
      assertProjectSession(epoch)
      if (useWorldDrafts.getState().world[projectKey] === draft)
        useWorldDrafts.getState().putWorld(projectKey, null)
      setMessage('World saved. Changes can be undone.')
    } catch (error) {
      setMessage(`${errorMessage(error)} Your draft is kept.`)
    }
  }
  const remove = () => {
    if (!item || disabled) return
    if (users.length && confirmDelete !== item.id) {
      setConfirmDelete(item.id)
      return
    }
    update({ ...document, [collection]: items.filter((one) => one.id !== item.id) })
    setSelection(null)
  }
  return (
    <>
      <div className="nrt-world-toolbar">
        <button
          className="btn is-primary"
          type="button"
          disabled={disabled || !draft || !!required.length}
          onClick={() => void submit()}
        >
          Save world
        </button>
        <button
          className="btn"
          type="button"
          disabled={save.isPending || !draft}
          onClick={() => useWorldDrafts.getState().putWorld(projectKey, null)}
        >
          Discard world changes
        </button>
        <span role="status">
          {(missingTarget &&
            'The requested record no longer exists. Showing the first available record in this category.') ||
            message ||
            (draft ? 'Unsaved world draft — kept when changing views.' : 'Saved world records')}
        </span>
      </div>
      {!!required.length && (
        <section aria-label="Required world fields">
          <h3>Complete these fields before saving</h3>
          {required.map((problem, index) => (
            <button
              key={index}
              type="button"
              className="btn"
              onClick={() => {
                onCollection(problem.collection)
                setSelection(problem.id)
              }}
            >
              {problem.message}
            </button>
          ))}
        </section>
      )}
      {readOnly && <p className="nrt-note">This project is read-only.</p>}
      {draft && draft.file.stamp?.hash !== file.stamp?.hash && (
        <p role="alert">
          The saved world changed. Saving checks for a conflict; discard to use the latest records.
        </p>
      )}
      <div className="nrt-world-body">
        <section className="nrt-world-list" aria-label="World records">
          <label>
            Find a record
            <input type="search" value={filter} onChange={(e) => setFilter(e.target.value)} />
          </label>
          <button
            className="btn"
            type="button"
            disabled={disabled}
            onClick={() => {
              const added = createWorldRecord(collection)
              update({ ...document, [collection]: [...items, added] })
              setSelection(added.id)
            }}
          >
            Add record
          </button>
          {items
            .filter((one) => one.name.toLocaleLowerCase().includes(filter.toLocaleLowerCase()))
            .map((one) => (
              <button
                className="btn"
                type="button"
                key={one.id}
                aria-pressed={one.id === item?.id}
                onClick={() => {
                  setSelection(one.id)
                  setConfirmDelete(null)
                }}
              >
                {one.name || 'Unnamed record'}
              </button>
            ))}
          {!items.length && <p>No records yet.</p>}
        </section>
        <section
          ref={detail}
          tabIndex={-1}
          className="nrt-world-detail"
          aria-label="Selected world record"
        >
          {item ? (
            <>
              <fieldset disabled={disabled}>
                <legend>{WORLD_COLLECTIONS.find((one) => one.key === collection)?.label}</legend>
                <WorldFields
                  item={item}
                  document={document}
                  characters={(nodes.data ?? [])
                    .filter((node) => node.kind === 'character')
                    .map(({ id, name }) => ({ id, name }))}
                  entities={(nodes.data ?? []).map(({ id, name }) => ({ id, name }))}
                  scenes={scenes.data?.scenes ?? []}
                  variables={variables.data?.document.variables ?? []}
                  onChange={replace}
                />
              </fieldset>
              {worldRecordEntities(item).length > 0 && (
                <nav aria-label="Related entities">
                  {worldRecordEntities(item).map((id) => (
                    <button
                      className="btn"
                      key={id}
                      disabled={!nodes.data?.some((node) => node.id === id)}
                      onClick={() => {
                        useUI.getState().openNarrativeEntity(projectKey, id)
                      }}
                    >
                      {nodes.data?.find((node) => node.id === id)?.name ?? `Missing entity: ${id}`}
                    </button>
                  ))}
                </nav>
              )}
              {users.length > 0 && (
                <section>
                  <h3>Used by</h3>
                  <ul>
                    {users.map(({ collection: group, record }) => (
                      <li key={record.id}>
                        <button
                          type="button"
                          className="btn"
                          onClick={() => {
                            onCollection(group)
                            setSelection(record.id)
                          }}
                        >
                          {record.name}
                        </button>
                      </li>
                    ))}
                  </ul>
                </section>
              )}
              <button type="button" className="btn" disabled={disabled} onClick={remove}>
                {confirmDelete === item.id ? 'Delete referenced record' : 'Delete record'}
              </button>
              {confirmDelete === item.id && (
                <p role="alert">
                  {users.length} records refer to this record. Deleting keeps those records and
                  reports their missing reference.
                </p>
              )}
              <section aria-label="World record diagnostics">
                <h3>Saved record checks</h3>
                {file.diagnostics
                  .filter((one) => one.recordId === item.id)
                  .map((one, index) => (
                    <p key={index}>
                      {one.field}: {one.message}
                    </p>
                  ))}
                {draft && <p className="nrt-note">Save to check the changed world records.</p>}
              </section>
            </>
          ) : (
            <p>Add a record to begin authoring the world.</p>
          )}
        </section>
      </div>
    </>
  )
}
