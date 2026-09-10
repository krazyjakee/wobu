import { useEffect, useRef, useState } from 'react'
import { Modal } from '../Modal'
import { errorMessage } from '../../lib/api'
import {
  narrativeScenariosList,
  narrativeScenarioRun,
  narrativeScenarioSave,
  type ScenarioFile,
  type ScenarioResult,
} from '../../lib/api/narrativeScenarios'
import type { PreviewTraceSite } from '../../lib/api/narrativePreview'
import './export.css'

export function NarrativeScenarioTests({
  onClose,
  onSource,
  readOnly,
}: {
  onClose: () => void
  onSource: (site: PreviewTraceSite) => void
  readOnly: boolean
}) {
  const [files, setFiles] = useState<ScenarioFile[]>([])
  const [results, setResults] = useState<Record<string, ScenarioResult | string>>({})
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [editing, setEditing] = useState<ScenarioFile | null>(null)
  const [source, setSource] = useState('')
  const cancel = useRef(false)
  const reload = async () => {
    setError('')
    try {
      setFiles(await narrativeScenariosList())
      setResults({})
    } catch (reason) {
      setError(errorMessage(reason))
    }
  }
  useEffect(() => {
    void narrativeScenariosList()
      .then(setFiles)
      .catch((reason: unknown) => setError(errorMessage(reason)))
    return () => {
      cancel.current = true
    }
  }, [])
  const run = async (selected: ScenarioFile[]) => {
    if (busy) return
    setBusy(true)
    cancel.current = false
    setError('')
    for (const file of selected) {
      if (cancel.current) break
      try {
        const result = await narrativeScenarioRun(file.id)
        setResults((previous) => ({ ...previous, [file.id]: result }))
      } catch (reason) {
        setResults((previous) => ({ ...previous, [file.id]: errorMessage(reason) }))
      }
    }
    setBusy(false)
  }
  const save = async () => {
    if (!editing || busy) return
    setBusy(true)
    setError('')
    try {
      const file = await narrativeScenarioSave(editing.name, source, editing)
      setFiles((previous) => previous.map((one) => (one.id === file.id ? file : one)))
      setResults((previous) => {
        const next = { ...previous }
        delete next[file.id]
        return next
      })
      setEditing(null)
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Modal
      onClose={onClose}
      busy={busy}
      titleId="scenario-tests-title"
      descriptionId="scenario-tests-description"
    >
      <section className="nrt-export nrt-scenario-tests">
        <h2 id="scenario-tests-title">Build · Scenario tests</h2>
        <p id="scenario-tests-description">
          Replay saved scenarios against current saved scenes. No generation or canonical state
          changes occur. Results stop at the first divergent step.
        </p>
        <div className="nrt-export-actions">
          <button className="btn" disabled={busy || !files.length} onClick={() => void run(files)}>
            Run all scenarios
          </button>
          <button className="btn" disabled={busy} onClick={() => void reload()}>
            Reload scenarios
          </button>
          {busy && (
            <button
              className="btn"
              onClick={() => {
                cancel.current = true
              }}
            >
              Stop after current scenario
            </button>
          )}
          <button className="btn" disabled={busy} onClick={onClose}>
            Close
          </button>
        </div>
        {!files.length && <p>Save a Preview run as a scenario to add a regression test.</p>}
        <ul aria-label="Scenario test results" className="nrt-scenario-results">
          {files.map((file) => {
            const result = results[file.id]
            const failure = result && typeof result !== 'string' ? result.report?.divergence : null
            return (
              <li key={file.id}>
                <h3>{file.name}</h3>
                <p>
                  {typeof result === 'string'
                    ? result
                    : result?.report?.passed
                      ? `Passed · ${result.report.checked_steps} steps`
                      : failure
                        ? `Failed at step ${failure.step + 1}: ${failure.field}`
                        : result
                          ? 'Compilation blocked'
                          : `${file.scenario.steps.length} asserted steps · Not run`}
                </p>
                <button className="btn" disabled={busy} onClick={() => void run([file])}>
                  Run {file.name}
                </button>
                <button
                  className="btn"
                  disabled={busy || readOnly}
                  onClick={() => {
                    setEditing(file)
                    setSource(JSON.stringify(file.scenario, null, 2))
                  }}
                >
                  Edit assertions for {file.name}
                </button>
                {failure && (
                  <>
                    <pre>
                      {JSON.stringify(
                        { expected: failure.expected, actual: failure.actual },
                        null,
                        2,
                      )}
                    </pre>
                    <button
                      className="btn"
                      onClick={() => {
                        onSource(failure.site)
                        onClose()
                      }}
                    >
                      Open first divergence in Script
                    </button>
                  </>
                )}
                {result &&
                  typeof result !== 'string' &&
                  !result.report &&
                  result.diagnostics.map((diagnostic, index) => (
                    <p key={index}>
                      <button
                        className="btn"
                        onClick={() => {
                          onSource({
                            scene: diagnostic.scene,
                            beat: null,
                            choice: null,
                            outcome: null,
                            slot: null,
                            variant: null,
                          })
                          onClose()
                        }}
                      >
                        {diagnostic.severity}: {diagnostic.message}
                      </button>
                    </p>
                  ))}
              </li>
            )
          })}
        </ul>
        {editing && (
          <fieldset disabled={busy}>
            <legend>Edit {editing.name}</legend>
            <p>
              Assertions may omit boundary fields or state keys to leave them unconstrained. State
              assertions check values; they never assign them.
            </p>
            <label>
              Scenario source (JSON)
              <textarea
                rows={14}
                value={source}
                onChange={(event) => setSource(event.target.value)}
              />
            </label>
            <button className="btn" onClick={() => void save()}>
              Save assertions
            </button>
            <button className="btn" onClick={() => setEditing(null)}>
              Cancel edit
            </button>
          </fieldset>
        )}
        {error && <p role="alert">{error}</p>}
      </section>
    </Modal>
  )
}
