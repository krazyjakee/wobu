import type { Operand, StateValue, VariableDecl } from '../../lib/api'

export function sameValueType(a: VariableDecl, b: VariableDecl): boolean {
  if (a.type === 'bool' || b.type === 'bool') return a.type === b.type
  if ('int' in a.type || 'int' in b.type) return 'int' in a.type && 'int' in b.type
  return (
    a.type.enum.members.every(
      (member) => b.type !== 'bool' && 'enum' in b.type && b.type.enum.members.includes(member),
    ) &&
    b.type.enum.members.every(
      (member) => a.type !== 'bool' && 'enum' in a.type && a.type.enum.members.includes(member),
    )
  )
}

export function defaultOperand(variable: VariableDecl): Operand {
  return { literal: variable.default }
}

export function validLiteral(value: StateValue, variable?: VariableDecl): boolean {
  if (!variable)
    return typeof value === 'string'
      ? validNarrativeName(value)
      : typeof value !== 'number' || Number.isSafeInteger(value)
  if (variable.type === 'bool') return typeof value === 'boolean'
  if ('enum' in variable.type)
    return typeof value === 'string' && variable.type.enum.members.includes(value)
  return (
    typeof value === 'number' &&
    Number.isSafeInteger(value) &&
    value >= variable.type.int.min &&
    value <= variable.type.int.max
  )
}

/** Match Rust Name's source identifier grammar, including YAML reserved words. */
export function validNarrativeName(value: string): boolean {
  return (
    /^[a-z][a-z0-9_]*$/.test(value) &&
    !['true', 'false', 'yes', 'no', 'on', 'off', 'y', 'n', 'null', 'nil', 'none'].includes(value)
  )
}
