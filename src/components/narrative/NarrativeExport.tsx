import { useState } from 'react'
import { save } from '@tauri-apps/plugin-dialog'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import { Modal } from '../Modal'
import { errorMessage } from '../../lib/api'
import {
  narrativeExport,
  narrativeExportCheck,
  type ExportOptions,
  type ExportCheck,
  type NarrativeExportReport,
} from '../../lib/api/narrativeExport'
import { PreviewCommands } from './PreviewCommands'
import './export.css'

export function NarrativeExport({ onClose }: { onClose: () => void }) {
  const [options, setOptions] = useState<ExportOptions>({
    profile: 'development',
    commands: {},
    debug: false,
  })
  const [validCommands, setValidCommands] = useState(true)
  const [destination, setDestination] = useState('')
  const [check, setCheck] = useState<ExportCheck | null>(null)
  const [report, setReport] = useState<NarrativeExportReport | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const update = (next: ExportOptions) => {
    setOptions(next)
    setCheck(null)
    setReport(null)
    setError('')
  }
  const validate = async () => {
    setBusy(true)
    setError('')
    setCheck(null)
    setReport(null)
    try {
      setCheck(await narrativeExportCheck(options))
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setBusy(false)
    }
  }
  const choose = async () => {
    try {
      const path = await save({
        title: 'New narrative package folder',
        defaultPath: 'narrative-export',
      })
      if (path) setDestination(path)
    } catch (reason) {
      setError(errorMessage(reason))
    }
  }
  const publish = async () => {
    if (!check?.payloadHash) return
    setBusy(true)
    setError('')
    setReport(null)
    try {
      setReport(await narrativeExport(options, destination, check.payloadHash))
    } catch (reason) {
      setError(errorMessage(reason))
      setCheck(null)
    } finally {
      setBusy(false)
    }
  }
  return (
    <Modal
      onClose={onClose}
      busy={busy}
      titleId="narrative-export-title"
      descriptionId="narrative-export-description"
    >
      <section className="nrt-export">
        <h2 id="narrative-export-title">Export narrative</h2>
        <p id="narrative-export-description">
          Native JSON package · Saved scenes and wording. Save or discard drafts before checking. No
          generation runs.
        </p>
        <fieldset disabled={busy}>
          <label>
            Profile
            <select
              value={options.profile}
              onChange={(event) =>
                update({
                  ...options,
                  profile: event.target.value as ExportOptions['profile'],
                  debug: false,
                })
              }
            >
              <option value="development">Development</option>
              <option value="release">Release</option>
            </select>
          </label>
          <label className="nrt-export-option">
            <input
              type="checkbox"
              checked={options.debug}
              disabled={options.profile === 'release'}
              onChange={(event) => update({ ...options, debug: event.target.checked })}
            />
            Include debug source maps (development only)
          </label>
          <PreviewCommands
            disabled={busy}
            onChange={(commands) => {
              setValidCommands(commands !== null)
              update({ ...options, commands: commands ?? {} })
            }}
          />
          <label>
            Destination folder
            <input
              value={destination}
              onChange={(event) => setDestination(event.target.value)}
              placeholder="Choose a new folder outside the project"
            />
          </label>
          <button type="button" className="btn" onClick={() => void choose()}>
            Choose destination…
          </button>
          <p>
            An existing folder is never overwritten. Interrupted exports keep an incomplete marker
            and cannot be loaded.
          </p>
        </fieldset>
        <p>
          Release requires approved, current, non-empty text. Freshness uses recorded source status.
          Media bindings are not authored yet; this package contains no media assets.
        </p>
        <div className="nrt-export-actions">
          <button className="btn" disabled={busy || !validCommands} onClick={() => void validate()}>
            Check export
          </button>
          <button
            className="btn btn-primary"
            disabled={busy || !validCommands || !check?.payloadHash || !destination.trim()}
            onClick={() => void publish()}
          >
            {busy ? 'Working…' : 'Export package'}
          </button>
          <button className="btn" disabled={busy} onClick={onClose}>
            Close
          </button>
        </div>
        {check && (
          <div role="status">
            {check.payloadHash
              ? `Ready: ${check.scenes} scenes, ${check.strings} strings, ${check.bytes} bytes.`
              : 'Export blocked. Fix the errors below and check again.'}
          </div>
        )}
        {!!check?.diagnostics.length && (
          <ul aria-label="Export diagnostics">
            {check.diagnostics.map((diagnostic, index) => (
              <li key={index}>
                {diagnostic.severity}: {diagnostic.message} (scene {diagnostic.scene})
              </li>
            ))}
          </ul>
        )}
        {error && <p role="alert">{error}</p>}
        {report && (
          <div role="status">
            <p>
              Exported {report.scenes} scenes and {report.strings} strings to {report.destination}.
            </p>
            <p className="nrt-export-hash">Payload: {report.payloadHash}</p>
            <button
              className="btn"
              onClick={() =>
                void revealItemInDir(report.destination).catch((reason: unknown) =>
                  setError(errorMessage(reason)),
                )
              }
            >
              Show exported folder
            </button>
          </div>
        )}
      </section>
    </Modal>
  )
}
