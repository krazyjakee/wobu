import { useEffect, useRef, useState } from 'react'
import type { Beat, Scene, SceneFile, Speaker } from '../../lib/api'
import { useSceneEditSession } from './useSceneEditSession'
import { SceneEditControls } from './SceneEditControls'
import { ScriptDialogue } from './ScriptDialogue'
import { ScriptReviewControls } from './ScriptReviewControls'
import { useScriptReview } from './useScriptReview'
import { ScriptRoutes } from './ScriptRoutes'
import { ScriptDiagnostics } from './ScriptDiagnostics'
import { useNarrativeReveal } from './useNarrativeReveal'
import { TypedCondition } from './TypedCondition'
import { useNodes, useScene, useScenes, useNarrativeState } from '../../lib/queries'
import { useUI } from '../../store/ui'
import { applySceneEdit, type SceneEditOperation } from './sceneEdits'
import { beatHasLockedText, speakerFromKey, speakerKey } from './scriptModel'
import { useSceneLibrary } from './sceneLibraryStore'
import './script.css'

export function NarrativeScriptPane({
  readOnly = false,
  projectKey = '',
}: {
  readOnly?: boolean
  projectKey?: string
}) {
  const sceneId = useUI((s) => s.narrative.sceneId)
  const file = useScene(sceneId)
  if (!sceneId) return <p className="nrt-note">Choose a scene to write its script.</p>
  if (file.isPending) return <p className="nrt-note">Reading the script…</p>
  if (file.isError) return <p role="alert">Could not read this script: {String(file.error)}</p>
  return <ScriptEditor key={sceneId} file={file.data} readOnly={readOnly} projectKey={projectKey} />
}

