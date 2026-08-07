import { useCallback, useEffect, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import type { AboutInfo, IndexInfo } from '../../lib/api'
import { aboutInfo, indexInfo, indexRebuild } from '../../lib/api'
import { THEME_LABELS, THEME_MODES } from '../../lib/ThemeMode'
import { invalidateWorld } from '../../lib/queries'
import {
  AUTOSAVE_DEFAULT,
  AUTOSAVE_MAX,
  AUTOSAVE_MIN,
  SCALE_MAX,
  SCALE_MIN,
  SCALE_STEP,
  useSettings,
} from '../../store/settings'
import { report, toast } from '../../store/ui'
import { ConfirmSheet } from '../ConfirmSheet'
import { Icon } from '../Icon'
import { formatSize } from './formatSize'

/**
 * The local SQLite index.
 *
 * Rebuild is offered prominently on purpose. The index is disposable by design
 * — every fact lives in the Markdown — and that property is worth surfacing
 * rather than hiding, because it makes "the navigator is showing something that
 * isn't there" fixable by the user instead of a support conversation.
 */
export function Storage() {
  const qc = useQueryClient()
  const [info, setInfo] = useState<IndexInfo | null>(null)
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState(false)

  const refresh = useCallback(async () => {
    try {
      setInfo(await indexInfo())
    } catch (e) {
      report(e, 'Could not read the search index')
    }
  }, [])

  useEffect(() => {
    let disposed = false
    void indexInfo().then(
      (value) => {
        if (!disposed) setInfo(value)
      },
      (error) => report(error, 'Could not read the search index'),
    )
    return () => {
      disposed = true
    }
  }, [])

  async function rebuild() {
    setConfirming(false)
    setBusy(true)
    try {
      await indexRebuild()
      invalidateWorld(qc)
      toast('The search index was rebuilt from the project folder.')
    } catch (e) {
      report(e, 'Could not rebuild the search index')
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
        The search index is what makes the world searchable. It holds no original copy of anything —
        every fact is in the project folder as Markdown — so deleting or rebuilding it is always
        safe.
      </p>

      <div className="set-row">
        <span className="set-label">Search index</span>
        <code className="set-path">{info.path}</code>
      </div>
      <div className="set-row">
        <span className="set-label">Size</span>
        <span className="set-value">
          {formatSize(info.sizeBytes)} · {info.nodeCount}{' '}
          {info.nodeCount === 1 ? 'entity' : 'entities'}
        </span>
      </div>

      <div className="set-acts">
        <button className="btn-mini" onClick={() => setConfirming(true)} disabled={busy}>
          <Icon name="refresh" size="sm" />
          {busy ? 'Rebuilding…' : 'Rebuild search index'}
        </button>
      </div>

      {confirming && (
        <ConfirmSheet
          title="Rebuild the search index?"
          body="Every entity file is read again from the project folder. Nothing you have written is touched — the index is only made from it. On a large world over a network drive this can take a while."
          confirmLabel="Rebuild"
          onConfirm={() => void rebuild()}
          onCancel={() => setConfirming(false)}
        />
      )}
    </section>
  )
}

export function EditorPrefs() {
  const delay = useSettings((s) => s.autosaveDelay)
  const setDelay = useSettings((s) => s.setAutosaveDelay)

  return (
    <section className="set-sec">
      <h3>Editor</h3>
      <div className="set-row set-row-col">
        <label className="set-label" htmlFor="settings-autosave-delay">
          Autosave after
        </label>
        <div className="set-slider">
          <input
            id="settings-autosave-delay"
            type="range"
            min={AUTOSAVE_MIN}
            max={AUTOSAVE_MAX}
            step={50}
            value={delay}
            aria-valuetext={`${(delay / 1000).toFixed(2)} seconds`}
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
 * Theme and interface scale.
 *
 * The theme is a three-way choice rather than a toggle because "match system"
 * is a different thing from either fixed value: it keeps following the desktop
 * afterwards, including a desktop that switches itself at sunset. Applying it
 * is not this component's job — `store/settings` writes the resolved theme onto
 * `<html>` the moment it changes, so the buttons only record what was asked
 * for (#134).
 */
export function Appearance() {
  const scale = useSettings((s) => s.uiScale)
  const setScale = useSettings((s) => s.setUiScale)
  const theme = useSettings((s) => s.theme)
  const setTheme = useSettings((s) => s.setTheme)
  const pct = Math.round(scale * 100)

  return (
    <section className="set-sec">
      <h3>Appearance</h3>
      <div className="set-row set-row-col">
        <span className="set-label" id="settings-theme-label">
          Theme
        </span>
        <div className="set-levels" role="group" aria-labelledby="settings-theme-label">
          {THEME_MODES.map((mode) => (
            <button
              key={mode}
              className={mode === theme ? 'btn-mini is-on' : 'btn-mini'}
              aria-pressed={mode === theme}
              onClick={() => setTheme(mode)}
            >
              {THEME_LABELS[mode]}
            </button>
          ))}
        </div>
      </div>
      <p className="set-note">
        Both themes carry the same palette: the influence-layer colours stay distinguishable, and
        text keeps its contrast either way. Match system follows the desktop as it changes.
      </p>
      <div className="set-row set-row-col">
        <label className="set-label" htmlFor="settings-interface-scale">
          Interface scale
        </label>
        <div className="set-slider">
          <input
            id="settings-interface-scale"
            type="range"
            min={SCALE_MIN}
            max={SCALE_MAX}
            step={SCALE_STEP}
            value={scale}
            aria-valuetext={`${pct} percent`}
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

export function About() {
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
