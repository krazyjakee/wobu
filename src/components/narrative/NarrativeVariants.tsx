import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Modal } from '../Modal'
import { errorMessage, type Scene } from '../../lib/api'
import { useMaterializeVariants } from '../../lib/queries/narrativeVariants'
import { assertProjectSession, projectSessionEpoch } from '../../lib/projectSession'
import {
  narrativeVariantsGet,
  narrativeVariantsPlan,
  narrativeVariantsBuild,
  type AnalysisReport,
} from '../../lib/api/narrativeVariants'
import './variants.css'
export function NarrativeVariants({
  projectKey,
  scene,
  beatId,
  readOnly,
  onClose,
  onBuild,
}: {
  projectKey: string
  scene: Scene
  beatId: string
  readOnly: boolean
  onClose: () => void
  onBuild: (id: string) => void
}) {
  const materialize = useMaterializeVariants()
  const beat = scene.beats?.find((b) => b.id === beatId)
  const query = useQuery({
    queryKey: ['narrative-variants', projectKey, scene.id, beatId],
    queryFn: () => narrativeVariantsGet(scene.id, beatId),
  })
  const [policy, setPolicy] = useState<string | null>(null)
  const [saved, setSaved] = useState<AnalysisReport | null>(null)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [slot, setSlot] = useState(beat?.dialogue?.[0]?.id ?? '')
  const [page, setPage] = useState(0)
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')
  const analysis = saved ?? query.data?.latest
  const report = analysis?.report
  const stale = !saved && query.data?.stale
  const rows = report?.rows ?? []
  const eligible = rows.filter((r) => r.classification === 'included')
  const pages = Math.max(1, Math.ceil(rows.length / 25))
  const currentSlot = beat?.dialogue?.find((s) => s.id === slot)
  const locked =
    currentSlot?.policy === 'locked' ||
    currentSlot?.variants?.some((v) => v.text.lifecycle?.policy === 'locked')
  const run = async (label: string, action: (epoch: number) => Promise<void>) => {
    setBusy(label)
    setError('')
    try {
      await action(projectSessionEpoch())
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setBusy('')
    }
  }
  return (
    <Modal
      titleId="variants-title"
      descriptionId="variants-description"
      className="sheet narrative-variants-modal"
      onClose={onClose}
    >
      <div className="narrative-variants">
        <header>
          <h2 id="variants-title">Variant matrix · {beat?.title ?? 'Beat'}</h2>
          <button className="btn" onClick={onClose}>
            Close
          </button>
        </header>
        <p id="variants-description">
          Plan configurations without a provider key. Included rows have witnesses under an
          explicitly declared state model. Runtime routes are not verified.
        </p>
        {query.isPending && <p role="status">Loading canonical policy and saved matrix…</p>}
        {(error || query.error) && <p role="alert">{error || errorMessage(query.error)}</p>}
        {query.data && (
          <details>
            <summary>Explicit analysis policy and limits</summary>
            <p>
              Declare complete initial states, invariants, quest-to-enum bindings and event effects.
              World prose supplies no effects. Keep external input enabled unless this transition
              model is explicitly closed.
            </p>
            <label>
              Policy JSON
              <textarea
                aria-label="Analysis policy JSON"
                spellCheck={false}
                value={policy ?? JSON.stringify(query.data.policy, null, 2)}
                onChange={(e) => setPolicy(e.target.value)}
                disabled={readOnly || !!busy}
              />
            </label>
          </details>
        )}
        <div className="variants-actions">
          <button
            className="btn"
            disabled={!query.data || readOnly || !!busy}
            onClick={() =>
              void run('Planning within the configured bounds…', async (epoch) => {
                const result = await narrativeVariantsPlan(
                  policy ?? JSON.stringify(query.data!.policy),
                  query.data!.capture.guard,
                )
                assertProjectSession(epoch)
                setSaved(result)
                setSelected(new Set())
                setPage(0)
                await query.refetch()
              })
            }
          >
            Save policy and plan
          </button>
          <button
            className="btn"
            disabled={!!busy}
            onClick={() => {
              setSaved(null)
              setPolicy(null)
              setSelected(new Set())
              void query.refetch()
            }}
          >
            Reload saved matrix
          </button>
        </div>
        {busy && <p role="status">{busy}</p>}
        {stale && (
          <p role="status">
            Sources or policy changed since this saved matrix. Replan before materializing.
          </p>
        )}
        {report && (
          <>
            <p className="variants-counts">
              Potential {report.potential.exact ? '' : '≥'}
              {report.potential.value} · Included {report.included.value} · Excluded{' '}
              {report.excluded.value} · Unknown {report.unknown.exact ? '' : '≥'}
              {report.unknown.value}
            </p>
            <p>
              Explored {report.explored_states} states. Limits: {report.limits.states} states,{' '}
              {report.limits.variants} rows, {report.limits.milliseconds} ms;{' '}
              {report.limits.witness_steps ?? 20000} retained witness steps.
            </p>
            <details>
              <summary>Variable buckets and proof assumptions</summary>
              <ul>
                {report.domains.map((d) => (
                  <li key={d.name}>
                    {d.name}: {JSON.stringify(d.ty)}. Integer values are exact singleton buckets.
                  </li>
                ))}
              </ul>
              {report.assumptions.map((a) => (
                <p key={a}>{a}</p>
              ))}
              {report.reasons.map((reason) => (
                <p key={reason}>{reason}</p>
              ))}
            </details>
            <div className="variants-actions">
              <button
                className="btn"
                onClick={() => setSelected(new Set(eligible.map((r) => r.id)))}
                disabled={!!busy || readOnly}
              >
                Select all included ({eligible.length})
              </button>
              <button className="btn" onClick={() => setSelected(new Set())}>
                Clear selection
              </button>
              <label>
                Target line
                <select
                  aria-label="Target line"
                  value={slot}
                  onChange={(e) => setSlot(e.target.value)}
                >
                  {beat?.dialogue?.map((s, i) => (
                    <option key={s.id} value={s.id}>
                      Line {i + 1} · {s.policy}
                    </option>
                  ))}
                </select>
              </label>
            </div>
            <div className="variants-table">
              <table>
                <thead>
                  <tr>
                    <th>Select</th>
                    <th>Configuration</th>
                    <th>Result and evidence</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.slice(page * 25, (page + 1) * 25).map((row) => (
                    <tr key={row.id}>
                      <td>
                        <input
                          type="checkbox"
                          aria-label={`Select configuration ${row.id}`}
                          checked={selected.has(row.id)}
                          disabled={readOnly || !!busy || row.classification !== 'included'}
                          onChange={(e) =>
                            setSelected((old) => {
                              const next = new Set(old)
                              if (e.target.checked) next.add(row.id)
                              else next.delete(row.id)
                              return next
                            })
                          }
                        />
                      </td>
                      <td>
                        <code>{JSON.stringify(row.values)}</code>
                        <small>{row.id}</small>
                      </td>
                      <td>
                        <strong>{row.classification}</strong>
                        <p>{row.reason}</p>
                        {row.witness && (
                          <details>
                            <summary>Witness state and authored priority</summary>
                            <pre>{JSON.stringify(row.witness, null, 2)}</pre>
                            {row.coverage.map((c) => (
                              <p key={c.slot}>
                                Line {c.slot}: {c.matching.length} matching; first{' '}
                                {c.selected ?? 'uncovered'}; explicit fallback{' '}
                                {c.fallback ?? 'none'}.
                              </p>
                            ))}
                          </details>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <div className="variants-actions">
              <button className="btn" disabled={page === 0} onClick={() => setPage((p) => p - 1)}>
                Previous page
              </button>
              <span>
                Page {page + 1} of {pages}
              </span>
              <button
                className="btn"
                disabled={page + 1 >= pages}
                onClick={() => setPage((p) => p + 1)}
              >
                Next page
              </button>
            </div>
            {locked && (
              <p>
                Unlock this slot and every existing variant before changing conditional priority.
              </p>
            )}
            <p>
              Generation count: {selected.size}. This creates conditional placeholders, records an
              undoable structural edit, then opens Build for review and execution. Existing wording
              remains in authored order after the new disjoint conditions.
            </p>
            <button
              className="btn btn-primary"
              disabled={readOnly || !!busy || !!stale || locked || !slot || selected.size === 0}
              onClick={() =>
                void run('Materializing guarded variants and preparing Build…', async (epoch) => {
                  const ids = [...selected]
                  await materialize.mutateAsync({ report: analysis!.id, slot, rows: ids })
                  assertProjectSession(epoch)
                  const build = await narrativeVariantsBuild(analysis!.id, slot, ids)
                  assertProjectSession(epoch)
                  onBuild(build.id)
                })
              }
            >
              Materialize {selected.size} variants and open Build
            </button>
          </>
        )}
      </div>
    </Modal>
  )
}
