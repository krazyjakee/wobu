import { useState } from 'react'
import { errorMessage } from '../../lib/api'
import {
  narrativeScenarioSave,
  narrativeScenariosList,
  type Scenario,
  type ScenarioFile,
} from '../../lib/api/narrativeScenarios'
import type { ScenarioTape } from './scenarioTape'

export function PreviewScenarios({
  sceneId,
  tape,
  busy,
  readOnly,
  onLoad,
  onOverlay,
}: {
  sceneId: string
  tape?: ScenarioTape
  busy: boolean
  readOnly: boolean
  onLoad: (scenario: Scenario) => Promise<void>
  /**
   * Draw a saved scenario on the Flow canvas without playing it (#188).
   *
   * Separate from `onLoad` on purpose: loading *starts a run* from the
   * scenario's inputs and will diverge from the tape the moment a different
   * choice is taken, whereas this reads the tape and nothing else. Collapsing
   * the two would mean inspecting a saved route silently replaced whatever was
   * on screen with a new run of it.
   */
  onOverlay?: (name: string, scenario: Scenario) => void
}) {
  const [name, setName] = useState('')
  const [files, setFiles] = useState<ScenarioFile[]>([])
  const [selected, setSelected] = useState('')
  const [working, setWorking] = useState(false)
  const [error, setError] = useState('')
  const [message, setMessage] = useState('')
  const perform = async (operation: () => Promise<void>) => {
    if (busy || working) return
    setWorking(true)
    setError('')
    setMessage('')
    try {
      await operation()
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setWorking(false)
    }
  }
  return (
    <details className="nrt-scenarios">
      <summary>Saved scenarios</summary>
      <p>
        Save the complete recorded run as a named test. Build → Scenario tests replays saved source
        and checks IDs and state, independent of wording.
      </p>
      <fieldset disabled={busy || working}>
        <label>
          Scenario name
          <input value={name} onChange={(event) => setName(event.target.value)} />
        </label>
        <button
          className="btn"
          disabled={readOnly || !tape || tape.incomplete || !name.trim()}
          onClick={() =>
            void perform(async () => {
              if (!tape || tape.incomplete) return
              const saved = await narrativeScenarioSave(name, tape.scenario)
              setMessage(
                `Saved “${saved.name}” with ${saved.scenario.steps.length} asserted steps.`,
              )
            })
          }
        >
          Save scenario
        </button>
        {tape?.incomplete && (
          <p role="alert">
            This run exceeded 1,000 recorded steps. Its tape is incomplete and cannot be saved.
            Restart with a shorter run.
          </p>
        )}
        <button
          className="btn"
          onClick={() =>
            void perform(async () => {
              setFiles(
                (await narrativeScenariosList()).filter((file) => file.scenario.scene === sceneId),
              )
            })
          }
        >
          Browse scenarios
        </button>
        <label>
          Saved scenario
          <select value={selected} onChange={(event) => setSelected(event.target.value)}>
            <option value="">Choose a scenario</option>
            {files.map((file) => (
              <option key={file.id} value={file.id}>
                {file.name}
              </option>
            ))}
          </select>
        </label>
        <button
          className="btn"
          disabled={!selected}
          onClick={() =>
            void perform(async () => {
              const file = files.find((one) => one.id === selected)
              if (file) {
                await onLoad(file.scenario)
                setName(file.name)
                setMessage(
                  'Loaded initial inputs and signatures. Continue the run, or run its assertions from Build.',
                )
              }
            })
          }
        >
          Load scenario
        </button>
        {onOverlay && (
          <button
            className="btn"
            disabled={!selected}
            onClick={() => {
              const file = files.find((one) => one.id === selected)
              if (!file) return
              onOverlay(file.name, file.scenario)
              setMessage(
                'Drawn on the Flow canvas as an overlay. Nothing was replayed and nothing was written.',
              )
            }}
          >
            Show route in Flow
          </button>
        )}
      </fieldset>
      {error && <p role="alert">{error}</p>}
      {message && <p role="status">{message}</p>}
    </details>
  )
}
