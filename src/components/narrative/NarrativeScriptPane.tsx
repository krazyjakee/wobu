import { useEffect, useRef, useState } from 'react'
import type { Beat, Destination, Scene, SceneFile, Speaker } from '../../lib/api'
import { prepareScriptText } from './scriptText'
import { ScriptDialogue } from './ScriptDialogue'
import { useNodes, useSaveScene, useScene, useScenes } from '../../lib/queries'
import { useUI } from '../../store/ui'
import { mintId } from './flow/source'
import {
  beatHasLockedText,
  duplicateScriptBeat,
  removeScriptBeat,
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
  const selected = useUI((s) => s.narrative)
  const select = useUI((s) => s.selectNarrative)
  const beat = scene.beats?.find((one) => one.id === selected.beatId) ?? scene.beats?.[0]
  const nodes = useNodes(true)
  const catalog = useScenes()
  const save = useSaveScene()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [message, setMessage] = useState('')
  const root = useRef<HTMLDivElement>(null)
  const searchVariant = useSceneLibrary((s) => s.searchVariant)
  const disabled = readOnly || busy || save.isPending
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
    if (!draft || disabled) return
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
      <div className="nrt-script-toolbar">
        <button
          type="button"
          className="btn is-primary"
          disabled={disabled || !draft}
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
          <input value={scene.name} onChange={(e) => edit({ ...scene, name: e.target.value })} />
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
          <ScriptDialogue
            beat={beat}
            disabled={disabled}
            selectedLineId={selected.lineId}
            changeBeat={changeBeat}
            speakerOptions={speakerOptions}
            onSelectSlot={(lineId) =>
              select({ sceneId: scene.id, beatId: beat.id, lineId }, 'script')
            }
          />
          <fieldset disabled={disabled}>
            <legend>Choices and outcomes</legend>
            {(beat.choices ?? []).map((choice, index) => (
              <div className="nrt-script-line" key={choice.id}>
                <label>
                  Choice {index + 1}
                  <input
                    value={choice.label}
                    onChange={(e) =>
                      changeBeat({
                        ...beat,
                        choices: beat.choices?.map((one) =>
                          one.id === choice.id ? { ...one, label: e.target.value } : one,
                        ),
                      })
                    }
                  />
                </label>
                <DestinationPicker
                  label={`Choice ${index + 1} destination`}
                  value={choice.to}
                  scene={scene}
                  scenes={catalog.data?.scenes ?? []}
                  onChange={(to) =>
                    changeBeat({
                      ...beat,
                      choices: beat.choices?.map((one) =>
                        one.id === choice.id ? { ...one, to } : one,
                      ),
                    })
                  }
                />
                <span className="nrt-script-status">
                  {choice.requires && choice.requires !== 'always'
                    ? 'Conditional'
                    : 'Always available'}{' '}
                  · {choice.effects?.length ?? 0} effects
                </span>
              </div>
            ))}
            {(beat.outcomes ?? []).map((outcome, index) => (
              <div className="nrt-script-line" key={outcome.id}>
                <DestinationPicker
                  label={`Outcome ${index + 1} destination`}
                  value={outcome.to}
                  scene={scene}
                  scenes={catalog.data?.scenes ?? []}
                  onChange={(to) =>
                    changeBeat({
                      ...beat,
                      outcomes: beat.outcomes?.map((one) =>
                        one.id === outcome.id ? { ...one, to } : one,
                      ),
                    })
                  }
                />
                <span className="nrt-script-status">
                  {outcome.when && outcome.when !== 'always' ? 'Conditional' : 'Always'} ·{' '}
                  {outcome.effects?.length ?? 0} effects
                </span>
              </div>
            ))}
            <p className="nrt-note">
              New routes explicitly end the scene until you choose another destination. Conditions
              and effects are preserved; edit them in Source.
            </p>
            <div className="nrt-script-actions">
              <button
                className="btn"
                onClick={() =>
                  changeBeat({
                    ...beat,
                    choices: [
                      ...(beat.choices ?? []),
                      { id: mintId(), label: 'New choice', to: { end: {} } },
                    ],
                  })
                }
              >
                Add choice ending
              </button>
              <button
                className="btn"
                onClick={() =>
                  changeBeat({
                    ...beat,
                    outcomes: [...(beat.outcomes ?? []), { id: mintId(), to: { end: {} } }],
                  })
                }
              >
                Add outcome ending
              </button>
            </div>
          </fieldset>
        </>
      )}
    </div>
  )
}

function DestinationPicker({
  label,
  value,
  scene,
  scenes,
  onChange,
}: {
  label: string
  value: Destination
  scene: Scene
  scenes: { id: string; name: string }[]
  onChange: (value: Destination) => void
}) {
  const selected =
    'beat' in value ? `beat:${value.beat}` : 'scene' in value ? `scene:${value.scene}` : 'end'
  const options = [
    { id: 'end', title: 'End scene' },
    ...(scene.beats ?? []).map((beat) => ({ id: `beat:${beat.id}`, title: `Beat: ${beat.title}` })),
    ...scenes.map((one) => ({ id: `scene:${one.id}`, title: `Scene: ${one.name}` })),
  ]
  return (
    <label>
      {label}
      <select
        value={selected}
        onChange={(e) => {
          const key = e.target.value
          onChange(
            key === 'end'
              ? { end: {} }
              : key.startsWith('beat:')
                ? { beat: key.slice(5) }
                : { scene: key.slice(6) },
          )
        }}
      >
        {!options.some((one) => one.id === selected) && (
          <option value={selected}>Missing destination: {selected}</option>
        )}
        {options.map((one) => (
          <option key={one.id} value={one.id}>
            {one.title}
          </option>
        ))}
      </select>
    </label>
  )
}
