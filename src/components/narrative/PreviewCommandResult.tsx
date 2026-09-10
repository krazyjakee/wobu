import { useState } from 'react'
import type { VariableDecl } from '../../lib/api'
import type { PreviewHostResult, PreviewState } from '../../lib/api/narrativePreview'
import { TypedOperand } from './TypedOperand'

/** Inputs use the paused graph's schema, even if the author edits source meanwhile. */
export function PreviewCommandResult({
  name,
  args,
  variables,
  state,
  disabled,
  onResult,
}: {
  name: string
  args: unknown[]
  variables: VariableDecl[]
  state: PreviewState
  disabled: boolean
  onResult: (result: PreviewHostResult) => void
}) {
  const [outputs, setOutputs] = useState<PreviewState>({})
  const [failure, setFailure] = useState('Simulated host failure')
  return (
    <>
      <h3>Host command: {name}</h3>
      <pre>{JSON.stringify(args)}</pre>
      <p>Preview is paused. Choose a result to simulate; no game action is performed.</p>
      <fieldset disabled={disabled}>
        <legend>Host state returned on success</legend>
        {variables.length === 0 && <p>No host variables are declared in this compiled scene.</p>}
        {variables.map((variable) => (
          <TypedOperand
            key={variable.name}
            label={`Host output ${variable.name}`}
            variable={variable}
            variables={[]}
            value={{ literal: outputs[variable.name] ?? state[variable.name] ?? variable.default }}
            onChange={(value) => {
              if ('literal' in value) setOutputs({ ...outputs, [variable.name]: value.literal })
            }}
          />
        ))}
        <label>
          Failure reason
          <input value={failure} onChange={(event) => setFailure(event.target.value)} />
        </label>
        <div className="nrt-script-actions">
          <button
            className="btn is-primary"
            onClick={() => onResult({ success: { host_inputs: outputs } })}
          >
            Acknowledge command
          </button>
          <button className="btn" onClick={() => onResult({ failed: { message: failure } })}>
            Fail command
          </button>
          <button className="btn" onClick={() => onResult('cancelled')}>
            Cancel command
          </button>
        </div>
      </fieldset>
      <p className="nrt-note">
        Failure or cancellation keeps this command pending for an explicit retry.
      </p>
    </>
  )
}