function ScriptEditor({
  file,
  readOnly,
  projectKey,
}: {
  file: SceneFile
  readOnly: boolean
  projectKey: string
}) {
  const review = useScriptReview(file, projectKey)
  const session = useSceneEditSession(file, projectKey, readOnly || review.mutation.isPending)
  const { draft, scene, unsafeInteger, disabled } = session
  const selected = useUI((s) => s.narrative)
  const select = useUI((s) => s.selectNarrative)
  const beat = scene.beats?.find((one) => one.id === selected.beatId) ?? scene.beats?.[0]
  const nodes = useNodes(true)
  const catalog = useScenes()
  const state = useNarrativeState()
  const variables = state.data?.document?.variables ?? []
  const root = useRef<HTMLDivElement>(null)
  const typing = useRef<string | null>(null)
  const fields = useRef(new WeakMap<HTMLElement, string>())
  const fieldSerial = useRef(0)
  const [operationMessage, setOperationMessage] = useState('')
  const searchVariant = useSceneLibrary((s) => s.searchVariant)
  const characters = (nodes.data ?? []).filter((node) => node.kind === 'character')

  useNarrativeReveal(root, projectKey, scene.id)
  useEffect(() => {
    if (searchVariant?.sceneId !== scene.id) return
    select(
      {
        sceneId: scene.id,
        beatId: selected.beatId,
        lineId: searchVariant.slotId,
        variantId: searchVariant.variantId,
        field: 'text',
      },
      'search',
      { projectKey },
    )
  }, [searchVariant, scene.id, selected.beatId, select, projectKey])

  const edit = (next: Scene) =>
    session.edit(next, typing.current ? { coalesceKey: typing.current } : undefined)
  const changeBeat = (next: Beat) =>
    edit({ ...scene, beats: scene.beats?.map((one) => (one.id === next.id ? next : one)) })
  const chooseBeat = (id: string) => select({ sceneId: scene.id, beatId: id }, 'script')
  const dispatchSceneEdit = (operation: SceneEditOperation) => {
    if (disabled) return
    const result = applySceneEdit(session.scene, operation)
    if ('refused' in result) {
      setOperationMessage(result.refused)
      return
    }
    if (!result.changed || !edit(result.scene)) return
    const removed = result.removed
      .map(
        (target) =>
          target.variantId ?? target.lineId ?? target.choiceId ?? target.outcomeId ?? target.beatId,
      )
      .filter((id): id is string => !!id)
    useUI.getState().forgetNarrative(removed)
    select(
      { ...result.target, field: result.target.field ?? (result.target.beatId ? 'title' : 'name') },
      'script',
      { projectKey },
    )
    setOperationMessage(
      result.removed.length
        ? 'Removed from the scene draft. Undo draft restores it.'
        : 'Scene draft updated.',
    )
  }
  const moveBeat = (by: number) => {
    if (beat)
      dispatchSceneEdit({
        kind: 'moveBeat',
        beatId: beat.id,
        index: (scene.beats ?? []).findIndex((one) => one.id === beat.id) + by,
      })
  }

  const speakerOptions = (speaker: Speaker) => {
    const current = speakerKey(speaker)
    return (
      <>
        <option value="narrator">Narrator</option>
        <option value="player">Player</option>
        {characters.map((node) => (
          <option key={node.id} value={node.id}>
            {node.name}
          </option>
        ))}
        {current !== 'narrator' &&
          current !== 'player' &&
          !characters.some((n) => n.id === current) && (
            <option value={current}>Missing character: {current}</option>
          )}
      </>
    )
  }

  return (
    <div
      className="nrt-script-editor"
      ref={root}
      onKeyDown={(event) => {
        if (!draft || !(event.metaKey || event.ctrlKey) || event.altKey) return
        const key = event.key.toLowerCase()
        if (key !== 'z' && key !== 'y') return
        event.preventDefault()
        event.stopPropagation()
        if (key === 'y' || event.shiftKey) session.redo()
        else session.undo()
      }}
      onClickCapture={() => {
        typing.current = null
      }}
      onChangeCapture={(event) => {
        const field = event.target
        if (
          field instanceof HTMLTextAreaElement ||
          (field instanceof HTMLInputElement && field.type === 'text')
        ) {
          if (!fields.current.has(field))
            fields.current.set(field, `field:${++fieldSerial.current}`)
          typing.current = fields.current.get(field)!
        } else typing.current = null
      }}
    >
      {unsafeInteger && (
        <p role="alert">
          This scene contains an integer outside the desktop editor’s exact range. Saving is
          disabled to preserve the source value; correct it in Source before editing this script.
        </p>
      )}
      <SceneEditControls session={session} />
      {operationMessage && <p role="status">{operationMessage}</p>}
      <ScriptDiagnostics
        scene={scene}
        onSelect={(diagnostic) => {
          select(
            {
              sceneId: scene.id,
              beatId: diagnostic.beatId ?? beat?.id ?? null,
              lineId: diagnostic.slotId ?? null,
              variantId: diagnostic.variantId,
              choiceId: diagnostic.choiceId,
              outcomeId: diagnostic.outcomeId,
              field: diagnostic.destination
                ? 'destination'
                : diagnostic.kind === 'entry'
                  ? 'entry'
                  : diagnostic.kind === 'participant'
                    ? 'participants'
                    : 'condition',
            },
            'diagnostic',
            { projectKey },
          )
        }}
      />
      {readOnly && <p className="nrt-note">This project folder is read-only.</p>}
      <fieldset disabled={disabled}>
        <legend>Scene</legend>
        <label>
          Scene name
          <input
            data-narrative-field="scene:name"
            value={scene.name}
            onChange={(e) => edit({ ...scene, name: e.target.value })}
          />
        </label>
        <label>
          Summary
          <textarea
            value={scene.summary ?? ''}
            onChange={(e) => edit({ ...scene, summary: e.target.value })}
          />
        </label>
        <label>
          Participants
          <select
            data-narrative-field="scene:participants"
            multiple
            value={(scene.participants ?? []).map((p) => p.entity)}
            onChange={(e) =>
              edit({
                ...scene,
                participants: Array.from(
                  e.target.selectedOptions,
                  (option) =>
                    scene.participants?.find((p) => p.entity === option.value) ?? {
                      entity: option.value,
                    },
                ),
              })
            }
          >
            {characters.map((node) => (
              <option key={node.id} value={node.id}>
                {node.name}
              </option>
            ))}
            {(scene.participants ?? [])
              .filter((p) => !characters.some((n) => n.id === p.entity))
              .map((p) => (
                <option key={p.entity} value={p.entity}>
                  Missing character: {p.entity}
                </option>
              ))}
          </select>
        </label>
      </fieldset>
      <fieldset disabled={disabled}>
        <div data-narrative-field="scene:entry">
          <TypedCondition
            label="Scene entry"
            value={scene.entry}
            variables={variables}
            onChange={(entry) => edit({ ...scene, entry })}
          />
        </div>
        {state.isError && (
          <p role="alert">Could not load declared state. Existing conditions are preserved.</p>
        )}
      </fieldset>
      <div className="nrt-script-toolbar">
        <label>
          Beat
          <select value={beat?.id ?? ''} onChange={(e) => chooseBeat(e.target.value)}>
            {!beat && <option value="">No beats yet</option>}
            {(scene.beats ?? []).map((one, index) => (
              <option key={one.id} value={one.id}>
                {index + 1}. {one.title}
              </option>
            ))}
          </select>
        </label>
        <button
          className="btn"
          disabled={disabled}
          onClick={() =>
            dispatchSceneEdit({ kind: 'addBeat', title: 'New beat', afterId: beat?.id })
          }
        >
          Add beat
        </button>
        {beat && (
          <>
            <button
              className="btn"
              disabled={disabled || scene.beats?.[0]?.id === beat.id}
              onClick={() => moveBeat(-1)}
            >
              Move up
            </button>
            <button
              className="btn"
              disabled={disabled || scene.beats?.at(-1)?.id === beat.id}
              onClick={() => moveBeat(1)}
            >
              Move down
            </button>
            <button
              className="btn"
              disabled={disabled}
              onClick={() => dispatchSceneEdit({ kind: 'duplicateBeat', beatId: beat.id })}
            >
              Duplicate beat
            </button>
            <button
              className="btn"
              disabled={disabled || beatHasLockedText(beat)}
              title={
                beatHasLockedText(beat) ? 'Unlock dialogue before deleting this beat.' : undefined
              }
              onClick={() => dispatchSceneEdit({ kind: 'removeBeat', beatId: beat.id })}
            >
              Delete beat
            </button>
          </>
        )}
      </div>
      {beat && (
        <>
          <fieldset disabled={disabled}>
            <legend>Beat intent</legend>
            <label>
              Beat title
              <input
                data-narrative-field={`beat:${beat.id}`}
                value={beat.title}
                onChange={(e) => changeBeat({ ...beat, title: e.target.value })}
              />
            </label>
            {(beat.intents ?? []).map((intent, index) => (
              <div className="nrt-script-row" key={index}>
                <label>
                  Intent speaker {index + 1}
                  <select
                    value={speakerKey(intent.subject)}
                    onChange={(e) =>
                      changeBeat({
                        ...beat,
                        intents: beat.intents?.map((one, at) =>
                          at === index ? { ...one, subject: speakerFromKey(e.target.value) } : one,
                        ),
                      })
                    }
                  >
                    {speakerOptions(intent.subject)}
                  </select>
                </label>
                <label>
                  Intent {index + 1}
                  <textarea
                    value={intent.intent}
                    onChange={(e) =>
                      changeBeat({
                        ...beat,
                        intents: beat.intents?.map((one, at) =>
                          at === index ? { ...one, intent: e.target.value } : one,
                        ),
                      })
                    }
                  />
                </label>
                <button
                  className="btn"
                  onClick={() =>
                    changeBeat({ ...beat, intents: beat.intents?.filter((_, at) => at !== index) })
                  }
                >
                  Delete intent {index + 1}
                </button>
              </div>
            ))}
            <button
              className="btn"
              onClick={() =>
                changeBeat({
                  ...beat,
                  intents: [...(beat.intents ?? []), { subject: 'player', intent: '' }],
                })
              }
            >
              Add intent
            </button>
            <label>
              Must convey (one per line)
              <textarea
                value={beat.must_convey?.join('\n') ?? ''}
                onChange={(e) => changeBeat({ ...beat, must_convey: e.target.value.split('\n') })}
              />
            </label>
            <label>
              Must not reveal (one per line)
              <textarea
                value={beat.must_not_reveal?.join('\n') ?? ''}
                onChange={(e) =>
                  changeBeat({ ...beat, must_not_reveal: e.target.value.split('\n') })
                }
              />
            </label>
          </fieldset>
          {review.query.isError && (
            <p role="alert">Review history could not be verified: {String(review.query.error)}</p>
          )}
          {draft && (
            <p className="nrt-note">
              Save or discard your script draft before changing review or policy.
            </p>
          )}
          <ScriptDialogue
            reviewControls={(slotId, variantId) => {
              const view = review.query.data
              const line = view?.lines?.find(
                (line) =>
                  line.target.slot === slotId &&
                  (variantId === null || line.target.variant === variantId),
              )
              return view && line ? (
                <ScriptReviewControls
                  view={view}
                  line={line}
                  slotOnly={variantId === null}
                  disabled={disabled || !!draft}
                  onApply={(action) =>
                    review.mutation.mutateAsync({
                      guard: view.guard,
                      target: line.target,
                      context_revision: line.context_revision,
                      state_json: view.state_json,
                      action,
                    })
                  }
                />
              ) : null
            }}
            beat={beat}
            variables={variables}
            onEditOperation={dispatchSceneEdit}
            disabled={disabled}
            selectedLineId={selected.lineId}
            speakerOptions={speakerOptions}
            onSelectSlot={(lineId, variantId) => {
              const current = useUI.getState().narrative
              if (current.lineId !== lineId || current.variantId !== variantId)
                select({ sceneId: scene.id, beatId: beat.id, lineId, variantId }, 'script')
            }}
          />
          <ScriptRoutes
            beat={beat}
            scene={scene}
            scenes={catalog.data?.scenes ?? []}
            variables={variables}
            disabled={disabled}
            onEditOperation={dispatchSceneEdit}
          />
        </>
      )}
    </div>
  )
}
