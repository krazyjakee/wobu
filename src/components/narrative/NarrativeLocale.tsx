import { useMemo, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { save } from '@tauri-apps/plugin-dialog'
import { Modal } from '../Modal'
import { errorMessage } from '../../lib/api'
import { invalidateNarrative } from '../../lib/queries/keys'
import {
  narrativeLocaleGet,
  narrativeLocalePolicy,
  narrativeLocaleExport,
  narrativeLocalePreview,
  narrativeLocaleImport,
  narrativeLocaleApprove,
  translationStatus,
  translationIndex,
  canonicalLocale,
  localeDirection,
  type LocaleDiagnostic,
} from '../../lib/api/narrativeLocale'
import './locale.css'
import { PageControls } from './PageControls'
import { InterchangeFormat } from './InterchangeFormat'
import {
  assertProjectSession,
  isProjectSession,
  projectSessionEpoch,
} from '../../lib/projectSession'
export function NarrativeLocale({
  projectKey,
  readOnly,
  onClose,
}: {
  projectKey: string
  readOnly: boolean
  onClose: () => void
}) {
  const [epoch] = useState(projectSessionEpoch)
  const client = useQueryClient()
  const query = useQuery({
    queryKey: ['narrative_locale', projectKey],
    queryFn: narrativeLocaleGet,
    retry: false,
  })
  const [locale, setLocale] = useState('fr')
  const [csv, setCsv] = useState(true)
  const [input, setInput] = useState('')
  const [preview, setPreview] = useState<LocaleDiagnostic[] | null>(null)
  const [message, setMessage] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [status, setStatus] = useState('')
  const [search, setSearch] = useState('')
  const [page, setPage] = useState(0)
  const [diagnosticPage, setDiagnosticPage] = useState(0)
  const run = async (action: () => Promise<void>) => {
    setBusy(true)
    setError('')
    try {
      assertProjectSession(epoch)
      await action()
    } catch (e) {
      if (isProjectSession(epoch)) setError(errorMessage(e))
    } finally {
      if (isProjectSession(epoch)) setBusy(false)
    }
  }
  const changed = (value: string) => {
    assertProjectSession(epoch)
    setInput(value)
    setPreview(null)
    setMessage('')
  }
  const view = query.data
  const index = useMemo(() => translationIndex(view), [view])
  const matching = useMemo(
    () =>
      Object.values(view?.sources ?? {}).filter(
        (source) =>
          (!status || translationStatus(view!, locale, source.id, index) === status) &&
          `${source.text} ${source.speaker} ${source.context} ${source.id}`
            .toLocaleLowerCase()
            .includes(search.toLocaleLowerCase()),
      ),
    [view, locale, status, search, index],
  )
  const lastPage = Math.max(0, Math.ceil(matching.length / 25) - 1)
  const currentPage = Math.min(page, lastPage)
  const lastDiagnosticPage = Math.max(0, Math.ceil((preview?.length ?? 0) / 50) - 1)
  const currentDiagnosticPage = Math.min(diagnosticPage, lastDiagnosticPage)
  return (
    <Modal
      onClose={onClose}
      busy={busy}
      titleId="locale-title"
      descriptionId="locale-description"
      className="sheet nrt-locale-sheet"
    >
      <section className="nrt-locale">
        <h2 id="locale-title">Localisation</h2>
        <p id="locale-description">
          Export approved, current, locked source wording. Imported translations begin Draft;
          approval is a separate decision. Source unlocks and edits retain earlier versions and
          withdraw freshness.
        </p>
        {query.isPending ? (
          <p role="status">Reading saved localisation…</p>
        ) : query.isError ? (
          <p role="alert">{errorMessage(query.error)}</p>
        ) : (
          view && (
            <>
              <fieldset disabled={busy}>
                <label>
                  Locale{' '}
                  <input
                    value={locale}
                    onChange={(e) => {
                      setLocale(canonicalLocale(e.target.value))
                      setPreview(null)
                    }}
                    placeholder="fr, ar, pt-BR"
                  />
                </label>
                <p>
                  Language[-Script][-REGION]. Source locale: {view.policy.source}. Fallback, when
                  explicitly enabled, truncates the target tag and finally uses source. No
                  sibling-region substitution.
                </p>
                <InterchangeFormat
                  label="Interchange format"
                  csv={csv}
                  onChange={(value) => {
                    setCsv(value)
                    setPreview(null)
                  }}
                />
                <button
                  className="btn"
                  onClick={() =>
                    void run(async () => {
                      const path = await save({
                        title: 'New localisation interchange file',
                        defaultPath: `translations-${locale}.${csv ? 'csv' : 'json'}`,
                      })
                      assertProjectSession(epoch)
                      if (path) {
                        changed(await narrativeLocaleExport(locale, csv, path))
                        setMessage(
                          'Exported approved locked source. Existing files are never overwritten.',
                        )
                      }
                    })
                  }
                >
                  Export locale…
                </button>
                <label>
                  Translation file{' '}
                  <input
                    type="file"
                    accept=".csv,.json"
                    onChange={(e) => {
                      const file = e.target.files?.[0]
                      if (file) void run(async () => changed(await file.text()))
                    }}
                  />
                </label>
                <label>
                  Interchange contents{' '}
                  <textarea
                    dir="auto"
                    rows={7}
                    value={input}
                    onChange={(e) => changed(e.target.value)}
                  />
                </label>
                <p>
                  UTF-8, quoted multiline CSV, and JSON are supported. Forms use
                  zero/one/two/few/many/other; other is required. The host selects a category,
                  falling back to other. Preserve named placeholders and format suffixes; values are
                  host-formatted plain text.
                </p>
                <button
                  className="btn"
                  disabled={!input.trim()}
                  onClick={() =>
                    void run(async () => setPreview(await narrativeLocalePreview(input, csv)))
                  }
                >
                  Preview import
                </button>
                <button
                  className="btn"
                  disabled={readOnly || preview === null}
                  onClick={() =>
                    void run(async () => {
                      const report = await narrativeLocaleImport(input, csv)
                      setPreview(report.diagnostics)
                      setMessage(
                        `${report.applied.length} rows imported; ${Object.keys(report.conflicts).length} write conflicts. ${Object.values(report.conflicts).join(' ')}`,
                      )
                      invalidateNarrative(client)
                      await query.refetch()
                    })
                  }
                >
                  Import eligible rows
                </button>
                {preview && (
                  <section aria-label="Import preview">
                    <p>
                      {preview.length === 0
                        ? 'No row problems found. Apply rechecks source and translation guards.'
                        : `${preview.length} row diagnostics. Invalid rows are skipped; missing rows are unchanged.`}
                    </p>
                    <ul>
                      {preview
                        .slice(currentDiagnosticPage * 50, (currentDiagnosticPage + 1) * 50)
                        .map((d, i) => (
                          <li key={i}>
                            {d.id}: {d.code} — {d.message}
                          </li>
                        ))}
                    </ul>
                    {lastDiagnosticPage > 0 && (
                      <PageControls
                        currentPage={currentDiagnosticPage}
                        lastPage={lastDiagnosticPage}
                        onPage={setDiagnosticPage}
                      />
                    )}
                  </section>
                )}
                <fieldset disabled={readOnly}>
                  <legend>Release locale policy</legend>
                  <label>
                    <input
                      type="checkbox"
                      checked={locale in view.policy.required}
                      onChange={(e) =>
                        void run(async () => {
                          const required = { ...view.policy.required }
                          if (e.target.checked) required[locale] = false
                          else delete required[locale]
                          await narrativeLocalePolicy(
                            { ...view.policy, required },
                            view.policy_guard,
                          )
                          invalidateNarrative(client)
                          await query.refetch()
                        })
                      }
                    />
                    Require this locale for Release
                  </label>
                  <label>
                    <input
                      type="checkbox"
                      disabled={!(locale in view.policy.required)}
                      checked={view.policy.required[locale] ?? false}
                      onChange={(e) =>
                        void run(async () => {
                          await narrativeLocalePolicy(
                            {
                              ...view.policy,
                              required: { ...view.policy.required, [locale]: e.target.checked },
                            },
                            view.policy_guard,
                          )
                          invalidateNarrative(client)
                          await query.refetch()
                        })
                      }
                    />
                    Explicitly allow truncation and source fallback
                  </label>
                </fieldset>
              </fieldset>
              <label>
                Translation status{' '}
                <select value={status} onChange={(e) => setStatus(e.target.value)}>
                  <option value="">All</option>
                  <option value="missing">Missing</option>
                  <option value="draft">Draft</option>
                  <option value="approved">Approved and current</option>
                  <option value="out_of_date">Out of date</option>
                </select>
              </label>
              <label>
                Search localisation{' '}
                <input
                  value={search}
                  onChange={(e) => {
                    setSearch(e.target.value)
                    setPage(0)
                  }}
                />
              </label>
              <p role="status">
                {matching.length} matching of {Object.keys(view.sources).length} source strings
              </p>
              <PageControls currentPage={currentPage} lastPage={lastPage} onPage={setPage} />
              <table>
                <thead>
                  <tr>
                    <th>Source</th>
                    <th>Translation</th>
                    <th>Status and history</th>
                  </tr>
                </thead>
                <tbody>
                  {matching.slice(currentPage * 25, (currentPage + 1) * 25).map((source) => {
                    const translation = index.get(`${locale}/${source.id}`)
                    const latest = translation?.history.at(-1)
                    const current = translationStatus(view, locale, source.id, index)
                    return (
                      <tr key={source.id}>
                        <td>
                          <p>
                            {source.speaker} · {source.context}
                          </p>
                          <p dir="auto">{source.text}</p>
                          <code>{source.id}</code>
                        </td>
                        <td>
                          {latest
                            ? Object.entries(latest.forms).map(([category, text]) => (
                                <div key={category}>
                                  <strong>{category}: </strong>
                                  <p dir={localeDirection(locale)}>{text}</p>
                                </div>
                              ))
                            : 'No translation'}
                        </td>
                        <td>
                          <p>
                            {current.replaceAll('_', ' ')}
                            {!source.ready ? ' · source not ready' : ''}
                          </p>
                          <button
                            className="btn"
                            disabled={busy || readOnly || current !== 'draft'}
                            onClick={() =>
                              void run(async () => {
                                await narrativeLocaleApprove({
                                  version: 1,
                                  locale,
                                  source,
                                  translation_guard:
                                    view.translation_guards[`${locale}/${source.id}`] ?? null,
                                  forms: latest!.forms,
                                })
                                invalidateNarrative(client)
                                await query.refetch()
                              })
                            }
                          >
                            Approve translation
                          </button>
                          <details>
                            <summary>{translation?.history.length ?? 0} versions</summary>
                            {translation?.history.map((version, i) => (
                              <p dir={localeDirection(locale)} key={i}>
                                {version.approved ? 'Approved' : 'Draft'} ·{' '}
                                {version.source_revision}: {JSON.stringify(version.forms)}
                              </p>
                            ))}
                          </details>
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </>
          )
        )}
        {message && <p role="status">{message}</p>}
        {error && <p role="alert">{error}</p>}
        <button className="btn" disabled={busy} onClick={onClose}>
          Close localisation
        </button>
      </section>
    </Modal>
  )
}
