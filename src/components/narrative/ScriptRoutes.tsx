import type { Beat, Destination, Scene, VariableDecl } from '../../lib/api'
import { mintId } from './flow/source'
import { ScriptOrderControls } from './ScriptOrderControls'
import { TypedCondition } from './TypedCondition'
import { TypedEffects } from './TypedEffects'

export function ScriptRoutes({
  beat,
  scene,
  scenes,
  variables,
  disabled,
  changeBeat,
}: {
  beat: Beat
  scene: Scene
  scenes: { id: string; name: string }[]
  variables: VariableDecl[]
  disabled: boolean
  changeBeat: (next: Beat) => void
}) {
  return (
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
            fieldId={`choice:${choice.id}:destination`}
            label={`Choice ${index + 1} destination`}
            value={choice.to}
            scene={scene}
            scenes={scenes}
            onChange={(to) =>
              changeBeat({
                ...beat,
                choices: beat.choices?.map((one) => (one.id === choice.id ? { ...one, to } : one)),
              })
            }
          />
          <div data-narrative-field={`choice:${choice.id}:condition`}>
            <TypedCondition
              label={`Choice ${index + 1}`}
              value={choice.requires}
              variables={variables}
              onChange={(requires) =>
                changeBeat({
                  ...beat,
                  choices: beat.choices?.map((one) =>
                    one.id === choice.id ? { ...one, requires } : one,
                  ),
                })
              }
            />
          </div>
          <TypedEffects
            label={`Choice ${index + 1}`}
            value={choice.effects}
            variables={variables}
            onChange={(effects) =>
              changeBeat({
                ...beat,
                choices: beat.choices?.map((one) =>
                  one.id === choice.id ? { ...one, effects } : one,
                ),
              })
            }
          />
          <ScriptOrderControls
            label={`choice ${index + 1}`}
            index={index}
            items={beat.choices ?? []}
            onChange={(choices) => changeBeat({ ...beat, choices })}
          />
          <button
            className="btn"
            onClick={() =>
              changeBeat({ ...beat, choices: beat.choices?.filter((one) => one.id !== choice.id) })
            }
          >
            Delete choice {index + 1}
          </button>
        </div>
      ))}
      {(beat.outcomes ?? []).map((outcome, index) => (
        <div className="nrt-script-line" key={outcome.id}>
          <DestinationPicker
            fieldId={`outcome:${outcome.id}:destination`}
            label={`Outcome ${index + 1} destination`}
            value={outcome.to}
            scene={scene}
            scenes={scenes}
            onChange={(to) =>
              changeBeat({
                ...beat,
                outcomes: beat.outcomes?.map((one) =>
                  one.id === outcome.id ? { ...one, to } : one,
                ),
              })
            }
          />
          <div data-narrative-field={`outcome:${outcome.id}:condition`}>
            <TypedCondition
              label={`Outcome ${index + 1}`}
              value={outcome.when}
              variables={variables}
              onChange={(when) =>
                changeBeat({
                  ...beat,
                  outcomes: beat.outcomes?.map((one) =>
                    one.id === outcome.id ? { ...one, when } : one,
                  ),
                })
              }
            />
          </div>
          <TypedEffects
            label={`Outcome ${index + 1}`}
            value={outcome.effects}
            variables={variables}
            onChange={(effects) =>
              changeBeat({
                ...beat,
                outcomes: beat.outcomes?.map((one) =>
                  one.id === outcome.id ? { ...one, effects } : one,
                ),
              })
            }
          />
          <ScriptOrderControls
            label={`outcome ${index + 1}`}
            index={index}
            items={beat.outcomes ?? []}
            onChange={(outcomes) => changeBeat({ ...beat, outcomes })}
          />
          <button
            className="btn"
            onClick={() =>
              changeBeat({
                ...beat,
                outcomes: beat.outcomes?.filter((one) => one.id !== outcome.id),
              })
            }
          >
            Delete outcome {index + 1}
          </button>
        </div>
      ))}
      <p className="nrt-note">
        New routes explicitly end the scene until you choose another destination. Effects run in the
        listed order.
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
  )
}

function DestinationPicker({
  fieldId,
  label,
  value,
  scene,
  scenes,
  onChange,
}: {
  fieldId: string
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
        data-narrative-field={fieldId}
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
