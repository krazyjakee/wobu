import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Modal } from '../Modal'
import { errorMessage, jobCancel, jobList, type Scene } from '../../lib/api'
import { useNarrativeState } from '../../lib/queries'
import type { ContextSelection } from '../../lib/api/narrativeContext'
import {
  narrativeGenerationHistory,
  narrativeGenerationPlan,
  narrativeGenerationRecover,
  narrativeGenerationRetry,
  narrativeGenerationStart,
  type GenerationPlan,
} from '../../lib/api/narrativeGeneration'
import './export.css'
import './generation.css'

export function NarrativeGeneration({
  scene,
  readOnly,
  onClose,
  projectKey,
}: {
  scene?: Scene
  projectKey: string
  readOnly: boolean
  onClose: () => void
}) {
  const schema = useNarrativeState()
  const [target, setTarget] = useState('missing')
  const [stateText, setStateText] = useState<string | null>(null)
  const [commands, setCommands] = useState('{}')
  const [plan, setPlan] = useState<GenerationPlan | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const history = useQuery({
    queryKey: ['narrative-generation-history', projectKey],
    queryFn: narrativeGenerationHistory,
    refetchInterval: 1500,
  })
  const jobs = useQuery({
    queryKey: ['narrative-generation-jobs', projectKey],
    queryFn: jobList,
    refetchInterval: 1000,
  })
  const lines: { label: string; selection: ContextSelection }[] = []
  for (const beat of scene?.beats ?? []) {
    for (const slot of beat.dialogue ?? []) {
      const base = { scene: scene!.id, beat: beat.id, slot: slot.id }
      if (!slot.variants?.length)
        lines.push({
          label: `${beat.title} · empty line ${slot.id.slice(-6)}`,
          selection: { ...base, variant: null },
        })
      for (const variant of slot.variants ?? [])
        lines.push({
          label: `${beat.title} · ${variant.text.body.slice(0, 48) || 'Empty variant'} · ${variant.text.lifecycle?.policy ?? 'edited'}`,
          selection: { ...base, variant: variant.id },
        })
    }
  }
  const defaults = JSON.stringify(
    Object.fromEntries((schema.data?.document.variables ?? []).map((v) => [v.name, v.default])),
    null,
    2,
  )
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true)
    setError('')
    try {
      await action()
      await history.refetch()
      await jobs.refetch()
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setBusy(false)
    }
  }
  const active = (jobs.data?.jobs ?? []).filter(
    (job) =>
      job.kind === 'narrative' &&
      history.data?.some((item) => item.request_id === job.subjectId) &&
      ['queued', 'running', 'retrying'].includes(job.state),
  )
  const preview = () =>
    run(async () => {
      if (!scene) throw new Error('Open a scene before planning generation.')
      const selected = target === 'missing' ? null : lines[Number(target)]?.selection
      if (target !== 'missing' && !selected) throw new Error('Choose a current dialogue variant.')
      // Preserve authored numeric spelling until Rust parses both state and command domains.
      const source = `{"scene":${JSON.stringify(scene.id)},"selection":${JSON.stringify(selected)},"state":${stateText ?? defaults},"commands":${commands},"token_budget":4000,"max_output_tokens":2048}`
      setPlan(await narrativeGenerationPlan(source))
    })
  return (
    <Modal
      onClose={onClose}
      busy={busy}
      titleId="nrt-generation-title"
      descriptionId="nrt-generation-description"
    >
      <section className="nrt-export nrt-generation">
        <h2 id="nrt-generation-title">Generate dialogue</h2>
        <p id="nrt-generation-description">
          Generate prose for saved dialogue slots. Slots and variants that are both Generated may
          update automatically. Edited wording produces a review proposal; Locked wording is
          skipped. Validated results are retained in Review.
        </p>
        <p>
          Uses the text provider and model from Settings. Provider charges may apply; Wobu has no
          account balance or exact cost estimate.
        </p>
        {readOnly && <p>This project is read-only. Generation requires write access.</p>}
        <label>
          Generation selection
          <select
            disabled={busy || readOnly || !scene}
            value={target}
            onChange={(event) => {
              setTarget(event.target.value)
              setPlan(null)
            }}
          >
            <option value="missing">Generate missing in {scene?.name ?? 'an open scene'}</option>
            {lines.map((line, index) => (
              <option key={`${line.selection.slot}-${line.selection.variant}`} value={index}>
                {line.label}
              </option>
            ))}
          </select>
        </label>
        <details>
          <summary>Scenario state and host command declarations</summary>
          <p>
            These inputs resolve knowledge and conditions for this batch. Changes to inputs require
            a new plan.
          </p>
          <label>
            Generation state (JSON)
            <textarea
              value={stateText ?? defaults}
              disabled={busy}
              onChange={(event) => {
                setStateText(event.target.value)
                setPlan(null)
              }}
            />
          </label>
          <label>
            Generation command signatures (JSON)
            <textarea
              value={commands}
              disabled={busy}
              onChange={(event) => {
                setCommands(event.target.value)
                setPlan(null)
              }}
            />
          </label>
        </details>
        <div className="nrt-export-actions">
          <button
            className="btn"
            disabled={busy || readOnly || !scene || schema.isLoading}
            onClick={() => void preview()}
          >
            Plan generation
          </button>
          <button className="btn" disabled={busy} onClick={onClose}>
            Close
          </button>
        </div>
        {plan && (
          <section className="nrt-generation-plan" aria-label="Frozen generation plan">
            <h3>
              {plan.requests.length} {plan.requests.length === 1 ? 'request' : 'requests'} ·{' '}
              {plan.skipped.length} skipped
            </h3>
            <p>
              {plan.provider} / {plan.model} · one line per request · at most 32 per batch
            </p>
            <p>
              Only an explicit rate-limit rejection retries automatically. Failed, cancelled or
              interrupted calls may have been billed; retrying them is your choice.
            </p>
            <ul>
              {plan.requests.map((request) => (
                <li key={request.request_id}>
                  <code>
                    {request.target.slot.slice(-8)} / {request.candidate_variant_id.slice(-8)}
                  </code>{' '}
                  · {request.expected_policy ?? 'empty slot'} · ~{request.context.estimated_tokens}{' '}
                  estimated input tokens
                  <details>
                    <summary>Frozen request and context</summary>
                    <pre>{request.prompt}</pre>
                  </details>
                </li>
              ))}
            </ul>
            {plan.skipped.map((item) => (
              <p key={`${item.target.slot}-${item.target.variant}`}>
                {item.target.slot.slice(-8)}: {item.reason}
              </p>
            ))}
            <button
              className="btn btn-primary"
              disabled={busy || readOnly || !plan.requests.length}
              onClick={() =>
                void run(async () => {
                  await narrativeGenerationStart(plan.id)
                  setPlan(null)
                })
              }
            >
              Queue {plan.requests.length} provider{' '}
              {plan.requests.length === 1 ? 'request' : 'requests'}
            </button>
          </section>
        )}
        <div className="nrt-export-actions">
          <h3>Generation history</h3>
          <button
            className="btn"
            disabled={busy || !active.length}
            onClick={() =>
              void run(async () => {
                await Promise.all(active.map((job) => jobCancel(job.id)))
              })
            }
          >
            Cancel active generation
          </button>
        </div>
        <ul className="nrt-generation-history">
          {(history.data ?? []).map((item) => {
            const matchingJobs =
              jobs.data?.jobs.filter(
                (one) => one.subjectId === item.request_id && one.kind === 'narrative',
              ) ?? []
            // Queue snapshots retain older terminal attempts in submission order.
            const job =
              matchingJobs.find((one) => ['queued', 'running', 'retrying'].includes(one.state)) ??
              matchingJobs.at(-1)
            const running = job && ['queued', 'running', 'retrying'].includes(job.state)
            const status = running
              ? job.state
              : item.status === 'interrupted' && job?.state === 'cancelled'
                ? 'cancelled before starting'
                : item.status
            return (
              <li key={item.request_id}>
                <h4>
                  {item.target.slot.slice(-8)} · {status.replaceAll('_', ' ')}
                </h4>
                <p>
                  {item.provider} / {item.model} · {item.attempts} recorded attempts ·{' '}
                  {item.usage.input} input + {item.usage.output} output tokens reported
                </p>
                {!running && item.billing_unknown && (
                  <p>
                    Billing is unknown. An interrupted request is never restarted automatically.
                  </p>
                )}
                {item.error_code && <p>{item.error_code}</p>}
                {item.candidate && <blockquote>{item.candidate.text}</blockquote>}
                {item.proposal_published && <p>Result retained in Review.</p>}
                {item.proposal_current_at_publication === false && (
                  <p>
                    Source, context or policy changed during generation. This proposal requires
                    conflict review.
                  </p>
                )}
                {running ? (
                  <button
                    className="btn"
                    disabled={busy}
                    onClick={() => void run(() => jobCancel(job.id))}
                  >
                    Cancel request
                  </button>
                ) : item.status === 'succeeded' ? (
                  !item.proposal_published && (
                    <button
                      className="btn"
                      disabled={busy || readOnly}
                      onClick={() => void run(() => narrativeGenerationRecover(item.request_id))}
                    >
                      Recover retained result — no provider call
                    </button>
                  )
                ) : (
                  <button
                    className="btn"
                    disabled={busy || readOnly}
                    onClick={() => void run(() => narrativeGenerationRetry(item.request_id))}
                  >
                    Retry request — may incur charges
                  </button>
                )}
              </li>
            )
          })}
        </ul>
        {!history.isLoading && !history.data?.length && (
          <p>No generation requests have been queued for this project.</p>
        )}
        {(error || history.error || jobs.error) && (
          <p role="alert">{error || errorMessage(history.error ?? jobs.error)}</p>
        )}
      </section>
    </Modal>
  )
}
