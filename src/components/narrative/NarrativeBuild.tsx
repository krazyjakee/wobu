import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Modal } from '../Modal'
import { errorMessage, jobCancel, jobList } from '../../lib/api'
import { useNarrativeState } from '../../lib/queries'
import {
  narrativeBuildList,
  narrativeBuildPlan,
  narrativeBuildStart,
  narrativeBuildStatus,
  type BuildAction,
  type BuildScope,
  type BuildItem,
  type NarrativeBuild as Build,
} from '../../lib/api/narrativeBuild'
import type { ContextSelection } from '../../lib/api/narrativeContext'
import './build.css'

const actionLabel: Record<BuildAction, string> = {
  generate: 'Regenerate',
  propose: 'Review proposal',
  locked: 'Locked',
  blocked: 'Blocked',
}
const isActive = (state: string) => ['queued', 'running', 'retrying'].includes(state)

export function NarrativeBuild({
  projectKey,
  readOnly,
  currentScene,
  onClose,
  onScenarios,
  onSource,
}: {
  projectKey: string
  readOnly: boolean
  currentScene?: string
  onClose: () => void
  onScenarios: () => void
  onSource: (target: ContextSelection, asset: boolean) => void
}) {
  const client = useQueryClient()
  const schema = useNarrativeState()
  const [scope, setScope] = useState<BuildScope>('affected')
  const [container, setContainer] = useState('all')
  const [stateText, setStateText] = useState<string | null>(null)
  const [commands, setCommands] = useState('{}')
  const [buildId, setBuildId] = useState<string | null>(null)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [page, setPage] = useState(0)
  const [error, setError] = useState('')
  const list = useQuery({
    queryKey: ['narrative-build-list', projectKey],
    queryFn: narrativeBuildList,
  })
  const jobs = useQuery({
    queryKey: ['narrative-build-jobs', projectKey],
    queryFn: jobList,
    refetchInterval: buildId ? 1500 : false,
  })
  const active = (jobs.data?.jobs ?? []).filter(
    (job) => job.kind === 'narrative' && isActive(job.state),
  )
  const status = useQuery({
    queryKey: ['narrative-build-status', projectKey, buildId],
    queryFn: () => narrativeBuildStatus(buildId!),
    enabled: !!buildId,
    refetchInterval: buildId ? 2500 : false,
  })
  const build = status.data?.build
  const dispatched = new Set(status.data?.dispatched ?? [])
  const decided = new Set(status.data?.decided ?? [])
  const history = new Map(status.data?.history.map((item) => [item.request_id, item]))
  const buildRequests = new Set(build?.items.map((item) => item.request_id))
  const running = active.filter((job) => buildRequests.has(job.subjectId ?? ''))
  const runningByRequest = new Map(running.map((job) => [job.subjectId, job]))
  const jobsByRequest = new Map((jobs.data?.jobs ?? []).map((job) => [job.subjectId, job]))
  const defaults = JSON.stringify(
    Object.fromEntries((schema.data?.document.variables ?? []).map((v) => [v.name, v.default])),
  )
  const run = async (action: () => Promise<void>) => {
    setBusy(true)
    setError('')
    try {
      await action()
      await list.refetch()
      await jobs.refetch()
      await client.invalidateQueries({ queryKey: ['narrative-build-status', projectKey] })
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setBusy(false)
    }
  }
  const open = (next: Build) => {
    setBuildId(next.id)
    setPage(0)
    setSelected(new Set(next.items.filter((item) => item.request_id).map((item) => item.id)))
    client.setQueryData(['narrative-build-status', projectKey, next.id], {
      build: next,
      history: [],
      dispatched: [],
      decided: [],
    })
  }
  const plan = () =>
    run(async () => {
      const containers = container === 'current' && currentScene ? [currentScene] : []
      const source = `{"scope":${JSON.stringify(scope)},"containers":${JSON.stringify(containers)},"state":${stateText ?? defaults},"commands":${commands},"token_budget":4000,"max_output_tokens":2048}`
      open(await narrativeBuildPlan(source))
    })
  const failed =
    build?.items.filter((item) => {
      const previous = item.request_id ? history.get(item.request_id) : undefined
      return (
        (previous && ['failed', 'invalid_output', 'cancelled'].includes(previous.status)) ||
        (previous?.status !== 'succeeded' && jobsByRequest.get(item.request_id)?.state === 'failed')
      )
    }) ?? []
  const isDone = (item: BuildItem) => {
    const previous = item.request_id ? history.get(item.request_id) : undefined
    return (
      previous?.status === 'succeeded' &&
      previous.proposal_published &&
      (item.action !== 'generate' ||
        decided.has(previous.receipt_id ?? '') ||
        previous.proposal_current_at_publication === false) &&
      (!item.reusable || dispatched.has(item.request_id ?? ''))
    )
  }
  const completed = build?.items.filter(isDone).length ?? 0
  const ready =
    build?.items.filter(
      (item) => item.request_id && !runningByRequest.has(item.request_id) && !isDone(item),
    ) ?? []
  const chosen = ready.filter((item) => selected.has(item.id))

  return (
    <Modal
      className="sheet nrt-build-sheet"
      onClose={onClose}
      busy={busy}
      titleId="nrt-build-title"
      descriptionId="nrt-build-description"
    >
      <section className="nrt-build">
        <header>
          <h2 id="nrt-build-title">Build · Affected content</h2>
          <button className="btn" onClick={onScenarios} disabled={busy}>
            Scenario tests
          </button>
        </header>
        <p id="nrt-build-description">
          Plan saved content without a provider key. Generation is explicit; validation and export
          use accepted wording.
        </p>
        <div className="nrt-build-controls">
          <label>
            Work to plan
            <select
              value={scope}
              onChange={(e) => setScope(e.target.value as BuildScope)}
              disabled={busy}
            >
              <option value="affected">Affected content</option>
              <option value="missing">Missing wording</option>
              <option value="all_selected">All in selected scope</option>
            </select>
          </label>
          <label>
            Selected scope
            <select
              value={container}
              onChange={(e) => setContainer(e.target.value)}
              disabled={busy}
            >
              <option value="all">Whole project</option>
              {currentScene && <option value="current">Current scene</option>}
            </select>
          </label>
          <button
            className="btn"
            disabled={busy || readOnly || schema.isLoading}
            onClick={() => void plan()}
          >
            Plan work
          </button>
        </div>
        <details>
          <summary>Scenario state and host command signatures</summary>
          <label>
            State (JSON)
            <textarea
              value={stateText ?? defaults}
              onChange={(e) => setStateText(e.target.value)}
            />
          </label>
          <label>
            Command signatures (JSON)
            <textarea value={commands} onChange={(e) => setCommands(e.target.value)} />
          </label>
        </details>
        {!!list.data?.length && (
          <label>
            Resume a saved build
            <select
              value={buildId ?? ''}
              disabled={busy}
              onChange={(e) => {
                if (e.target.value)
                  void run(async () => {
                    const saved = await narrativeBuildStatus(e.target.value)
                    open(saved.build)
                    client.setQueryData(
                      ['narrative-build-status', projectKey, saved.build.id],
                      saved,
                    )
                  })
              }}
            >
              <option value="">Choose a build</option>
              {list.data.map((item) => (
                <option value={item.id} key={item.id}>
                  {item.id.slice(-8)} · {item.scope.replaceAll('_', ' ')} · {item.items} items ·{' '}
                  {item.model}
                </option>
              ))}
            </select>
          </label>
        )}
        {busy && (
          <p role="status">Preparing saved build work… Completed results remain retained.</p>
        )}
        {readOnly && <p>This project is read-only. Retained builds can be inspected.</p>}
        {(error || status.error || list.error) && (
          <p role="alert">{error || errorMessage(status.error ?? list.error)}</p>
        )}
        {build && (
          <>
            <p>
              {build.provider} / {build.model} · {build.items.length} items · {completed} completed
              · {running.length} active
            </p>
            <p aria-label="Policy counts">
              {(['generate', 'propose', 'locked', 'blocked'] as const)
                .map(
                  (action) =>
                    `${build.items.filter((item) => item.action === action).length} ${actionLabel[action]}`,
                )
                .join(' · ')}
              {' · '}
              {build.items.filter((item) => item.reusable).length} reusable
            </p>
            {build.diagnostics.map((message, i) => (
              <p key={i}>{message}</p>
            ))}
            <div className="nrt-build-controls">
              <button
                className="btn"
                onClick={() => setSelected(new Set(ready.map((item) => item.id)))}
              >
                Select eligible
              </button>
              <button className="btn" onClick={() => setSelected(new Set())}>
                Clear selection
              </button>
              <button
                className="btn"
                disabled={!failed.length}
                onClick={() => setSelected(new Set(failed.map((item) => item.id)))}
              >
                Select failed items
              </button>
            </div>
            <nav className="nrt-build-controls" aria-label="Build pages">
              <button className="btn" disabled={page === 0} onClick={() => setPage(page - 1)}>
                Previous page
              </button>
              <span>
                {Math.min(page * 50 + 1, build.items.length)}–
                {Math.min((page + 1) * 50, build.items.length)} of {build.items.length}
              </span>
              <button
                className="btn"
                disabled={(page + 1) * 50 >= build.items.length}
                onClick={() => setPage(page + 1)}
              >
                Next page
              </button>
            </nav>
            <div className="nrt-build-table">
              <table>
                <thead>
                  <tr>
                    <th>Select</th>
                    <th>Content and reason</th>
                    <th>Action</th>
                    <th>Progress</th>
                  </tr>
                </thead>
                <tbody>
                  {build.items.slice(page * 50, (page + 1) * 50).map((item) => {
                    const previous = item.request_id ? history.get(item.request_id) : undefined
                    const live = runningByRequest.get(item.request_id)
                    const lastJob = jobsByRequest.get(item.request_id)
                    const done = isDone(item)
                    return (
                      <tr key={item.id}>
                        <td>
                          <input
                            type="checkbox"
                            aria-label={`Select ${item.label}`}
                            checked={selected.has(item.id) && !done}
                            disabled={!item.request_id || done || !!live || busy}
                            onChange={(e) =>
                              setSelected((current) => {
                                const next = new Set(current)
                                if (e.target.checked) next.add(item.id)
                                else next.delete(item.id)
                                return next
                              })
                            }
                          />
                        </td>
                        <td>
                          <button
                            className="btn-link"
                            onClick={() => onSource(item.target, item.asset)}
                          >
                            {item.label}
                          </button>
                          {(item.reasons.length > 0 || item.diagnostics.length > 0) && (
                            <details>
                              <summary>Why included</summary>
                              {item.reasons.map((reason, i) => (
                                <p key={i}>
                                  {reason.source} → {reason.context}: {reason.message}
                                </p>
                              ))}
                              {item.diagnostics.map((message, i) => (
                                <p key={i}>{message}</p>
                              ))}
                            </details>
                          )}
                        </td>
                        <td>
                          {actionLabel[item.action]}
                          {item.reusable && ' · Reuse stored result'}
                        </td>
                        <td>
                          {live?.state ??
                            (done
                              ? previous?.proposal_current_at_publication === false
                                ? 'Retained for review: source changed'
                                : 'Completed'
                              : previous?.status === 'succeeded'
                                ? item.reusable
                                  ? 'Stored result ready'
                                  : previous.proposal_published
                                    ? 'Resume acceptance; no provider call'
                                    : 'Recover publication'
                                : previous?.attempts
                                  ? previous.status
                                  : item.request_id
                                    ? dispatched.has(item.request_id)
                                      ? 'Interrupted; charge unknown; explicit resume required'
                                      : 'Ready'
                                    : 'Excluded')}
                          {lastJob?.state === 'failed' && !done && <p>{lastJob.failure.message}</p>}
                          {previous?.billing_unknown && previous.attempts > 0 && (
                            <p>Previous charge unknown</p>
                          )}
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </div>
            <p>
              Finished results are retained. Retrying failed or interrupted calls may incur another
              charge. Nothing resumes automatically after restart.
            </p>
            <div className="nrt-build-controls">
              <button
                className="btn btn-primary"
                disabled={busy || readOnly || !chosen.length}
                onClick={() =>
                  void run(async () => {
                    await narrativeBuildStart(
                      build.id,
                      chosen.map((item) => item.id),
                    )
                  })
                }
              >
                Generate / resume {chosen.length} selected
              </button>
              <button
                className="btn"
                disabled={busy || !running.length}
                onClick={() =>
                  void run(async () => {
                    await Promise.all(running.map((job) => jobCancel(job.id)))
                  })
                }
              >
                Cancel active items
              </button>
            </div>
          </>
        )}
        <button className="btn" onClick={onClose} disabled={busy}>
          Close
        </button>
      </section>
    </Modal>
  )
}
