import { useEffect, useRef, useState } from 'react'
import type { Beat, Scene, SceneFile, Speaker } from '../../lib/api'
import { prepareScriptText } from './scriptText'
import { ScriptDialogue } from './ScriptDialogue'
import { ScriptReviewControls } from './ScriptReviewControls'
import { useScriptReview } from './useScriptReview'
import { ScriptRoutes } from './ScriptRoutes'
import { ScriptDiagnostics } from './ScriptDiagnostics'
import { diagnosticField } from './scriptDiagnosticField'
import { TypedCondition } from './TypedCondition'
import { hasUnsafeInteger } from './integerInput'
import { useNodes, useSaveScene, useScene, useScenes, useNarrativeState } from '../../lib/queries'
import { useUI } from '../../store/ui'
import { mintId } from './flow/source'
import {
  beatHasLockedText,
  duplicateScriptBeat,
  removeScriptBeat,
  removeScriptVariant,
  removeScriptSlot,
  speakerFromKey,
  speakerKey,
} from './scriptModel'
import { useScriptDrafts } from './scriptDrafts'
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
  const key = `${projectKey}:${file.scene.id}`
  const draft = useScriptDrafts((s) => s.drafts[key])
  const scene = draft?.scene ?? file.scene
  const unsafeInteger = hasUnsafeInteger(scene)
  const selected = useUI((s) => s.narrative)
  const select = useUI((s) => s.selectNarrative)
  const beat = scene.beats?.find((one) => one.id === selected.beatId) ?? scene.beats?.[0]
  const nodes = useNodes(true)
  const catalog = useScenes()
  const state = useNarrativeState()
  const variables = state.data?.document?.variables ?? []
  const save = useSaveScene()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [message, setMessage] = useState('')
  const [focusField, setFocusField] = useState<string | null>(null)
  const root = useRef<HTMLDivElement>(null)
  const searchVariant = useSceneLibrary((s) => s.searchVariant)
  const review = useScriptReview(file, projectKey)
  const disabled = readOnly || busy || save.isPending || review.mutation.isPending
  const characters = (nodes.data ?? []).filter((node) => node.kind === 'character')

  useEffect(() => {
    if (!selected.lineId) return
    root.current
      ?.querySelector<HTMLElement>(`[data-slot-id="${selected.lineId}"]`)
      ?.scrollIntoView?.({
        block: 'nearest',
      })
  }, [selected.lineId, selected.beatId])

  useEffect(() => {
    if (searchVariant?.sceneId !== scene.id) return
    const field = root.current?.querySelector<HTMLTextAreaElement>(
      `[data-variant-id="${searchVariant.variantId}"]`,
    )
    field?.scrollIntoView?.({ block: 'center' })
    field?.focus()
  }, [searchVariant, scene.id])

  useEffect(() => {
    if (!focusField) return
    const field = Array.from(
      root.current?.querySelectorAll<HTMLElement>('[data-narrative-field]') ?? [],
    ).find((element) => element.dataset.narrativeField === focusField)
    const control = field?.matches('input, select, textarea')
      ? field
      : field?.querySelector<HTMLElement>('input, select, textarea')
    control?.scrollIntoView?.({ block: 'center' })
    control?.focus()
    if (control) setFocusField(null)
  }, [focusField, selected.beatId])

  const edit = (next: Scene) => {
    if (disabled) return
    useScriptDrafts.getState().put(key, { file: draft?.file ?? file, scene: next })
    setMessage('')
  }
  const changeBeat = (next: Beat) =>
    edit({ ...scene, beats: scene.beats?.map((one) => (one.id === next.id ? next : one)) })
  const chooseBeat = (id: string) => select({ sceneId: scene.id, beatId: id }, 'script')
  const moveBeat = (by: number) => {
    if (!beat) return
    const beats = [...(scene.beats ?? [])]
    const from = beats.findIndex((one) => one.id === beat.id)
    const to = from + by
    if (to < 0 || to >= beats.length) return
    ;[beats[from], beats[to]] = [beats[to]!, beats[from]!]
    edit({ ...scene, beats })
  }

  const submit = async () => {
    if (!draft || disabled || unsafeInteger) return
    setBusy(true)
    setError('')
    try {
      const next = await prepareScriptText(draft.file.scene, draft.scene)
      await save.mutateAsync({ file: draft.file, scene: next })
      // A remounted editor can contain newer typing while this save finishes.
      // Keep its original stamp so the next write detects the intervening save.
      if (useScriptDrafts.getState().drafts[key] === draft) {
        useScriptDrafts.getState().clear(key)
      }
      setMessage('Script saved. Changes are available in Flow and can be undone.')
    } catch (failure) {
      setError(String(failure))
    } finally {
      setBusy(false)
    }
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
    <div className="nrt-script-editor" ref={root}>
      {unsafeInteger && (
        <p role="alert">
          This scene contains an integer outside the desktop editor’s exact range. Saving is
          disabled to preserve the source value; correct it in Source before editing this script.
        </p>
      )}
      <div className="nrt-script-toolbar">
        <button
          type="button"
          className="btn is-primary"
          disabled={disabled || !draft || unsafeInteger}
          onClick={() => void submit()}
        >
          {busy ? 'Saving…' : 'Save script'}
        </button>
        <button
          type="button"
          className="btn"
          disabled={busy || save.isPending || !draft}
          onClick={() => useScriptDrafts.getState().clear(key)}
        >
          Discard changes
        </button>
        <span role="status">
          {draft ? 'Unsaved draft — kept while switching tabs.' : message || 'Saved script'}
        </span>
      </div>
      <ScriptDiagnostics
        scene={scene}
        onSelect={(diagnostic) => {
          select(
            {
              sceneId: scene.id,
              beatId: diagnostic.beatId ?? beat?.id ?? null,
              lineId: diagnostic.slotId ?? null,
            },
            'script',
          )
          setFocusField(diagnosticField(diagnostic))
        }}
      />
      {error && (
        <p className="inline-error" role="alert">
          Could not save: {error}. Your draft is kept.
        </p>
      )}
      {draft && draft.file.stamp?.hash !== file.stamp?.hash && (
        <p role="alert">
          The scene changed since this draft began. Saving will check for a conflict; discard to
          load the latest version.
        </p>
      )}
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
          onClick={() => {
            const next = { id: mintId(), title: 'New beat' }
            edit({ ...scene, beats: [...(scene.beats ?? []), next] })
            chooseBeat(next.id)
          }}
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
              onClick={() => {
                const copy = duplicateScriptBeat(beat)
                const beats = [...(scene.beats ?? [])]
                beats.splice(beats.findIndex((one) => one.id === beat.id) + 1, 0, copy)
                edit({ ...scene, beats })
                chooseBeat(copy.id)
              }}
            >
              Duplicate beat
            </button>
            <button
              className="btn"
              disabled={disabled || beatHasLockedText(beat)}
              title={
                beatHasLockedText(beat) ? 'Unlock dialogue before deleting this beat.' : undefined
              }
              onClick={() => {
                const next = removeScriptBeat(scene, beat.id)
                edit(next)
                select({ sceneId: scene.id, beatId: next.beats?.[0]?.id ?? null }, 'script')
              }}
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
            onDeleteVariant={(slotId, variantId) =>
              edit(removeScriptVariant(scene, beat.id, slotId, variantId))
            }
            onDeleteSlot={(slotId) => edit(removeScriptSlot(scene, beat.id, slotId))}
            disabled={disabled}
            selectedLineId={selected.lineId}
            changeBeat={changeBeat}
            speakerOptions={speakerOptions}
            onSelectSlot={(lineId) =>
              select({ sceneId: scene.id, beatId: beat.id, lineId }, 'script')
            }
          />
          <ScriptRoutes
            beat={beat}
            scene={scene}
            scenes={catalog.data?.scenes ?? []}
            variables={variables}
            disabled={disabled}
            changeBeat={changeBeat}
          />
        </>
      )}
    </div>
  )
}
