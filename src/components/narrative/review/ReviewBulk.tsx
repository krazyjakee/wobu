import { useState } from 'react'
import { errorMessage } from '../../../lib/api'
import {
  narrativeReviewBatch,
  type ReviewAction,
  type ReviewBatchItem,
  type ReviewRequest,
} from '../../../lib/api/narrativeReview'
import { reviewTargetKey, type ReviewRow } from './reviewModel'
import { useReviewDrafts } from './reviewDrafts'

type Operation = 'approve' | 'attest' | 'lock' | 'unlock'
interface Plan {
  fingerprint: string
  requests: ReviewRequest[]
  items: ReviewBatchItem[]
}
export function ReviewBulk({
  rows,
  projectKey,
  readOnly,
  onApplied,
}: {
  rows: ReviewRow[]
  projectKey: string
  readOnly: boolean
  onApplied: () => void
}) {
  const [operation, setOperation] = useState<Operation>('approve')
  const [plan, setPlan] = useState<Plan | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const drafts = useReviewDrafts((s) => s.drafts)
  const selectedDrafts = Object.entries(drafts).filter(
    ([key, draft]) =>
      key.startsWith(`${projectKey}:`) &&
      rows.some((row) => row.key === reviewTargetKey(draft.authorization.target)),
  )
  const fingerprint = JSON.stringify({
    operation,
    rows: rows.map((row) => ({
      target: row.line.target,
      guard: row.scene.guard,
      context: row.line.context_revision,
      state: row.scene.state_json,
    })),
    drafts: selectedDrafts,
  })
  const current = plan?.fingerprint === fingerprint
  const prepare = async () => {
    setBusy(true)
    setError('')
    setPlan(null)
    const requests: ReviewRequest[] = []
    const skipped: ReviewBatchItem[] = []
    for (const row of rows) {
      const draft = selectedDrafts.some(
        ([, draft]) => reviewTargetKey(draft.authorization.target) === row.key,
      )
      if (draft || requests.length >= 128) {
        skipped.push({
          index: -1,
          target: row.line.target,
          status: 'skipped',
          reason: draft
            ? 'This line has local edits in a current or proposed version. Save or discard them first.'
            : 'Only 128 decisions can be reviewed in one batch. Select fewer lines.',
        })
        continue
      }
      const action: ReviewAction =
        operation === 'approve' || operation === 'attest'
          ? { kind: operation }
          : {
              kind: 'policy',
              scope:
                row.line.slot_policy === 'locked' || !row.line.target.variant ? 'slot' : 'variant',
              policy: operation === 'lock' ? 'locked' : 'edited',
            }
      requests.push({
        guard: row.scene.guard,
        target: row.line.target,
        context_revision: row.line.context_revision,
        state_json: row.scene.state_json,
        action,
      })
    }
    try {
      const result = requests.length ? await narrativeReviewBatch(requests, false) : { items: [] }
      setPlan({
        fingerprint,
        requests: requests.filter((_, index) =>
          result.items.some((item) => item.index === index && item.status === 'eligible'),
        ),
        items: [...result.items, ...skipped],
      })
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setBusy(false)
    }
  }
  const apply = async () => {
    if (!plan || !current || !plan.requests.length) return
    setBusy(true)
    setError('')
    try {
      const result = await narrativeReviewBatch(plan.requests, true)
      const decisions = new Map(result.items.map((item) => [reviewTargetKey(item.target), item]))
      setPlan({
        ...plan,
        requests: [],
        items: plan.items.map((item) => decisions.get(reviewTargetKey(item.target)) ?? item),
      })
      onApplied()
    } catch (failure) {
      setError(errorMessage(failure))
      onApplied()
    } finally {
      setBusy(false)
    }
  }
  if (!rows.length)
    return <p>Select lines to review a batch of approval, attestation or policy decisions.</p>
  return (
    <section aria-label="Batch review decisions" className="nrt-review-bulk">
      <div className="nrt-review-toolbar">
        <label className="field">
          Batch action
          <select
            value={operation}
            disabled={busy}
            onChange={(e) => setOperation(e.target.value as Operation)}
          >
            <option value="approve">Approve saved wording</option>
            <option value="attest">Attest unchanged wording</option>
            <option value="lock">Lock wording</option>
            <option value="unlock">Unlock to Edited</option>
          </select>
        </label>
        <button className="btn" disabled={busy || readOnly} onClick={() => void prepare()}>
          Review {rows.length} selected decisions
        </button>
        <button
          className="btn btn-primary"
          disabled={busy || readOnly || !current || !plan?.requests.length}
          onClick={() => void apply()}
        >
          Apply {plan?.requests.length ?? 0} eligible decisions
        </button>
      </div>
      <p>
        Each scene is committed once using its original reviewed revision. Other scenes may succeed
        if one conflicts. Unlock explicitly sets policy to Edited.
      </p>
      {plan && (
        <>
          <p role="status">
            {plan.items.filter((i) => i.status === 'eligible').length} eligible ·{' '}
            {plan.items.filter((i) => i.status === 'skipped').length} skipped ·{' '}
            {plan.items.filter((i) => i.status === 'conflicting').length} conflicting ·{' '}
            {plan.items.filter((i) => i.status === 'applied').length} applied
          </p>
          {!current && (
            <p>
              Selection, source context, action or local drafts changed. Review the batch again
              before applying; the original authorization has not been refreshed.
            </p>
          )}
          <details>
            <summary>Decision details</summary>
            <ul>
              {plan.items.map((item) => (
                <li key={reviewTargetKey(item.target)}>
                  {item.target.slot}: {item.status} — {item.reason}
                </li>
              ))}
            </ul>
          </details>
        </>
      )}
      {error && (
        <p role="alert">
          {error} Refresh the queue to inspect any completed decisions before preparing another
          batch.
        </p>
      )}
    </section>
  )
}
