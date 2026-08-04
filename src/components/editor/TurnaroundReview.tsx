import { useCallback, useEffect, useMemo, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import * as api from '../../lib/api'
import type { MeshOptions, QueueSnapshot, WobuNode } from '../../lib/api'
import { useAssetThumb } from '../../lib/queries'
import { report, toast } from '../../store/ui'
import { TipButton } from '../Tooltip'

/**
 * The review-and-reconstruct half of Concept 3D (#110).
 *
 * The viewer beside this could always open a mesh; nothing could make one. The
 * missing step was never the provider — `wobu-imagine` has had a signed,
 * region-pinned Hunyuan3D adapter and a local ComfyUI one for a while — it was
 * that a person had no way to say *these eight pictures, that face count, yes I
 * know it costs money*.
 *
 * Three things this pane insists on, each of them a rule from
 * `docs/guide/concept-3d.html` rather than a preference:
 *
 * 1. **Every required view is on screen before the button is live.** A mesh is
 *    only ever as good as its worst view, and the bad one is cheap to fix now
 *    and expensive to discover afterwards.
 * 2. **A reroll is one image.** The Turnaround preset locks one seed across
 *    eight views so that they are views of the same object; re-rolling the back
 *    view therefore has to be a single tagged generation on a fresh seed, not
 *    another eight.
 * 3. **A paid backend is confirmed, not estimated.** Hunyuan3D bills per job and
 *    the international API does not report the amount back, so there is no
 *    honest number to put in front of the user — only an honest sentence.
 */
export function TurnaroundReview({
  node,
  queue,
  readOnly,
}: {
  node: WobuNode
  queue: QueueSnapshot
  readOnly: boolean
}) {
  const qc = useQueryClient()
  const sheet = useQuery({
    queryKey: ['turnaround_sheet', node.id],
    queryFn: () => api.turnaroundSheet(node.id),
    retry: false,
  })
  const options = useQuery({
    queryKey: ['mesh_options'],
    queryFn: () => api.meshOptions(),
    retry: false,
  })

  const [picked, setPicked] = useState<Record<string, string>>({})
  const [faceCount, setFaceCount] = useState<number | null>(null)
  const [pbr, setPbr] = useState(false)
  const [generateType, setGenerateType] = useState<string | null>(null)
  const [accepted, setAccepted] = useState(false)
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  // A receipt landing is the only thing that changes this pane's inputs, and it
  // arrives whether or not the Concepts tab happens to be mounted — so the 3D
  // tab subscribes for itself rather than relying on that tab's listener.
  useEffect(() => {
    if (!api.isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined
    void listen<api.GenerationRecorded>('generation:recorded', (event) => {
      if (event.payload.subjectId !== node.id) return
      void qc.invalidateQueries({ queryKey: ['turnaround_sheet', node.id] })
      void qc.invalidateQueries({ queryKey: ['mesh_concepts', node.id] })
    })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* the folder watcher remains the slower catch-up path */
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [node.id, qc])

  const meshJob =
    queue.jobs.find(
      (job) => job.kind === 'mesh' && job.subjectId === node.id && !isTerminal(job.state),
    ) ?? null
  const progress = useJobProgress(meshJob?.id ?? null)
  const failedJob = lastMeshFailure(queue, node.id)

  const opts = options.data
  const views = sheet.data?.views
  const slots = useMemo(() => views ?? [], [views])
  // Default every slot to its newest take; an explicit pick or a batch choice
  // overrides it. Derived rather than stored so a reroll that has just landed
  // becomes the default without anything having to reset state.
  const chosen = useMemo(() => {
    const out: Record<string, string> = {}
    for (const slot of slots) {
      const explicit = picked[slot.viewType]
      const valid = explicit && slot.takes.some((take) => take.generationId === explicit)
      out[slot.viewType] = valid ? (explicit as string) : (slot.takes[0]?.generationId ?? '')
    }
    return out
  }, [picked, slots])

  const maxViews = opts?.maxViews ?? 0
  // Which views actually reach the provider. `View::ALL` order is the sheet's
  // order and the request's, so a backend that takes fewer keeps the prefix —
  // front first, because a single-image reconstruction *is* the front view.
  const sending = slots.slice(0, Math.max(1, maxViews)).map((slot) => slot.viewType)
  const missingRequired = sending.filter((viewType) => !chosen[viewType])
  const generationIds = sending.map((viewType) => chosen[viewType]).filter(Boolean) as string[]

  const resolvedFaceCount = faceCount ?? opts?.defaultFaceCount ?? 0
  const resolvedGenerateType = generateType ?? opts?.generateTypes[0] ?? 'Normal'
  const needsConsent = !!opts?.requiresBilling
  const blocked =
    readOnly ||
    !!busy ||
    !!meshJob ||
    !opts?.ready ||
    missingRequired.length > 0 ||
    (needsConsent && !accepted)

  const runGeneration = useCallback(
    async (views: string[] | undefined, label: string) => {
      setError(null)
      setBusy(label)
      try {
        await api.generateStart(node.id, {
          preset: 'turnaround',
          // A reroll has to differ from the take it replaces, and the preset
          // hands every view the caller's one seed — so the new seed is the
          // whole of the change.
          ...(views ? { views, seed: randomSeed() } : {}),
        })
        toast(views ? `Re-rolling the ${views.join(', ')} view` : 'Turnaround queued')
      } catch (reason) {
        setError(api.errorMessage(reason))
        report(reason, 'Could not queue those turnaround views')
      } finally {
        setBusy(null)
      }
    },
    [node.id],
  )

  async function reconstruct() {
    setError(null)
    setBusy('reconstruct')
    try {
      await api.meshStart(node.id, generationIds, {
        faceCount: resolvedFaceCount,
        enablePbr: pbr,
        generateType: resolvedGenerateType,
        acceptCost: accepted,
      })
      toast('Reconstruction queued')
    } catch (reason) {
      setError(api.errorMessage(reason))
      report(reason, 'Could not start reconstruction')
    } finally {
      setBusy(null)
    }
  }

  async function stop() {
    if (!meshJob) return
    try {
      await api.jobCancel(meshJob.id)
    } catch (reason) {
      setError(api.errorMessage(reason))
    }
  }

  if (sheet.isPending) return <aside className="mesh-make">Reading the turnaround…</aside>
  if (sheet.isError) {
    return (
      <aside className="mesh-make">
        Could not read the turnaround: {api.errorMessage(sheet.error)}
      </aside>
    )
  }

  const empty = slots.every((slot) => slot.takes.length === 0)

  return (
    <aside className="mesh-make" aria-label={`Reconstruct a mesh for ${node.name}`}>
      <header>
        <h3>Make a mesh</h3>
        <span>{opts?.provider ? opts.label : 'No 3D backend selected'}</span>
      </header>

      {empty ? (
        <div className="mesh-make-empty">
          <p>
            A mesh is reconstructed from a Turnaround: eight views of {node.name} on one locked
            seed. Generate one, review it here, then reconstruct.
          </p>
          <button
            className="btn btn-primary"
            type="button"
            disabled={readOnly || !!busy}
            onClick={() => void runGeneration(undefined, 'sheet')}
          >
            {busy === 'sheet' ? 'Queuing…' : 'Generate turnaround'}
          </button>
        </div>
      ) : (
        <>
          {(sheet.data?.batches.length ?? 0) > 1 && (
            <nav className="mesh-make-batches" aria-label="Completed turnaround batches">
              {sheet.data?.batches.map((batch) => {
                const active = batch.generationIds.every(
                  (id, index) => chosen[slots[index]?.viewType ?? ''] === id,
                )
                return (
                  <button
                    key={batch.seed}
                    type="button"
                    className={active ? 'btn-mini is-on' : 'btn-mini'}
                    aria-pressed={active}
                    onClick={() =>
                      setPicked(
                        Object.fromEntries(
                          batch.generationIds.map((id, index) => [
                            slots[index]?.viewType ?? '',
                            id,
                          ]),
                        ),
                      )
                    }
                  >
                    seed {batch.seed} · {new Date(batch.createdAt).toLocaleDateString()}
                  </button>
                )
              })}
            </nav>
          )}

          <div className="mesh-make-grid">
            {slots.map((slot, index) => (
              <ViewSlot
                key={slot.viewType}
                slot={slot}
                chosenId={chosen[slot.viewType] ?? ''}
                sent={index < Math.max(1, maxViews)}
                busy={busy === slot.viewType}
                readOnly={readOnly || !!busy}
                onPick={(generationId) =>
                  setPicked((current) => ({ ...current, [slot.viewType]: generationId }))
                }
                onReroll={() => void runGeneration([slot.viewType], slot.viewType)}
              />
            ))}
          </div>

          {(sheet.data?.missing.length ?? 0) > 0 && (
            <p className="mesh-make-missing" role="status">
              Not rendered yet: {sheet.data?.missing.join(', ')}.
              <button
                className="btn-mini"
                type="button"
                disabled={readOnly || !!busy}
                onClick={() => void runGeneration(sheet.data?.missing, 'missing')}
              >
                {busy === 'missing' ? 'Queuing…' : 'Generate the missing views'}
              </button>
            </p>
          )}
        </>
      )}

      {opts && opts.provider && (
        <ReconstructControls
          options={opts}
          faceCount={resolvedFaceCount}
          onFaceCount={setFaceCount}
          pbr={pbr}
          onPbr={setPbr}
          generateType={resolvedGenerateType}
          onGenerateType={setGenerateType}
          sending={sending}
          disabled={readOnly || !!meshJob}
        />
      )}

      {opts && !opts.ready && (
        <p className="mesh-make-blocked" role="status">
          {opts.detail}
        </p>
      )}

      {needsConsent && (
        <label className="mesh-make-consent">
          <input
            type="checkbox"
            checked={accepted}
            disabled={readOnly || !!meshJob}
            onChange={(event) => setAccepted(event.target.checked)}
          />
          <span>
            {opts?.label} charges for every submitted job, including one that is cancelled while it
            runs, and does not report the amount back — so Wobu cannot show a price. Start it
            anyway.
          </span>
        </label>
      )}

      {meshJob ? (
        <div className="mesh-make-active" role="status">
          <span>
            {meshJob.label} · {meshJob.state}
            {progress ? ` · ${progress}` : ''}
          </span>
          <button className="btn-mini" type="button" onClick={() => void stop()}>
            Stop
          </button>
        </div>
      ) : (
        <TipButton
          className="btn btn-primary"
          disabledReason={
            !blocked
              ? null
              : readOnly
                ? 'This project is read-only, and a reconstruction writes a mesh into it.'
                : missingRequired.length
                  ? `A mesh needs every required view first. Still missing: ${missingRequired.join(', ')}.`
                  : 'Wait for the views to finish.'
          }
          tip="Send the approved views to the mesh backend"
          onClick={() => void reconstruct()}
        >
          {busy === 'reconstruct' ? 'Queuing…' : 'Reconstruct mesh'}
        </TipButton>
      )}

      {failedJob && !meshJob && (
        <p className="mesh-make-error" role="status">
          {failedJob.message}
          {failedJob.billed === 'charged' ? ' This job was charged for.' : ''}
        </p>
      )}
      {error && <p className="mesh-make-error">{error}</p>}
    </aside>
  )
}

function ReconstructControls({
  options,
  faceCount,
  onFaceCount,
  pbr,
  onPbr,
  generateType,
  onGenerateType,
  sending,
  disabled,
}: {
  options: MeshOptions
  faceCount: number
  onFaceCount: (value: number) => void
  pbr: boolean
  onPbr: (value: boolean) => void
  generateType: string
  onGenerateType: (value: string) => void
  sending: string[]
  disabled: boolean
}) {
  return (
    <div className="mesh-make-options" role="group" aria-label="Reconstruction options">
      <label>
        <span>Faces</span>
        <input
          type="number"
          aria-label="Face count"
          min={options.faceCountMin}
          max={options.faceCountMax}
          step={1000}
          value={faceCount}
          disabled={disabled}
          onChange={(event) => onFaceCount(Number(event.target.value))}
        />
        <small>
          {options.faceCountMin.toLocaleString()}–{options.faceCountMax.toLocaleString()}
        </small>
      </label>
      <label>
        <span>Mode</span>
        <select
          aria-label="Reconstruction mode"
          value={generateType}
          disabled={disabled || options.generateTypes.length < 2}
          onChange={(event) => onGenerateType(event.target.value)}
        >
          {options.generateTypes.map((value) => (
            <option key={value} value={value}>
              {value}
            </option>
          ))}
        </select>
      </label>
      <label>
        <input
          type="checkbox"
          checked={pbr && options.pbr}
          disabled={disabled || !options.pbr}
          onChange={(event) => onPbr(event.target.checked)}
        />
        <span>PBR materials{options.pbr ? '' : ' — not offered by this backend'}</span>
      </label>
      <p className="mesh-make-sending">
        {options.model} · sending {sending.length} view{sending.length === 1 ? '' : 's'} (
        {sending.join(', ')})
        {options.maxViews < 8
          ? ' — this tier reconstructs from fewer views than a turnaround has.'
          : ''}
      </p>
    </div>
  )
}

function ViewSlot({
  slot,
  chosenId,
  sent,
  busy,
  readOnly,
  onPick,
  onReroll,
}: {
  slot: api.TurnaroundSlot
  chosenId: string
  sent: boolean
  busy: boolean
  readOnly: boolean
  onPick: (generationId: string) => void
  onReroll: () => void
}) {
  const take = slot.takes.find((candidate) => candidate.generationId === chosenId) ?? null
  const thumb = useAssetThumb(take?.assetId ?? null)
  const index = take ? slot.takes.indexOf(take) : -1
  const label = slot.viewType.replace('_', ' ')

  return (
    <figure
      className={`mesh-make-tile${take ? '' : ' is-empty'}${sent ? '' : ' is-unused'}`}
      aria-label={`${label} view`}
    >
      <div className="asset-media-frame">
        {thumb.data ? (
          <img src={convertFileSrc(thumb.data)} alt={`${label} turnaround view`} />
        ) : (
          <span>{take ? 'Loading…' : 'Not rendered'}</span>
        )}
      </div>
      <figcaption>
        <b>{label}</b>
        {take && <span>seed {take.seed}</span>}
        {!sent && <span>not sent to this backend</span>}
      </figcaption>
      <div className="mesh-make-tile-actions">
        {slot.takes.length > 1 && (
          <button
            className="btn-mini"
            type="button"
            aria-label={`Cycle ${label} take`}
            onClick={() =>
              onPick(slot.takes[(index + 1) % slot.takes.length]?.generationId ?? chosenId)
            }
          >
            take {index + 1}/{slot.takes.length}
          </button>
        )}
        <button
          className="btn-mini"
          type="button"
          aria-label={take ? `Re-roll the ${label} view` : `Generate the ${label} view`}
          disabled={readOnly}
          onClick={onReroll}
        >
          {busy ? '…' : take ? 'Reroll' : 'Generate'}
        </button>
      </div>
    </figure>
  )
}

/**
 * The adapter's own words for where a mesh job has got to.
 *
 * A subscription rather than a query for `useEnhanceStream`'s reason: there is
 * nothing to fetch, the backend already sends whole frames, and a mesh job is
 * minutes long — which is exactly how long a pane with no progress at all looks
 * broken for.
 */
function useJobProgress(jobId: string | null): string | null {
  const [note, setNote] = useState<{ jobId: string; text: string } | null>(null)

  useEffect(() => {
    if (!jobId || !api.isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined
    void listen<api.JobProgress>(api.JOB_EVENTS.progress, (event) => {
      if (event.payload.id !== jobId) return
      const percent = event.payload.total
        ? Math.round(
            (Math.min(event.payload.done, event.payload.total) * 100) / event.payload.total,
          )
        : 0
      setNote({ jobId, text: event.payload.note ?? `${percent}%` })
    })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* the queue snapshot still reports the state */
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [jobId])

  return note?.jobId === jobId ? note.text : null
}

function isTerminal(state: QueueSnapshot['jobs'][number]['state']): boolean {
  return state === 'done' || state === 'failed' || state === 'cancelled'
}

/**
 * The most recent failed reconstruction for this entity, or nothing.
 *
 * Newest-last scan rather than `findLast`, which is ES2023 and this project
 * targets ES2022.
 */
function lastMeshFailure(queue: QueueSnapshot, nodeId: string): api.JobFailure | null {
  for (let index = queue.jobs.length - 1; index >= 0; index -= 1) {
    const job = queue.jobs[index]
    if (!job || job.kind !== 'mesh' || job.subjectId !== nodeId) continue
    if (job.state === 'failed') return job.failure
    if (!isTerminal(job.state)) return null
  }
  return null
}

/** The same 53-bit range the Inspector's reroll uses. */
function randomSeed(): number {
  return Math.floor(Math.random() * Number.MAX_SAFE_INTEGER)
}
