import { useState } from 'react'
import { errorMessage } from '../../../lib/api'
import type { ReviewAction, ReviewContext, ReviewRequest } from '../../../lib/api/narrativeReview'
import type { GenerationHistory } from '../../../lib/api/narrativeGeneration'
import { ReviewDiff } from './ReviewDiff'
import { reviewPolicy, type ReviewRow } from './reviewModel'
import { useReviewDrafts, type ReviewDraft } from './reviewDrafts'

export function ReviewDetail({
  row,
  projectKey,
  readOnly,
  proposalId,
  onProposal,
  onApply,
  onContext,
  onSource,
  provenance,
}: {
  row: ReviewRow
  projectKey: string
  readOnly: boolean
  proposalId: string | null
  onProposal: (id: string | null) => void
  onApply: (request: ReviewRequest) => Promise<void>
  onContext: (state: string) => Promise<ReviewContext>
  onSource: () => void
  provenance?: GenerationHistory
}) {
  const { line, scene } = row
  const selected = line.proposals.find((one) => one.id === proposalId) ?? null
  const draftKey = `${projectKey}:${row.key}:${proposalId ?? 'current'}`
  const draft = useReviewDrafts((state) => state.drafts[draftKey])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [message, setMessage] = useState('')
  const [context, setContext] = useState<ReviewContext | null>(null)
  const [inspectedGuard, setInspectedGuard] = useState('')
  const proposal = draft?.proposal ?? selected
  const baseline = proposal?.candidate.text ?? line.text?.body ?? ''
  const body = draft?.body ?? baseline
  const locked = reviewPolicy(line) === 'locked'
  const changed =
    draft &&
    (JSON.stringify(draft.authorization.guard) !== JSON.stringify(scene.guard) ||
      draft.authorization.context_revision !== line.context_revision)
  const authorization = draft?.authorization ?? {
    guard: scene.guard,
    target: line.target,
    context_revision: line.context_revision,
    state_json: scene.state_json,
  }
  const initial: ReviewDraft = {
    authorization,
    body: baseline,
    originalBody: baseline,
    currentBody: line.text?.body ?? '',
    currentRevision: line.text?.revision ?? null,
    proposal: selected,
  }
  const apply = async (action: ReviewAction) => {
    setBusy(true)
    setError('')
    setMessage('')
    const submitting = draft
    try {
      await onApply({ ...authorization, action })
      if (submitting) useReviewDrafts.getState().clear(draftKey, submitting)
      setMessage('Review decision saved. Canonical history retains the previous version.')
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setBusy(false)
    }
  }
  const accept = () =>
    proposal &&
    apply({
      kind: 'accept',
      proposal_id: proposal.id,
      proposal_hash: proposal.hash,
      reviewed_text: body === proposal.candidate.text ? null : body,
    })
  const edit = (next: string) => {
    try {
      useReviewDrafts.getState().edit(draftKey, initial, next)
      setError('')
      setMessage('')
    } catch (failure) {
      setError(errorMessage(failure))
    }
  }
  const inspect = async () => {
    setBusy(true)
    setError('')
    try {
      const guard = JSON.stringify(scene.guard)
      setContext(await onContext(scene.state_json))
      setInspectedGuard(guard)
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setBusy(false)
    }
  }
  const adopt = () => {
    const currentKey = `${projectKey}:${row.key}:current`
    if (useReviewDrafts.getState().drafts[currentKey]) {
      setError(
        'A current-wording draft already exists. Save or discard that draft before adopting this proposal.',
      )
      return
    }
    if (
      !context ||
      inspectedGuard !== JSON.stringify(scene.guard) ||
      context.revision !== line.context_revision
    )
      return
    try {
      useReviewDrafts.getState().edit(
        currentKey,
        {
          ...initial,
          authorization: {
            guard: scene.guard,
            target: line.target,
            context_revision: context.revision,
            state_json: scene.state_json,
          },
          proposal: null,
          originalBody: line.text?.body ?? '',
        },
        body,
      )
      if (draft) useReviewDrafts.getState().clear(draftKey, draft)
      onProposal(null)
    } catch (failure) {
      setError(errorMessage(failure))
    }
  }
  const policyScope = line.slot_policy === 'locked' || !line.target.variant ? 'slot' : 'variant'
  const lifecycleDisabled = busy || readOnly || !!draft || !line.text
  return (
    <article className="nrt-review-detail" aria-label="Selected dialogue review">
      <header>
        <div>
          <h2>{row.speakerName}</h2>
          <p>{row.sceneName}</p>
        </div>
        <button className="btn" onClick={onSource}>
          Open in Script
        </button>
      </header>
      <p className="nrt-review-status">
        {reviewPolicy(line)} · {line.review} ·{' '}
        {line.freshness === 'out_of_date' ? 'Out of date' : 'Current'} ·{' '}
        {line.approval_valid ? 'Approval valid' : 'No valid approval'}
      </p>
      <p>{line.reason}</p>
      <label>
        Version to review
        <select
          value={proposalId ?? 'current'}
          onChange={(event) =>
            onProposal(event.target.value === 'current' ? null : event.target.value)
          }
          disabled={busy}
        >
          <option value="current">Current accepted wording</option>
          {line.proposals.map((one) => (
            <option key={one.id} value={one.id}>
              {one.status} proposal · {one.candidate.text.slice(0, 60)}
            </option>
          ))}
        </select>
      </label>
      {proposal && <p>{proposal.reason}</p>}
      {provenance && (
        <p>
          Generated by {provenance.provider} / {provenance.model}. {provenance.usage.input} input
          and {provenance.usage.output} output tokens reported.
        </p>
      )}
      {changed && (
        <p role="status" className="nrt-review-notice">
          Saved source changed while you were editing. Your wording and original reviewed revision
          are retained; saving will check for a conflict.
        </p>
      )}
      {proposal && (
        <ReviewDiff
          current={line.text?.body ?? ''}
          candidate={body}
          currentRevision={line.text?.revision ?? null}
          baseRevision={proposal.base_revision}
          baseWording={proposal.base_wording}
        />
      )}
      <label>
        Reviewed wording
        <textarea
          value={body}
          disabled={
            busy || readOnly || locked || (proposal !== null && proposal.status !== 'pending')
          }
          onChange={(event) => edit(event.target.value)}
          onKeyDown={(event) => {
            if (
              (event.ctrlKey || event.metaKey) &&
              event.key === 'Enter' &&
              !locked &&
              !readOnly &&
              !busy
            ) {
              event.preventDefault()
              event.stopPropagation()
              if (proposal?.status === 'pending') void accept()
              else if (draft) void apply({ kind: 'edit', body })
            }
          }}
        />
      </label>
      {proposal?.status === 'pending' && (
        <details>
          <summary>Use proposal wording as a manual edit</summary>
          <p>
            A stale proposal cannot be rebased by accepting it. Inspect the current context, then
            prepare a separate manual edit. Saving it is explicit and the original proposal remains
            pending.
          </p>
          <button className="btn" disabled={busy} onClick={() => void inspect()}>
            Inspect context for manual edit
          </button>
          <button
            className="btn"
            disabled={
              busy ||
              readOnly ||
              locked ||
              !line.text ||
              !context ||
              inspectedGuard !== JSON.stringify(scene.guard) ||
              context.revision !== line.context_revision
            }
            onClick={adopt}
          >
            Prepare manual edit from this wording
          </button>
        </details>
      )}
      {locked && (
        <p>
          This wording is protected. Explicitly unlock before editing or accepting a replacement.
        </p>
      )}
      {!!draft && (
        <p>
          Your local revision is retained across refreshes and navigation. Save or discard it before
          changing approval or policy.
        </p>
      )}
      <div className="nrt-review-actions">
        {proposal?.status === 'pending' ? (
          <>
            <button
              className="btn btn-primary"
              disabled={busy || readOnly || locked || !body.trim()}
              onClick={() => void accept()}
            >
              {body === proposal.candidate.text ? 'Accept proposal' : 'Accept edited proposal'}
            </button>
            <button
              className="btn"
              disabled={busy || readOnly || !!draft}
              title={draft ? 'Discard your local edits before rejecting this proposal.' : undefined}
              onClick={() =>
                void apply({
                  kind: 'reject',
                  proposal_id: proposal.id,
                  proposal_hash: proposal.hash,
                })
              }
            >
              Reject proposal
            </button>
          </>
        ) : (
          !proposal && (
            <button
              className="btn btn-primary"
              disabled={busy || readOnly || locked || !draft || !body.trim()}
              onClick={() => void apply({ kind: 'edit', body })}
            >
              Save edited wording
            </button>
          )
        )}
        {draft && (
          <button
            className="btn"
            disabled={busy}
            onClick={() => {
              useReviewDrafts.getState().clear(draftKey)
              setMessage('Local edits discarded. The saved wording is unchanged.')
            }}
          >
            Discard local edits
          </button>
        )}
        <button
          className="btn"
          disabled={lifecycleDisabled}
          onClick={() => void apply({ kind: 'approve' })}
        >
          Approve saved wording
        </button>
        <button
          className="btn"
          disabled={lifecycleDisabled}
          onClick={() => void apply({ kind: 'attest' })}
        >
          Attest unchanged wording
        </button>
        <button
          className="btn"
          disabled={busy || readOnly || !!draft}
          onClick={() =>
            void apply({ kind: 'policy', scope: policyScope, policy: locked ? 'edited' : 'locked' })
          }
        >
          {locked ? 'Unlock' : 'Lock'} {policyScope === 'slot' ? 'slot' : 'wording'}
        </button>
      </div>
      <details>
        <summary>Source context and provenance</summary>
        <p>{scene.context_summary}</p>
        <p>
          Reviewed context revision: <code>{authorization.context_revision}</code>
        </p>
        <p>
          Slot: <code>{line.target.slot}</code> · Variant:{' '}
          <code>{line.target.variant ?? 'Empty slot'}</code>
        </p>
        <p>
          Speaker identity:{' '}
          <code>{typeof line.speaker === 'string' ? line.speaker : line.speaker.entity}</code>
        </p>
        <p>
          Slot policy: {line.slot_policy} · Text provenance:{' '}
          <code>{JSON.stringify(line.text?.provenance ?? 'human')}</code>
        </p>
        {proposal && (
          <p>
            Request: <code>{proposal.request_id}</code> · Receipt:{' '}
            <code>{proposal.receipt_id}</code>
          </p>
        )}
        <button className="btn" disabled={busy} onClick={() => void inspect()}>
          Inspect current source context
        </button>
        {context && (
          <>
            <p>
              Inspected revision: <code>{context.revision}</code>. Inspecting does not replace the
              revision guarding your edits.
            </p>
            <pre>{JSON.stringify(context.inputs, null, 2)}</pre>
          </>
        )}
      </details>
      <details>
        <summary>Decision history</summary>
        <ul>
          {scene.history
            .filter((event) => !event.target || event.target.slot === line.target.slot)
            .map((event) => (
              <li key={event.id}>
                {event.action} · {event.actor}{' '}
                {event.context_revision && <code>{event.context_revision}</code>}
              </li>
            ))}
        </ul>
        {!scene.history.length && <p>No editorial decisions have been recorded.</p>}
      </details>
      {message && <p role="status">{message}</p>}
      {error && (
        <p role="alert">{error} Your local wording and the original proposal remain available.</p>
      )}
    </article>
  )
}
