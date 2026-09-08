import { useState } from 'react'
import type { GenerationPolicy } from '../../lib/api'
import {
  narrativeReviewContext,
  type ReviewAction,
  type ReviewContext,
  type ReviewLine,
  type ReviewSceneView,
} from '../../lib/api/narrativeReview'

export function ScriptReviewControls({
  view,
  line,
  slotOnly = false,
  disabled,
  onApply,
}: {
  view: ReviewSceneView
  line: ReviewLine
  slotOnly?: boolean
  disabled: boolean
  onApply: (action: ReviewAction) => Promise<unknown>
}) {
  const identity = `${line.target.variant}:${line.context_revision}:${line.text?.revision}:${view.guard.head}:${view.guard.stamp?.hash}`
  const [inspected, setInspected] = useState<{ identity: string; value: ReviewContext } | null>(
    null,
  )
  const [acknowledgement, setAcknowledgement] = useState<string | null>(null)
  const [error, setError] = useState('')
  const context = inspected?.identity === identity ? inspected.value : null
  const acknowledged = !!context && acknowledgement === identity
  const run = async (action: ReviewAction) => {
    setError('')
    try {
      await onApply(action)
    } catch (error) {
      setError(String(error))
    }
  }
  const policy = slotOnly ? line.slot_policy : (line.text?.lifecycle?.policy ?? 'edited')
  const scope = slotOnly ? 'slot' : 'variant'
  return (
    <div className="nrt-line-review">
      <label>
        {slotOnly ? 'Slot policy' : 'Wording policy'}
        <select
          disabled={disabled}
          value={policy}
          onChange={(event) =>
            void run({ kind: 'policy', scope, policy: event.target.value as GenerationPolicy })
          }
        >
          <option value="generated">Generated — allow replacement</option>
          <option value="edited">Edited — proposals only</option>
          <option value="locked">Locked — protect wording</option>
        </select>
      </label>
      {!slotOnly && (
        <>
          <p className="nrt-script-status">
            {line.approval_valid ? 'Approved' : 'Draft'} ·{' '}
            {line.freshness === 'current' ? 'Current' : 'Out of date'}
          </p>
          <p className="nrt-note">{line.reason}</p>
          <button
            className="btn"
            disabled={disabled}
            onClick={async () => {
              setError('')
              try {
                const captured = await narrativeReviewContext(line.target, view.state_json)
                if (captured.revision !== line.context_revision)
                  throw new Error('Context changed. Refresh the saved review before deciding.')
                setInspected({ identity, value: captured })
              } catch (error) {
                setError(String(error))
              }
            }}
          >
            Inspect reviewed context
          </button>
          {context && (
            <details open>
              <summary>Context revision {context.revision.slice(0, 12)}</summary>
              <p>{view.context_summary}</p>
              <pre>{JSON.stringify(context.inputs, null, 2)}</pre>
              <label className="nrt-review-ack">
                <input
                  type="checkbox"
                  disabled={disabled}
                  checked={acknowledged}
                  onChange={(event) => setAcknowledgement(event.target.checked ? identity : null)}
                />{' '}
                I reviewed this wording against the displayed context.
              </label>
            </details>
          )}
          <div className="nrt-line-review-actions">
            <button
              className="btn"
              disabled={disabled || !acknowledged || !line.text?.body.trim()}
              onClick={() => void run({ kind: 'approve' })}
            >
              Approve wording
            </button>
            <button
              className="btn"
              disabled={disabled || !acknowledged || line.freshness === 'current'}
              onClick={() => void run({ kind: 'attest' })}
            >
              Attest unchanged wording still fits
            </button>
          </div>
          {line.proposals
            .filter((p) => p.status === 'pending')
            .map((proposal) => (
              <details key={proposal.id}>
                <summary>Generated proposal · {proposal.id.slice(-8)}</summary>
                <p>{proposal.candidate.text}</p>
                <p className="nrt-note">{proposal.reason}</p>
                <button
                  className="btn"
                  disabled={disabled || policy === 'locked' || line.slot_policy === 'locked'}
                  onClick={() =>
                    void run({
                      kind: 'accept',
                      proposal_id: proposal.id,
                      proposal_hash: proposal.hash,
                      reviewed_text: null,
                    })
                  }
                >
                  Accept proposal
                </button>
                <button
                  className="btn"
                  disabled={disabled}
                  onClick={() =>
                    void run({
                      kind: 'reject',
                      proposal_id: proposal.id,
                      proposal_hash: proposal.hash,
                    })
                  }
                >
                  Reject proposal
                </button>
              </details>
            ))}
        </>
      )}
      {error && <p role="alert">{error}</p>}
    </div>
  )
}
