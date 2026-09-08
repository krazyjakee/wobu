import { useRef } from 'react'
import type { NarrativeDiagnostic, Scene } from '../../../lib/api'
import { useNarrativeState, useScenes } from '../../../lib/queries'
import { useUI } from '../../../store/ui'
import { DestinationPicker } from '../DestinationPicker'
import { TypedCondition } from '../TypedCondition'
import { TypedEffects } from '../TypedEffects'
import type { RouteRef, SceneEditOperation } from '../sceneEdits'
import { beatHasLockedText } from '../sceneEditPrimitives'
import { useNarrativeReveal } from '../useNarrativeReveal'

/** The same typed fields beside canvas and outline, over the shared source draft. */
export function FlowElementEditor({
  scene,
  projectKey,
  disabled,
  onEditOperation,
  diagnostics,
}: {
  scene: Scene
  diagnostics: NarrativeDiagnostic[]
  projectKey: string
  disabled: boolean
  onEditOperation: (operation: SceneEditOperation) => boolean
}) {
  const root = useRef<HTMLDivElement>(null)
  useNarrativeReveal(root, projectKey, scene.id, true)
  const selected = useUI((state) => state.narrative)
  const state = useNarrativeState()
  const catalog = useScenes()
  const variables = state.data?.document?.variables ?? []
  const scenes = catalog.data?.scenes ?? []
  const beat =
    selected.sceneId === scene.id
      ? scene.beats?.find((beat) => beat.id === selected.beatId)
      : undefined
  const choice = beat?.choices?.find((route) => route.id === selected.choiceId)
  const outcome = beat?.outcomes?.find((route) => route.id === selected.outcomeId)
  const route = choice ?? outcome
  const ref: RouteRef | null =
    beat && route ? { kind: choice ? 'choice' : 'outcome', beatId: beat.id, id: route.id } : null
  const index = beat ? scene.beats!.indexOf(beat) : -1
  return (
    <div className="nrt-flow-element-editor" ref={root}>
      <h3>{route ? (choice ? 'Choice' : 'Outcome') : beat ? 'Beat' : 'Scene'} details</h3>
      {route &&
        diagnostics
          .filter(
            (found) =>
              found.destination &&
              (choice ? found.choiceId === choice.id : found.outcomeId === outcome?.id),
          )
          .map((found, index) => (
            <button
              className="btn inline-error"
              key={`${found.code}:${index}`}
              onClick={() =>
                useUI.getState().selectNarrative(
                  {
                    sceneId: scene.id,
                    beatId: beat!.id,
                    ...(choice ? { choiceId: choice.id } : { outcomeId: outcome!.id }),
                    field: 'destination',
                  },
                  'diagnostic',
                  { projectKey, focus: true },
                )
              }
            >
              {found.message} — Edit destination
            </button>
          ))}
      <fieldset disabled={disabled}>
        {!beat ? (
          <>
            <label>
              Scene name
              <input
                data-narrative-field="scene:name"
                value={scene.name}
                onChange={(event) =>
                  onEditOperation({ kind: 'patchScene', changes: { name: event.target.value } })
                }
              />
            </label>
            <div data-narrative-field="scene:entry">
              <TypedCondition
                label="Scene entry"
                value={scene.entry}
                variables={variables}
                onChange={(value) =>
                  onEditOperation({ kind: 'setCondition', owner: { kind: 'scene' }, value })
                }
              />
            </div>
          </>
        ) : ref && route ? (
          <>
            {choice && (
              <label>
                Choice label
                <input
                  data-narrative-field={`choice:${choice.id}:label`}
                  value={choice.label}
                  onChange={(event) =>
                    onEditOperation({
                      kind: 'setChoiceLabel',
                      beatId: beat.id,
                      choiceId: choice.id,
                      value: event.target.value,
                    })
                  }
                />
              </label>
            )}
            <DestinationPicker
              fieldId={`${ref.kind}:${ref.id}:destination`}
              label="Route destination"
              value={route.to}
              scene={scene}
              scenes={scenes}
              onChange={(value) => onEditOperation({ kind: 'setDestination', route: ref, value })}
            />
            <div data-narrative-field={`${ref.kind}:${ref.id}:condition`}>
              <TypedCondition
                label={choice ? 'Choice requirement' : 'Outcome condition'}
                value={choice ? choice.requires : outcome?.when}
                variables={variables}
                onChange={(value) => onEditOperation({ kind: 'setCondition', owner: ref, value })}
              />
            </div>
            <div data-narrative-field={`${ref.kind}:${ref.id}:effects`}>
              <TypedEffects
                label="Route"
                value={route.effects}
                variables={variables}
                onChange={(value) => onEditOperation({ kind: 'setEffects', route: ref, value })}
              />
            </div>
            <p className="nrt-note">
              Effects run in the listed order. An unresolved destination blocks compilation.
            </p>
            <div className="nrt-script-actions">
              <button
                className="btn"
                onClick={() =>
                  onEditOperation({ kind: 'setDestination', route: ref, value: { unresolved: {} } })
                }
              >
                Disconnect destination
              </button>
              <button
                className="btn"
                onClick={() => onEditOperation({ kind: 'removeRoute', route: ref })}
              >
                Delete route
              </button>
            </div>
            {renderRouteOrder()}
          </>
        ) : (
          <>
            <label>
              Beat title
              <input
                data-narrative-field={`beat:${beat.id}`}
                value={beat.title}
                onChange={(event) =>
                  onEditOperation({
                    kind: 'patchBeat',
                    beatId: beat.id,
                    changes: { title: event.target.value },
                  })
                }
              />
            </label>
            <p className="nrt-note">
              Dialogue and variants remain in Script. These controls change the shared scene draft.
            </p>
            <div className="nrt-script-actions">
              <button
                className="btn"
                disabled={index <= 0}
                onClick={() =>
                  onEditOperation({ kind: 'moveBeat', beatId: beat.id, index: index - 1 })
                }
              >
                Move beat up
              </button>
              <button
                className="btn"
                disabled={index >= (scene.beats?.length ?? 0) - 1}
                onClick={() =>
                  onEditOperation({ kind: 'moveBeat', beatId: beat.id, index: index + 1 })
                }
              >
                Move beat down
              </button>
              <button
                className="btn"
                onClick={() => onEditOperation({ kind: 'duplicateBeat', beatId: beat.id })}
              >
                Duplicate beat
              </button>
              <button
                className="btn"
                disabled={beatHasLockedText(beat)}
                title={
                  beatHasLockedText(beat) ? 'Unlock dialogue through Review first.' : undefined
                }
                onClick={() => onEditOperation({ kind: 'removeBeat', beatId: beat.id })}
              >
                Delete beat
              </button>
            </div>
          </>
        )}
      </fieldset>
      {state.isError && (
        <p role="alert">Declared state could not be loaded: {String(state.error)}</p>
      )}
    </div>
  )

  function renderRouteOrder() {
    if (!beat || !ref || !route) return null
    const list = choice ? beat.choices! : beat.outcomes!
    const at = list.findIndex((one) => one.id === route.id)
    return (
      <div className="nrt-script-actions">
        <button
          className="btn"
          disabled={at === 0}
          onClick={() => onEditOperation({ kind: 'moveRoute', route: ref, index: at - 1 })}
        >
          Move route up
        </button>
        <button
          className="btn"
          disabled={at === list.length - 1}
          onClick={() => onEditOperation({ kind: 'moveRoute', route: ref, index: at + 1 })}
        >
          Move route down
        </button>
      </div>
    )
  }
}
