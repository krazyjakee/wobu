import { useMemo, useRef, useState, type ReactNode } from 'react'
import { Modal } from '../../Modal'
import type {
  ReviewContext,
  ReviewRequest,
  ReviewSceneView,
  ReviewTarget,
} from '../../../lib/api/narrativeReview'
import type { GenerationHistory } from '../../../lib/api/narrativeGeneration'
import { ReviewDetail } from './ReviewDetail'
import {
  EMPTY_REVIEW_FILTERS,
  filterReviewRows,
  reviewPolicy,
  reviewTargetKey,
  type ReviewFilters,
  type ReviewRow,
} from './reviewModel'
import { useReviewDrafts } from './reviewDrafts'
import './review.css'

const PAGE_SIZE = 50
export function ReviewQueue({
  projectKey,
  readOnly,
  views,
  sceneName,
  speakerName,
  loading,
  error,
  generationHistory = [],
  onRefresh,
  onApply,
  onContext,
  onSource,
  onClose,
  renderBulk,
}: {
  projectKey: string
  readOnly: boolean
  views: ReviewSceneView[]
  sceneName: (id: string) => string
  speakerName: (id: string) => string
  loading: boolean
  error: string
  generationHistory?: GenerationHistory[]
  onRefresh: (stateJson: string | null) => Promise<void>
  onApply: (request: ReviewRequest) => Promise<void>
  onContext: (target: ReviewTarget, stateJson: string) => Promise<ReviewContext>
  onSource: (target: ReviewTarget) => void
  onClose: () => void
  renderBulk?: (rows: ReviewRow[]) => ReactNode
}) {
  const [filters, setFilters] = useState<ReviewFilters>(EMPTY_REVIEW_FILTERS)
  const [selection, setSelection] = useState<string | null>(null)
  const [proposals, setProposals] = useState<Record<string, string | null>>({})
  const [checked, setChecked] = useState<string[]>([])
  const [page, setPage] = useState(0)
  const [stateJson, setStateJson] = useState<string | null>(null)
  const buttons = useRef(new Map<string, HTMLButtonElement>())
  const drafts = useReviewDrafts((state) => state.drafts)
  const rows = useMemo(
    () =>
      views.flatMap((scene) =>
        scene.lines.map((line): ReviewRow => ({
          key: reviewTargetKey(line.target),
          scene,
          line,
          sceneName: sceneName(scene.scene_id),
          speakerKey: typeof line.speaker === 'string' ? line.speaker : line.speaker.entity,
          speakerName:
            typeof line.speaker === 'string'
              ? line.speaker === 'player'
                ? 'Player'
                : 'Narrator'
              : speakerName(line.speaker.entity),
        })),
      ),
    [views, sceneName, speakerName],
  )
  const filtered = filterReviewRows(rows, filters)
  const current =
    rows.find((row) => row.key === selection) ?? (selection === null ? filtered[0] : undefined)
  const selectedProposal = current
    ? proposals[current.key] === undefined
      ? (current.line.proposals.find((proposal) => proposal.status === 'pending')?.id ?? null)
      : proposals[current.key]!
    : null
  const lastPage = Math.max(0, Math.ceil(filtered.length / PAGE_SIZE) - 1)
  const currentPage = Math.min(page, lastPage)
  const visible = filtered.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE)
  const selectedRows = rows.filter((row) => checked.includes(row.key))
  const retained = Object.entries(drafts).filter(([key]) => key.startsWith(`${projectKey}:`))
  const orphaned = retained.filter(
    ([, draft]) => !rows.some((row) => row.key === reviewTargetKey(draft.authorization.target)),
  )
  const update = (patch: Partial<ReviewFilters>) => {
    setFilters((before) => ({ ...before, ...patch }))
    setPage(0)
  }
  const choose = (row: ReviewRow) => {
    setSelection(row.key)
  }
  const move = (key: string, offset: number) => {
    const index = filtered.findIndex((row) => row.key === key)
    const next = filtered[Math.max(0, Math.min(filtered.length - 1, index + offset))]
    if (!next) return
    setSelection(next.key)
    setPage(Math.floor(filtered.indexOf(next) / PAGE_SIZE))
    requestAnimationFrame(() => buttons.current.get(next.key)?.focus())
  }
  return (
    <Modal
      onClose={onClose}
      className="sheet nrt-review-sheet"
      titleId="nrt-review-title"
      descriptionId="nrt-review-description"
    >
      <section className="nrt-review">
        <header className="nrt-review-heading">
          <div>
            <h1 id="nrt-review-title">Dialogue review</h1>
            <p id="nrt-review-description">
              Compare drafts, protect wording, and record approval against the context you reviewed.
            </p>
          </div>
          <button className="btn" onClick={onClose}>
            Close review
          </button>
        </header>
        {readOnly && (
          <p>Read-only project. Wording, provenance and history remain available for inspection.</p>
        )}
        <div className="nrt-review-filters">
          <label className="field">
            Search review queue
            <input
              type="search"
              value={filters.query}
              onChange={(event) => update({ query: event.target.value })}
            />
          </label>
          <label className="field">
            Scene
            <select
              value={filters.scene}
              onChange={(event) => update({ scene: event.target.value })}
            >
              <option value="">All scenes</option>
              {views.map((view) => (
                <option key={view.scene_id} value={view.scene_id}>
                  {sceneName(view.scene_id)}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            Speaker
            <select
              value={filters.speaker}
              onChange={(event) => update({ speaker: event.target.value })}
            >
              <option value="">All speakers</option>
              {[...new Map(rows.map((row) => [row.speakerKey, row.speakerName])).entries()].map(
                ([key, name]) => (
                  <option key={key} value={key}>
                    {name}
                  </option>
                ),
              )}
            </select>
          </label>
          <label className="field">
            Policy
            <select
              value={filters.policy}
              onChange={(event) =>
                update({ policy: event.target.value as ReviewFilters['policy'] })
              }
            >
              <option value="">All policies</option>
              <option value="generated">Generated</option>
              <option value="edited">Edited</option>
              <option value="locked">Locked</option>
            </select>
          </label>
          <label className="field">
            Approval
            <select
              value={filters.approval}
              onChange={(event) =>
                update({ approval: event.target.value as ReviewFilters['approval'] })
              }
            >
              <option value="">All approvals</option>
              <option value="draft">Draft</option>
              <option value="approved">Valid approval</option>
              <option value="invalid">Invalidated approval</option>
            </select>
          </label>
          <label className="field">
            Freshness
            <select
              value={filters.freshness}
              onChange={(event) =>
                update({ freshness: event.target.value as ReviewFilters['freshness'] })
              }
            >
              <option value="">All freshness</option>
              <option value="current">Current</option>
              <option value="out_of_date">Out of date</option>
            </select>
          </label>
        </div>
        <div className="nrt-review-toolbar">
          <p>
            {filtered.length} matching lines · {checked.length} selected · {retained.length} local
            edits retained
          </p>
          <button className="btn" disabled={loading} onClick={() => void onRefresh(stateJson)}>
            Refresh queue
          </button>
          <button
            className="btn"
            onClick={() => {
              setFilters(EMPTY_REVIEW_FILTERS)
              setPage(0)
            }}
          >
            Clear filters
          </button>
        </div>
        <details className="nrt-review-scenario">
          <summary>Scenario used to review conditions and context</summary>
          <p>
            Inspecting a different scenario refreshes saved context. Local edits keep their original
            reviewed scenario and guards.
          </p>
          <label className="field">
            Review scenario state (JSON)
            <textarea
              aria-label="Review scenario state (JSON)"
              value={stateJson ?? views[0]?.state_json ?? '{}'}
              onChange={(event) => setStateJson(event.target.value)}
            />
          </label>
          <button className="btn" disabled={loading} onClick={() => void onRefresh(stateJson)}>
            Inspect review scenario
          </button>
        </details>
        {renderBulk?.(selectedRows)}
        {loading && <p role="status">Reading saved narrative review records…</p>}
        {error && <p role="alert">{error}</p>}
        <div className="nrt-review-body">
          <section className="nrt-review-list" aria-label="Project review queue">
            <div className="nrt-review-toolbar">
              <button
                className="btn"
                disabled={!visible.length}
                onClick={() =>
                  setChecked((before) => [
                    ...new Set([...before, ...visible.map((row) => row.key)]),
                  ])
                }
              >
                Select this page
              </button>
              <button className="btn" disabled={!checked.length} onClick={() => setChecked([])}>
                Clear selection
              </button>
            </div>
            <ul>
              {visible.map((row) => (
                <li key={row.key} className={current?.key === row.key ? 'is-selected' : ''}>
                  <input
                    type="checkbox"
                    aria-label={`Select ${row.speakerName} ${row.line.target.slot}`}
                    checked={checked.includes(row.key)}
                    onChange={(event) =>
                      setChecked((before) =>
                        event.target.checked
                          ? [...before, row.key]
                          : before.filter((key) => key !== row.key),
                      )
                    }
                  />
                  <button
                    ref={(node) => {
                      if (node) buttons.current.set(row.key, node)
                      else buttons.current.delete(row.key)
                    }}
                    className="nrt-review-open"
                    aria-label={`${row.speakerName} ${row.sceneName}: ${row.line.target.slot}`}
                    aria-current={current?.key === row.key ? 'true' : undefined}
                    onClick={() => choose(row)}
                    onKeyDown={(event) => {
                      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
                        event.preventDefault()
                        move(row.key, event.key === 'ArrowDown' ? 1 : -1)
                      }
                    }}
                  >
                    <strong>{row.speakerName}</strong>
                    <span>{row.sceneName}</span>
                    <span>
                      {row.line.text?.body ||
                        row.line.proposals.find((p) => p.status === 'pending')?.candidate.text ||
                        'No accepted wording'}
                    </span>
                    <small>
                      {reviewPolicy(row.line)} · {row.line.review} ·{' '}
                      {row.line.freshness === 'out_of_date' ? 'Out of date' : 'Current'}
                    </small>
                    <small>
                      {
                        row.line.proposals.filter((proposal) => proposal.status === 'pending')
                          .length
                      }{' '}
                      pending proposals
                    </small>
                  </button>
                </li>
              ))}
            </ul>
            {!loading && !filtered.length && (
              <p>
                {rows.length
                  ? 'No lines match these filters. Clear a filter to see the rest of the queue.'
                  : 'No dialogue is available for review. Write a scene or generate a draft, then refresh.'}
              </p>
            )}
            <nav aria-label="Review queue pages">
              <button
                className="btn"
                disabled={currentPage === 0}
                onClick={() => setPage(currentPage - 1)}
              >
                Previous page
              </button>
              <span>
                Page {currentPage + 1} of {lastPage + 1}
              </span>
              <button
                className="btn"
                disabled={currentPage === lastPage}
                onClick={() => setPage(currentPage + 1)}
              >
                Next page
              </button>
            </nav>
          </section>
          {current ? (
            <ReviewDetail
              key={`${projectKey}:${current.key}:${selectedProposal ?? 'current'}`}
              row={current}
              projectKey={projectKey}
              readOnly={readOnly}
              proposalId={selectedProposal}
              onProposal={(id) => setProposals((before) => ({ ...before, [current.key]: id }))}
              onApply={onApply}
              onContext={(state) => onContext(current.line.target, state)}
              onSource={() => onSource(current.line.target)}
              provenance={generationHistory.find(
                (item) =>
                  item.request_id ===
                  current.line.proposals.find((proposal) => proposal.id === selectedProposal)
                    ?.request_id,
              )}
            />
          ) : (
            <p className="nrt-review-empty">
              Choose a line to compare its wording and review context.
            </p>
          )}
        </div>
        {!!orphaned.length && (
          <section aria-label="Retained edits for removed lines">
            <h2>Retained edits for removed lines</h2>
            {orphaned.map(([key, draft]) => (
              <article key={key}>
                <p>
                  The source line is no longer in the current queue. Your writing is retained here
                  for copying or recovery.
                </p>
                <textarea
                  aria-label={`Retained wording ${draft.authorization.target.slot}`}
                  readOnly
                  value={draft.body}
                />
                <button className="btn" onClick={() => useReviewDrafts.getState().clear(key)}>
                  Discard retained edit
                </button>
              </article>
            ))}
          </section>
        )}
      </section>
    </Modal>
  )
}
