import { useState } from 'react'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { Condition, Effect, VariableDecl } from '../../lib/api'
import { TypedCondition } from './TypedCondition'
import { TypedEffects } from './TypedEffects'

const variables: VariableDecl[] = [
  { name: 'trust', type: { int: { min: 0, max: 100 } }, default: 10 },
  { name: 'seen', type: 'bool', default: false },
  { name: 'quest', type: { enum: { members: ['active', 'done'] } }, default: 'active' },
  { name: 'external', type: 'bool', default: false, owner: 'host' },
]

function ConditionHarness({ initial = 'always' }: { initial?: Condition }) {
  const [value, setValue] = useState(initial)
  return (
    <>
      <TypedCondition label="Gate" value={value} variables={variables} onChange={setValue} />
      <output data-testid="value">{JSON.stringify(value)}</output>
    </>
  )
}
function EffectsHarness() {
  const [value, setValue] = useState<Effect[]>([])
  return (
    <>
      <TypedEffects label="Choice" value={value} variables={variables} onChange={setValue} />
      <output data-testid="value">{JSON.stringify(value)}</output>
    </>
  )
}
const current = () => JSON.parse(screen.getByTestId('value').textContent!)

describe('typed narrative forms', () => {
  it('authors bool/enum comparisons and rejects fractional or out-of-range integers', () => {
    render(<ConditionHarness />)
    fireEvent.change(screen.getByLabelText('Gate rule'), { target: { value: 'compare' } })
    fireEvent.change(screen.getByLabelText('Gate value'), { target: { value: '40' } })
    expect(current().compare.value).toEqual({ literal: 40 })
    fireEvent.change(screen.getByLabelText('Gate value'), { target: { value: '101' } })
    fireEvent.change(screen.getByLabelText('Gate value'), { target: { value: '1.5' } })
    expect(current().compare.value).toEqual({ literal: 40 })
    fireEvent.change(screen.getByLabelText('Gate variable'), { target: { value: 'seen' } })
    expect(screen.getByLabelText('Gate operator').querySelectorAll('option')).toHaveLength(2)
    fireEvent.change(screen.getByLabelText('Gate value'), { target: { value: 'true' } })
    expect(current().compare.value).toEqual({ literal: true })
    fireEvent.change(screen.getByLabelText('Gate variable'), { target: { value: 'quest' } })
    fireEvent.change(screen.getByLabelText('Gate value'), { target: { value: 'done' } })
    expect(current().compare.value).toEqual({ literal: 'done' })
  })

  it('wraps nested expressions without discarding sibling conditions', () => {
    const unknown = { future_expression: { data: [1, 2] } } as unknown as Condition
    render(<ConditionHarness initial={{ all: [unknown, { not: 'never' }] }} />)
    fireEvent.change(screen.getByLabelText('Gate condition 2 negated rule'), {
      target: { value: 'always' },
    })
    expect(current()).toEqual({ all: [unknown, { not: 'always' }] })
    fireEvent.change(screen.getByLabelText('Gate rule'), { target: { value: 'any' } })
    expect(current()).toEqual({ any: [{ all: [unknown, { not: 'always' }] }] })
  })

  it('preserves missing declarations until a writer explicitly replaces them', () => {
    const initial: Condition = { compare: { var: 'removed', op: 'ge', value: { var: 'legacy' } } }
    const changed = vi.fn()
    render(<TypedCondition label="Gate" value={initial} variables={variables} onChange={changed} />)
    expect(screen.getByLabelText('Gate variable')).toHaveValue('removed')
    expect(screen.getByLabelText('Gate value')).toHaveValue('legacy')
    expect(changed).not.toHaveBeenCalled()
  })

  it('excludes host writes, orders effects and retains typed command arguments', () => {
    render(<EffectsHarness />)
    fireEvent.click(screen.getByRole('button', { name: 'Add Choice effect' }))
    const writable = screen.getByLabelText('Choice effect 1 variable')
    expect(writable.querySelector('option[value="external"]')).toBeNull()
    fireEvent.change(screen.getByLabelText('Choice effect 1 type'), { target: { value: 'add' } })
    fireEvent.change(screen.getByLabelText('Choice effect 1 amount'), { target: { value: '-5' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add Choice effect' }))
    fireEvent.change(screen.getByLabelText('Choice effect 2 type'), {
      target: { value: 'command' },
    })
    fireEvent.change(screen.getByLabelText('Choice effect 2 command name'), {
      target: { value: 'camera_closeup' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Add Choice effect 2 argument' }))
    fireEvent.change(screen.getByLabelText('Choice effect 2 argument 1 source'), {
      target: { value: 'variable' },
    })
    fireEvent.change(screen.getByLabelText('Choice effect 2 argument 1'), {
      target: { value: 'external' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Move Choice effect 2 up' }))
    expect(current()).toEqual([
      { command: { name: 'camera_closeup', args: [{ var: 'external' }] } },
      { add: { var: 'trust', by: -5 } },
    ])
  })
})
