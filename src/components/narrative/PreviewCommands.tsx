import { useState } from 'react'
import type { VarType } from '../../lib/api'
import { validNarrativeName } from './typedFormModel'

/** Host signatures are supplied explicitly; Preview never invokes the host itself. */
export function PreviewCommands({
  onChange,
  disabled,
}: {
  onChange: (commands: Record<string, VarType[]> | null) => void
  disabled: boolean
}) {
  const [source, setSource] = useState('{}')
  const [error, setError] = useState('')
  return (
    <details>
      <summary>Host command signatures</summary>
      <p>
        Declare command argument types for this preview, for example{' '}
        <code>{'{"camera_closeup": ["bool"]}'}</code>. Commands pause for your acknowledgement.
      </p>
      <label>
        Command signatures (JSON)
        <textarea
          disabled={disabled}
          value={source}
          onChange={(event) => {
            setSource(event.target.value)
            try {
              const value: unknown = JSON.parse(event.target.value)
              if (!value || typeof value !== 'object' || Array.isArray(value))
                throw new Error('Use an object mapping command names to argument type arrays.')
              for (const [name, args] of Object.entries(value)) {
                if (!validNarrativeName(name) || !Array.isArray(args) || !args.every(validType))
                  throw new Error(
                    'Argument types must be "bool", {"int":{"min":0,"max":100}} or {"enum":{"members":["active","done"]}}.',
                  )
              }
              setError('')
              onChange(value as Record<string, VarType[]>)
            } catch (failure) {
              setError(String(failure))
              onChange(null)
            }
          }}
        />
      </label>
      {error && <p role="alert">{error}</p>}
    </details>
  )
}
function validType(value: unknown): boolean {
  if (value === 'bool') return true
  if (!value || typeof value !== 'object') return false
  if (
    'int' in value &&
    value.int &&
    typeof value.int === 'object' &&
    'min' in value.int &&
    'max' in value.int
  ) {
    const { min, max } = value.int
    return (
      typeof min === 'number' &&
      typeof max === 'number' &&
      Number.isSafeInteger(min) &&
      Number.isSafeInteger(max) &&
      min <= max
    )
  }
  if ('enum' in value && value.enum && typeof value.enum === 'object' && 'members' in value.enum) {
    const members = value.enum.members
    return (
      Array.isArray(members) &&
      members.length > 0 &&
      members.every((member) => typeof member === 'string' && validNarrativeName(member)) &&
      new Set(members).size === members.length
    )
  }
  return false
}
