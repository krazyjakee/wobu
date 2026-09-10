import { useState } from 'react'
import { Modal } from '../Modal'
import { errorMessage } from '../../lib/api'
import { useModalReport } from './useModalReport'
import {
  narrativeWordingReport,
  narrativeWordingSuppress,
  type DuplicatedWording,
  type WordingReport,
} from '../../lib/api/narrativeWording'
import './export.css'

/** Which editor a copy lives in, for a reader deciding where to go and fix it. */
function where(site: DuplicatedWording['sites'][number]) {
  const kind = 'scene' in site.container ? 'scene' : 'supporting text'
  return `${site.containerName} (${kind})`
}

/**
 * #209. Wordings stored in more than one place, and the reasons given for the
 * ones that are deliberate.
 *
 * Its own pane rather than a row in the scene diagnostics footer, because the
 * question is not answerable about one scene: a line's copy may be in a quest
 * summary the writer is not looking at. It is also not on the path of a
 * keystroke — the check reads every document — so it is asked when opened and
 * when refreshed, and never on render.
 */
export function NarrativeWording({
  readOnly,
  onClose,
}: {
  readOnly: boolean
  onClose: () => void
}) {
  const {
    data: report,
    setData: setReport,
    loading,
    error,
    setError,
    refresh,
  } = useModalReport(narrativeWordingReport)
  const [saving, setSaving] = useState(false)
  const [reasons, setReasons] = useState<Record<string, string>>({})
  const busy = loading || saving

  /** Send the whole list, because the guard is what makes two writers safe. */
  const write = async (next: WordingReport['suppressions']) => {
    if (busy || readOnly || !report) return
    setSaving(true)
    setError('')
    try {
      setReport(await narrativeWordingSuppress(next, report.guard))
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setSaving(false)
    }
  }

  const allow = (revision: string) => {
    const rationale = reasons[revision] ?? ''
    if (!rationale.trim() || !report) return
    void write([...report.suppressions, { revision, rationale }])
    setReasons((previous) => ({ ...previous, [revision]: '' }))
  }

  return (
    <Modal
      onClose={onClose}
      busy={saving}
      titleId="narrative-wording-title"
      descriptionId="narrative-wording-description"
    >
      <section className="nrt-export">
        <h2 id="narrative-wording-title">Repeated wording</h2>
        <p id="narrative-wording-description">
          Two wordings that share a revision are the same words with the same origin, stored twice —
          usually a line pasted into a second place. Repetition is often deliberate, so each one can
          be allowed with a reason; the reason is kept with the project and survives a rebuilt
          index.
        </p>
        <div className="nrt-export-actions">
          <button className="btn" disabled={busy} onClick={() => void refresh(busy)}>
            Check again
          </button>
          <button className="btn" disabled={saving} onClick={onClose}>
            Close
          </button>
        </div>
        {readOnly && (
          <p>This project folder is read-only. Findings are shown; allowing needs write access.</p>
        )}
        {loading && <p role="status">Reading every scene and supporting text asset…</p>}
        {!loading && report && !report.duplicated.length && !error && (
          <p role="status">No wording is stored in more than one place.</p>
        )}
        {!!report?.unreadable.length && (
          <ul aria-label="Unreadable narrative sources">
            {report.unreadable.map((rel) => (
              <li key={rel}>{rel} would not parse, so its wordings are in none of these counts.</li>
            ))}
          </ul>
        )}
        <ul aria-label="Duplicated wording">
          {(report?.duplicated ?? []).map((found) => (
            <li key={found.revision}>
              <blockquote>{found.body}</blockquote>
              <p>{found.sites.length} copies:</p>
              <ul aria-label={`Copies of ${found.revision}`}>
                {found.sites.map((site) => (
                  <li key={site.variant}>{where(site)}</li>
                ))}
              </ul>
              <label>
                Why this repetition is deliberate
                <input
                  value={reasons[found.revision] ?? ''}
                  disabled={busy || readOnly}
                  onChange={(event) =>
                    setReasons((previous) => ({
                      ...previous,
                      [found.revision]: event.target.value,
                    }))
                  }
                />
              </label>
              <button
                className="btn"
                disabled={busy || readOnly || !(reasons[found.revision] ?? '').trim()}
                onClick={() => allow(found.revision)}
              >
                Allow this repetition
              </button>
            </li>
          ))}
        </ul>
        {!!report?.suppressions.length && (
          <ul aria-label="Allowed repetitions">
            {report.suppressions.map((suppression) => (
              <li key={suppression.revision}>
                {suppression.rationale}
                <button
                  className="btn"
                  disabled={busy || readOnly}
                  onClick={() =>
                    void write(
                      report.suppressions.filter((one) => one.revision !== suppression.revision),
                    )
                  }
                  aria-label={`Withdraw the reason for ${suppression.revision}`}
                >
                  Withdraw
                </button>
              </li>
            ))}
          </ul>
        )}
        {error && <p role="alert">{error}</p>}
      </section>
    </Modal>
  )
}
