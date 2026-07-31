import { useCallback, useEffect, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import type { AboutInfo, IndexInfo, LogInfo, LogLevel } from '../lib/api'
import {
  LOG_LEVELS,
  aboutInfo,
  indexInfo,
  indexRebuild,
  logInfo,
  logReveal,
  logSetLevel,
  logTail,
} from '../lib/api'
import { invalidateWorld } from '../lib/queries'
import {
  AUTOSAVE_DEFAULT,
  AUTOSAVE_MAX,
  AUTOSAVE_MIN,
  SCALE_MAX,
  SCALE_MIN,
  SCALE_STEP,
  useSettings,
} from '../store/settings'
import { report, toast } from '../store/ui'
import { ConfirmSheet } from './ConfirmSheet'
import { Icon } from './Icon'

/**
 * The Settings surface.
 *
 * Sections are independent and each owns its own loading, so the M5 providers
 * pane drops in as one more `<section className="set-sec">` rather than a
 * rewrite. Nothing here is stubbed: a control that looks configurable but is
 * not is worse than an honest absence, which is why there is no theme switch —
 * see Appearance.
 */
export function Settings() {
  return (
    <div className="settings-mode">
      <div className="settings">
        <h2>Settings</h2>
        <Storage />
        <EditorPrefs />
        <Appearance />
        <Diagnostics />
        <About />
        <section className="set-sec">
          <h3>Providers and models</h3>
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

/**
 * The local SQLite index.
 *
 * Rebuild is offered prominently on purpose. The index is disposable by design
 * — every fact lives in the Markdown — and that property is worth surfacing
 * rather than hiding, because it makes "the navigator is showing something that
 * isn't there" fixable by the user instead of a support conversation.
 */
function Storage() {
  const qc = useQueryClient()
  const [info, setInfo] = useState<IndexInfo | null>(null)
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState(false)

  const refresh = useCallback(async () => {
    try {
      setInfo(await indexInfo())
    } catch (e) {
      report(e, 'Could not read the index')
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  async function rebuild() {
    setConfirming(false)
    setBusy(true)
    try {
      await indexRebuild()
      invalidateWorld(qc)
      toast('Index rebuilt from the project folder.')
    } catch (e) {
      report(e, 'Could not rebuild the index')
    } finally {
      setBusy(false)
      void refresh()
    }
  }

  if (!info) return null

  return (
    <section className="set-sec">
      <h3>Storage</h3>
      <p className="set-note">
        The index makes the world searchable. It holds no original copy of anything — every fact is
        in the project folder as Markdown — so deleting or rebuilding it is always safe.
      </p>

      <div className="set-row">
        <span className="set-label">Index</span>
        <code className="set-path">{info.path}</code>
      </div>
      <div className="set-row">
        <span className="set-label">Size</span>
        <span className="set-value">
          {formatSize(info.sizeBytes)} · {info.nodeCount}{' '}
          {info.nodeCount === 1 ? 'node' : 'nodes'}
        </span>
      </div>

      <div className="set-acts">
        <button className="btn-mini" onClick={() => setConfirming(true)} disabled={busy}>
          <Icon name="refresh" size="sm" />
          {busy ? 'Rebuilding…' : 'Rebuild index'}
        </button>
      </div>

      {confirming && (
        <ConfirmSheet
          title="Rebuild the index?"
          body="Every node file is read again from the project folder. Nothing you have written is touched — the index is derived from it. On a large world over a network share this can take a while."
          confirmLabel="Rebuild"
          onConfirm={() => void rebuild()}
          onCancel={() => setConfirming(false)}
        />
      )}
    </section>
  )
}

function EditorPrefs() {
  const delay = useSettings((s) => s.autosaveDelay)
  const setDelay = useSettings((s) => s.setAutosaveDelay)

  return (
    <section className="set-sec">
      <h3>Editor</h3>
      <div className="set-row set-row-col">
        <span className="set-label">Autosave after</span>
        <div className="set-slider">
          <input
            type="range"
            min={AUTOSAVE_MIN}
            max={AUTOSAVE_MAX}
            step={50}
            value={delay}
            onChange={(e) => setDelay(Number(e.target.value))}
          />
          <span className="set-value">{(delay / 1000).toFixed(2)}s</span>
          {delay !== AUTOSAVE_DEFAULT && (
            <button className="btn-mini" onClick={() => setDelay(AUTOSAVE_DEFAULT)}>
              Reset
            </button>
          )}
        </div>
      </div>
      <p className="set-note">
        How long typing has to stop before the file is written. Shorter means less to lose if
        something goes wrong; longer means fewer writes, which matters on a network share where
        every save is a round trip. It applies to the pane you already have open.
      </p>
    </section>
  )
}

/**
 * There is no theme control, and that is deliberate rather than unfinished.
 *
 * The palette in `styles/tokens.css` is the design — ported verbatim from the
 * prototype and specified in docs/03-ui-layout.md, down to the influence layer
 * colours that carry meaning in the Inspector. A light theme is a second
 * palette that has to be designed, not a switch that has not been wired, and
 * inventing one here would be picking the product's visual identity from a
 * settings pane.
 */
function Appearance() {
  const scale = useSettings((s) => s.uiScale)
  const setScale = useSettings((s) => s.setUiScale)
  const pct = Math.round(scale * 100)

  return (
    <section className="set-sec">
      <h3>Appearance</h3>
      <div className="set-row set-row-col">
        <span className="set-label">Interface scale</span>
        <div className="set-slider">
          <input
            type="range"
            min={SCALE_MIN}
            max={SCALE_MAX}
            step={SCALE_STEP}
            value={scale}
            onChange={(e) => setScale(Number(e.target.value))}
          />
          <span className="set-value">{pct}%</span>
          {scale !== 1 && (
            <button className="btn-mini" onClick={() => setScale(1)}>
              Reset
            </button>
          )}
        </div>
      </div>
      <p className="set-note">
        Scales the whole interface, not just the text — the navigator and inspector are fixed
        widths, so type alone would grow inside boxes that did not.
      </p>
    </section>
  )
}

function About() {
  const [info, setInfo] = useState<AboutInfo | null>(null)

  useEffect(() => {
    void aboutInfo().then(setInfo, (e: unknown) => report(e, 'Could not read the version'))
  }, [])

  if (!info) return null

  return (
    <section className="set-sec">
      <h3>About</h3>
      <div className="set-row">
        <span className="set-label">Version</span>
        <span className="set-value">wobu {info.appVersion}</span>
      </div>
      <div className="set-row">
        <span className="set-label">Project</span>
        <span className="set-value">schema v{info.projectSchemaVersion}</span>
      </div>
      <div className="set-row">
        <span className="set-label">Index</span>
        <span className="set-value">schema v{info.indexSchemaVersion}</span>
      </div>
      <p className="set-note">
        The project schema is the on-disk format of the folder — a world written by a newer Wobu
        than this one cannot be opened here, rather than opened and quietly stripped of what this
        build does not understand. The index schema is local only; a change to it rebuilds on next
        open, which is what a long pause after an update is.
      </p>
    </section>
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
