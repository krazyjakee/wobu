import type { Beat, Scene, VariableDecl } from '../../lib/api'
import type { RouteRef, SceneEditOperation } from './sceneEdits'
import { DestinationPicker } from './DestinationPicker'
import { ScriptOrderControls } from './ScriptOrderControls'
import { TypedCondition } from './TypedCondition'
import { TypedEffects } from './TypedEffects'

export function ScriptRoutes({
  beat,
  scene,
  scenes,
  variables,
  disabled,
  onEditOperation,
}: {
  beat: Beat
  scene: Scene
  scenes: { id: string; name: string }[]
  variables: VariableDecl[]
  disabled: boolean
  onEditOperation: (operation: SceneEditOperation) => void
}) {
  return (
    <fieldset disabled={disabled}>
      <legend>Choices and outcomes</legend>
      {(['choice', 'outcome'] as const).flatMap((kind) => {
        const routes = kind === 'choice' ? (beat.choices ?? []) : (beat.outcomes ?? [])
        return routes.map((route, index) => {
          const ref: RouteRef = { kind, beatId: beat.id, id: route.id }
          const choice = 'label' in route ? route : null
          const label = `${kind === 'choice' ? 'Choice' : 'Outcome'} ${index + 1}`
          return (
            <div className="nrt-script-line" key={route.id}>
              {choice && (
                <label>
                  {label}
                  <input
                    data-narrative-field={`choice:${route.id}:label`}
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
                fieldId={`${kind}:${route.id}:destination`}
                label={`${label} destination`}
                value={route.to}
                scene={scene}
                scenes={scenes}
                onChange={(value) => onEditOperation({ kind: 'setDestination', route: ref, value })}
              />
              <div data-narrative-field={`${kind}:${route.id}:condition`}>
                <TypedCondition
                  label={label}
                  value={choice ? choice.requires : 'when' in route ? route.when : undefined}
                  variables={variables}
                  onChange={(value) => onEditOperation({ kind: 'setCondition', owner: ref, value })}
                />
              </div>
              <div data-narrative-field={`${kind}:${route.id}:effects`}>
                <TypedEffects
                  label={label}
                  value={route.effects}
                  variables={variables}
                  onChange={(value) => onEditOperation({ kind: 'setEffects', route: ref, value })}
                />
              </div>
              <ScriptOrderControls<{ id: string }>
                label={`${kind} ${index + 1}`}
                index={index}
                items={routes}
                onChange={(next) =>
                  onEditOperation({
                    kind: 'moveRoute',
                    route: ref,
                    index: next.findIndex((item) => item.id === route.id),
                  })
                }
              />
              <button
                className="btn"
                onClick={() => onEditOperation({ kind: 'removeRoute', route: ref })}
              >
                Delete {kind} {index + 1}
              </button>
            </div>
          )
        })
      })}
      <p className="nrt-note">
        New routes stay unresolved until you choose a destination. Effects run in the listed order.
      </p>
      <div className="nrt-script-actions">
        <button
          className="btn"
          onClick={() =>
            onEditOperation({
              kind: 'addChoice',
              beatId: beat.id,
              label: 'New choice',
              to: { unresolved: {} },
            })
          }
        >
          Add choice
        </button>
        <button
          className="btn"
          onClick={() =>
            onEditOperation({ kind: 'addOutcome', beatId: beat.id, to: { unresolved: {} } })
          }
        >
          Add outcome
        </button>
        <button
          className="btn"
          onClick={() => onEditOperation({ kind: 'addOutcome', beatId: beat.id, to: { end: {} } })}
        >
          Add ending
        </button>
      </div>
    </fieldset>
  )
}
