import type { CompareOp, Condition, VariableDecl } from '../../lib/api'
import { defaultOperand } from './typedFormModel'
import { TypedOperand } from './TypedOperand'

export function TypedCondition({
  label,
  value = 'always',
  variables,
  onChange,
  disabled,
}: {
  disabled?: boolean
  label: string
  value?: Condition
  variables: VariableDecl[]
  onChange: (value: Condition) => void
}) {
  const kind =
    typeof value === 'string'
      ? value
      : 'compare' in value
        ? 'compare'
        : 'not' in value
          ? 'not'
          : 'all' in value
            ? 'all'
            : 'any' in value
              ? 'any'
              : 'unknown'
  const comparison = typeof value === 'object' && 'compare' in value ? value.compare : null
  const variable = variables.find((one) => one.name === comparison?.var)
  const operators: CompareOp[] =
    variable?.type !== 'bool' && variable?.type && 'int' in variable.type
      ? ['eq', 'ne', 'lt', 'le', 'gt', 'ge']
      : ['eq', 'ne']
  const group =
    typeof value === 'object' && ('all' in value || 'any' in value)
      ? 'all' in value
        ? value.all
        : value.any
      : null
  const changeGroup = (children: Condition[]) =>
    onChange(kind === 'all' ? { all: children } : { any: children })
  return (
    <fieldset disabled={disabled} className="nrt-typed-condition">
      <legend>{label}</legend>
      <label>
        {label} rule
        <select
          value={kind}
          onChange={(event) => {
            const next = event.target.value
            if (next === 'compare')
              onChange({
                compare: {
                  var: variables[0]!.name,
                  op: 'eq',
                  value: defaultOperand(variables[0]!),
                },
              })
            else if (next === 'not') onChange({ not: value })
            else if (next === 'all') onChange({ all: [value] })
            else if (next === 'any') onChange({ any: [value] })
            else if (next === 'always' || next === 'never') onChange(next)
          }}
        >
          <option value="always">Always</option>
          <option value="never">Never</option>
          <option value="compare" disabled={!variables.length}>
            Compare state
          </option>
          <option value="all">All conditions</option>
          <option value="any">Any condition</option>
          <option value="not">Not</option>
          {kind === 'unknown' && (
            <option value="unknown">Unsupported expression — preserved</option>
          )}
        </select>
      </label>
      {comparison && (
        <>
          <label>
            {label} variable
            <select
              value={comparison.var}
              onChange={(event) => {
                const next = variables.find((one) => one.name === event.target.value)!
                onChange({ compare: { var: next.name, op: 'eq', value: defaultOperand(next) } })
              }}
            >
              {!variable && (
                <option value={comparison.var}>Missing declaration: {comparison.var}</option>
              )}
              {variables.map((one) => (
                <option key={one.name}>{one.name}</option>
              ))}
            </select>
          </label>
          <label>
            {label} operator
            <select
              value={comparison.op}
              onChange={(event) =>
                onChange({ compare: { ...comparison, op: event.target.value as CompareOp } })
              }
            >
              {!operators.includes(comparison.op) && (
                <option value={comparison.op}>Invalid operator: {comparison.op}</option>
              )}
              {operators.map((op) => (
                <option key={op} value={op}>
                  {{ eq: '=', ne: '≠', lt: '<', le: '≤', gt: '>', ge: '≥' }[op]}
                </option>
              ))}
            </select>
          </label>
          <TypedOperand
            label={`${label} value`}
            variable={variable}
            variables={variables}
            value={comparison.value}
            onChange={(next) => onChange({ compare: { ...comparison, value: next } })}
          />
        </>
      )}
      {typeof value === 'object' && 'not' in value && (
        <TypedCondition
          label={`${label} negated`}
          value={value.not}
          variables={variables}
          onChange={(not) => onChange({ not })}
        />
      )}
      {group && (
        <>
          {group.map((child, index) => (
            <div key={index}>
              <TypedCondition
                label={`${label} condition ${index + 1}`}
                value={child}
                variables={variables}
                onChange={(next) =>
                  changeGroup(group.map((one, at) => (at === index ? next : one)))
                }
              />
              <button
                className="btn"
                onClick={() => changeGroup(group.filter((_, at) => at !== index))}
              >
                Remove {label} condition {index + 1}
              </button>
            </div>
          ))}
          <button className="btn" onClick={() => changeGroup([...group, 'always'])}>
            Add {label} condition
          </button>
        </>
      )}
      {kind === 'unknown' && <pre>{JSON.stringify(value, null, 2)}</pre>}
    </fieldset>
  )
}
