import type { Operand, VariableDecl } from '../../lib/api'
import { parseSafeInteger } from './integerInput'
import { defaultOperand, sameValueType, validLiteral } from './typedFormModel'

export function TypedOperand({
  label,
  value,
  variable,
  variables,
  onChange,
}: {
  label: string
  value: Operand
  variable?: VariableDecl
  variables: VariableDecl[]
  onChange: (value: Operand) => void
}) {
  const compatible = variable ? variables.filter((one) => sameValueType(variable, one)) : variables
  const mode = 'var' in value ? 'variable' : typeof value.literal
  return (
    <div className="nrt-script-row">
      <label>
        {label} source
        <select
          value={mode}
          onChange={(event) => {
            const kind = event.target.value
            onChange(
              kind === 'variable'
                ? { var: compatible[0]!.name }
                : variable
                  ? defaultOperand(variable)
                  : { literal: kind === 'boolean' ? false : kind === 'number' ? 0 : 'member' },
            )
          }}
        >
          {variable ? (
            <option value={typeof variable.default}>Literal</option>
          ) : (
            <>
              <option value="boolean">Boolean</option>
              <option value="number">Integer</option>
              <option value="string">Enum member</option>
            </>
          )}
          <option value="variable" disabled={!compatible.length}>
            Variable
          </option>
        </select>
      </label>
      {'var' in value ? (
        <label>
          {label}
          <select value={value.var} onChange={(event) => onChange({ var: event.target.value })}>
            {!compatible.some((one) => one.name === value.var) && (
              <option value={value.var}>Missing or incompatible: {value.var}</option>
            )}
            {compatible.map((one) => (
              <option key={one.name} value={one.name}>
                {one.name}
              </option>
            ))}
          </select>
        </label>
      ) : variable?.type === 'bool' || typeof value.literal === 'boolean' ? (
        <label>
          {label}
          <select
            value={String(value.literal)}
            onChange={(event) => onChange({ literal: event.target.value === 'true' })}
          >
            <option value="false">False</option>
            <option value="true">True</option>
          </select>
        </label>
      ) : variable && 'enum' in variable.type ? (
        <label>
          {label}
          <select
            value={String(value.literal)}
            onChange={(event) => onChange({ literal: event.target.value })}
          >
            {!variable.type.enum.members.includes(String(value.literal)) && (
              <option value={String(value.literal)}>Invalid member: {String(value.literal)}</option>
            )}
            {variable.type.enum.members.map((member) => (
              <option key={member}>{member}</option>
            ))}
          </select>
        </label>
      ) : (
        <label>
          {label}
          <input
            type={typeof value.literal === 'number' ? 'number' : 'text'}
            step={1}
            value={String(value.literal)}
            min={variable && 'int' in variable.type ? variable.type.int.min : undefined}
            max={variable && 'int' in variable.type ? variable.type.int.max : undefined}
            onChange={(event) => {
              const next =
                typeof value.literal === 'number'
                  ? parseSafeInteger(event.target.value)
                  : event.target.value
              if (
                next !== undefined &&
                (typeof next === 'string' || event.target.value !== '') &&
                validLiteral(next, variable)
              )
                onChange({ literal: next })
            }}
          />
        </label>
      )}
    </div>
  )
}
