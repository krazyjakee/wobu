import { useCallback, useEffect, useState } from 'react'
import type { LogInfo, LogLevel } from '../lib/api'
import { LOG_LEVELS, logInfo, logReveal, logSetLevel, logTail } from '../lib/api'
import { report, toast } from '../store/ui'
import { Icon } from './Icon'

/**
 * The Settings surface.
 *
 * Only Diagnostics is real. Provider keys, model defaults and appearance are
 * the M5 slice (#13) and are deliberately not stubbed here — an empty control
 * that looks configurable is worse than an honest absence.
 */
export function Settings() {
  return (
    <div className="settings-mode">
      <div className="settings">
        <h2>Settings</h2>
        <Diagnostics />
        <section className="set-sec">
          <h3>Providers, models and appearance</h3>
          <span className="milestone">M5 — Enhance (first BYOK providers)</span>
          <p className="set-note">
            Keys live in the OS keychain and never in the project folder, so this section lands with
            the first provider that needs one.
          </p>
        </section>
      </div>
    </div>
  )
}

const LEVEL_HELP: Record<LogLevel, string> = {
  off: 'Records nothing at all, errors included. The file stops growing and stops being useful.',
  error: 'Only failures.',
  warn: 'Failures, and the things that recovered.',
  info: 'The default. Failures plus which project was opened and when.',
  debug: 'Adds the technical detail behind each failure. Use this to reproduce something once.',
}

/**
 * The log, and the two things a user actually needs to do with it: change how
 * much it records, and get at the file.
 *
 * Reading it back is offered on purpose. The file is redacted on write, but
 * "trust us" is not a reasonable thing to ask of someone about to paste it into
 * a public issue tracker — so they can look first.
 */
function Diagnostics() {
  const [info, setInfo] = useState<LogInfo | null>(null)
  const [preview, setPreview] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const refresh = useCallback(async () => {
    try {
      setInfo(await logInfo())
    } catch (e) {
      report(e, 'Could not read the log settings')
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  // While the contents are on screen they are kept current. A snapshot would be
  // the wrong default for a pane whose whole job is "this is what you are about
  // to send" — the app keeps logging, and a preview that quietly falls behind
  // the file understates what is in it. Only runs while expanded.
  const open = preview !== null
  useEffect(() => {
    if (!open) return
    const id = window.setInterval(() => {
      void logTail(200).then(setPreview, () => {
        /* a transient read failure is not worth a toast every two seconds */
      })
      void refresh()
    }, 2000)
    return () => window.clearInterval(id)
  }, [open, refresh])

  async function changeLevel(level: LogLevel) {
    // Optimistic: the control is a radio group and a lagging selection reads as
    // a dropped click. A failure re-reads the truth below.
    setInfo((prev) => (prev ? { ...prev, level } : prev))
    try {
      await logSetLevel(level)
    } catch (e) {
      report(e, 'Could not change the log level')
    } finally {
      void refresh()
    }
  }

  async function reveal() {
    try {
      await logReveal()
    } catch (e) {
      report(e, 'Could not show the log')
    }
  }

  async function togglePreview() {
    if (preview !== null) {
      setPreview(null)
      return
    }
    setBusy(true)
    try {
      setPreview(await logTail(200))
    } catch (e) {
      report(e, 'Could not read the log')
    } finally {
      setBusy(false)
    }
  }

  if (!info) return null

  return (
    <section className="set-sec">
      <h3>Diagnostics</h3>
      <p className="set-note">
        Wobu sends nothing anywhere. When something goes wrong, this file is the only account of
        it — it stays on this machine until you hand it over yourself.
      </p>

      <div className="set-row">
        <span className="set-label">Log file</span>
        <code className="set-path">{info.path}</code>
      </div>
      <div className="set-row">
        <span className="set-label">Size</span>
        <span className="set-value">
          {info.exists ? formatSize(info.sizeBytes) : 'nothing recorded yet'}
        </span>
      </div>

      <div className="set-row set-row-col">
        <span className="set-label">Level</span>
        <div className="set-levels">
          {LOG_LEVELS.map((l) => (
            <button
              key={l}
              className={l === info.level ? 'btn-mini is-on' : 'btn-mini'}
              onClick={() => void changeLevel(l)}
            >
              {l}
            </button>
          ))}
        </div>
        <p className="set-note">{LEVEL_HELP[info.level]}</p>
      </div>

      <div className="set-acts">
        <button className="btn-mini" onClick={() => void reveal()}>
          <Icon name="folder" size="sm" />
          Reveal log file
        </button>
        <button className="btn-mini" onClick={() => void togglePreview()} disabled={busy}>
          <Icon name="search" size="sm" />
          {preview !== null ? 'Hide contents' : busy ? 'Reading…' : 'Show contents'}
        </button>
        {preview !== null && (
          <button
            className="btn-mini"
            onClick={() => {
              void navigator.clipboard.writeText(preview).then(
                () => toast('Log copied.'),
                (e: unknown) => report(e, 'Could not copy the log'),
              )
            }}
          >
            <Icon name="copy" size="sm" />
            Copy
          </button>
        )}
      </div>

      {preview !== null && (
        <pre className="set-log">
          {preview.trim() ? preview : 'Nothing has been recorded yet.'}
        </pre>
      )}
    </section>
  )
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
