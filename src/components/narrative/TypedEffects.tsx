import type { Effect, VariableDecl } from '../../lib/api'
import { parseSafeInteger } from './integerInput'
import { defaultOperand } from './typedFormModel'
import { TypedOperand } from './TypedOperand'

export function TypedEffects({
  label,
  value = [],
  variables,
  onChange,
}: {
  label: string
  value?: Effect[]
  variables: VariableDecl[]
  onChange: (value: Effect[]) => void
}) {
  const writable = variables.filter((one) => one.owner !== 'host')
  const integers = writable.filter((one) => one.type !== 'bool' && 'int' in one.type)
  const replace = (index: number, next: Effect) =>
    onChange(value.map((one, at) => (at === index ? next : one)))
  return (
    <fieldset>
      <legend>{label} effects (in order)</legend>
      {value.map((effect, index) => {
        const kind =
          'set' in effect
            ? 'set'
            : 'add' in effect
              ? 'add'
              : 'command' in effect
                ? 'command'
                : 'unknown'
        const stateName = 'set' in effect ? effect.set.var : 'add' in effect ? effect.add.var : ''
        const available = kind === 'add' ? integers : writable
        const variable = available.find((one) => one.name === stateName)
        const name = `${label} effect ${index + 1}`
        return (
          <div className="nrt-script-line" key={index}>
            <label>
              {name} type
              <select
                value={kind}
                onChange={(event) => {
                  if (event.target.value === 'set')
                    replace(index, {
                      set: { var: writable[0]!.name, value: defaultOperand(writable[0]!) },
                    })
                  else if (event.target.value === 'add')
                    replace(index, { add: { var: integers[0]!.name, by: 1 } })
                  else replace(index, { command: { name: 'host_command', args: [] } })
                }}
              >
                <option value="set" disabled={!writable.length}>
                  Set state
                </option>
                <option value="add" disabled={!integers.length}>
                  Add integer
                </option>
                <option value="command">Host command</option>
                {kind === 'unknown' && (
                  <option value="unknown">Unsupported effect — preserved</option>
                )}
              </select>
            </label>
            {('set' in effect || 'add' in effect) && (
              <label>
                {name} variable
                <select
                  value={stateName}
                  onChange={(event) => {
                    const next = available.find((one) => one.name === event.target.value)!
                    replace(
                      index,
                      'set' in effect
                        ? { set: { var: next.name, value: defaultOperand(next) } }
                        : { add: { var: next.name, by: effect.add.by } },
                    )
                  }}
                >
                  {!variable && (
                    <option value={stateName}>Missing or read-only: {stateName}</option>
                  )}
                  {available.map((one) => (
                    <option key={one.name}>{one.name}</option>
                  ))}
                </select>
              </label>
            )}
            {'set' in effect && (
              <TypedOperand
                label={`${name} value`}
                variable={variable}
                variables={variables}
                value={effect.set.value}
                onChange={(next) => replace(index, { set: { ...effect.set, value: next } })}
              />
            )}
            {'add' in effect && (
              <label>
                {name} amount
                <input
                  type="number"
                  step={1}
                  value={effect.add.by}
                  onChange={(event) => {
                    const by = parseSafeInteger(event.target.value)
                    if (by !== undefined) replace(index, { add: { ...effect.add, by } })
                  }}
                />
              </label>
            )}
            {'command' in effect && (
              <>
                <label>
                  {name} command name
                  <input
                    value={effect.command.name}
                    onChange={(event) =>
                      replace(index, { command: { ...effect.command, name: event.target.value } })
                    }
                  />
                </label>
                {(effect.command.args ?? []).map((arg, at) => (
                  <div key={at}>
                    <TypedOperand
                      label={`${name} argument ${at + 1}`}
                      variables={variables}
                      value={arg}
                      onChange={(next) =>
                        replace(index, {
                          command: {
                            ...effect.command,
                            args: effect.command.args?.map((one, position) =>
                              position === at ? next : one,
                            ),
                          },
                        })
                      }
                    />
                    <button
                      className="btn"
                      onClick={() =>
                        replace(index, {
                          command: {
                            ...effect.command,
                            args: effect.command.args?.filter((_, position) => position !== at),
                          },
                        })
                      }
                    >
                      Remove {name} argument {at + 1}
                    </button>
                  </div>
                ))}
                <button
                  className="btn"
                  onClick={() =>
                    replace(index, {
                      command: {
                        ...effect.command,
                        args: [...(effect.command.args ?? []), { literal: false }],
                      },
                    })
                  }
                >
                  Add {name} argument
                </button>
              </>
            )}
            {kind === 'unknown' && <pre>{JSON.stringify(effect, null, 2)}</pre>}
            <div className="nrt-script-actions">
              <button
                className="btn"
                disabled={index === 0}
                onClick={() => {
                  const next = [...value]
                  ;[next[index - 1], next[index]] = [next[index]!, next[index - 1]!]
                  onChange(next)
                }}
              >
                Move {name} up
              </button>
              <button
                className="btn"
                disabled={index === value.length - 1}
                onClick={() => {
                  const next = [...value]
                  ;[next[index + 1], next[index]] = [next[index]!, next[index + 1]!]
                  onChange(next)
                }}
              >
                Move {name} down
              </button>
              <button
                className="btn"
                onClick={() => onChange(value.filter((_, at) => at !== index))}
              >
                Delete {name}
              </button>
            </div>
          </div>
        )
      })}
      <button
        className="btn"
        onClick={() =>
          onChange([
            ...value,
            writable[0]
              ? { set: { var: writable[0].name, value: defaultOperand(writable[0]) } }
              : { command: { name: 'host_command', args: [] } },
          ])
        }
      >
        Add {label} effect
      </button>
    </fieldset>
  )
}
