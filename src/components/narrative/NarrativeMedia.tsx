import { useMemo, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { open, save } from '@tauri-apps/plugin-dialog'
import { Modal } from '../Modal'
import { errorMessage } from '../../lib/api'
import { invalidateNarrative } from '../../lib/queries/keys'
import { canonicalLocale } from '../../lib/api/narrativeLocale'
import {
  assertProjectSession,
  isProjectSession,
  projectSessionEpoch,
} from '../../lib/projectSession'
import {
  narrativeMediaGet,
  narrativeMediaPolicy,
  narrativeMediaExport,
  narrativeMediaPreview,
  narrativeMediaImport,
  narrativeMediaAudition,
  mediaKey,
  mediaStatus,
  type MediaDiagnostic,
  type MediaAudition as Audition,
} from '../../lib/api/narrativeMedia'
import { PageControls } from './PageControls'
import { InterchangeFormat } from './InterchangeFormat'
import { MediaAudition } from './media/MediaAudition'
import { MediaLine } from './media/MediaLine'
import { useMediaNotes } from './media/notesDrafts'
import './locale.css'
import './media.css'
export function NarrativeMedia({
  projectKey,
  readOnly,
  onClose,
}: {
  projectKey: string
  readOnly: boolean
  onClose: () => void
}) {
  const [epoch] = useState(projectSessionEpoch)
  const [locale, setLocale] = useState('en')
  const query = useQuery({
    queryKey: ['narrative_media', projectKey, locale],
    queryFn: () => narrativeMediaGet(locale),
    retry: false,
  })
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [message, setMessage] = useState('')
  const [input, setInput] = useState('')
  const [directory, setDirectory] = useState('')
  const [csv, setCsv] = useState(true)
  const [preview, setPreview] = useState<MediaDiagnostic[] | null>(null)
  const [audition, setAudition] = useState<Audition | null>(null)
  const [search, setSearch] = useState('')
  const [status, setStatus] = useState('')
  const [page, setPage] = useState(0)
  const [diagnosticPage, setDiagnosticPage] = useState(0)
  const perform = async (action: () => Promise<void>) => {
    setError('')
    setBusy(true)
    try {
      assertProjectSession(epoch)
      await action()
    } catch (failure) {
      if (isProjectSession(epoch)) setError(errorMessage(failure))
    } finally {
      if (isProjectSession(epoch)) setBusy(false)
    }
  }
  const refresh = async () => {
    invalidateNarrative(client)
    await query.refetch()
  }
  const view = query.data
  const invalid = useMemo(() => new Set(view?.diagnostics.map((d) => d.key)), [view])
  const matching = useMemo(
    () =>
      view?.rows.filter(
        (row) =>
          (!status ||
            mediaStatus(row, view.bindings[mediaKey(row.key)], invalid.has(mediaKey(row.key))) ===
              status) &&
          `${row.text} ${row.source.speaker} ${row.source.context} ${row.key.id}`
            .toLocaleLowerCase()
            .includes(search.toLocaleLowerCase()),
      ) ?? [],
    [view, status, search, invalid],
  )
  const lastPage = Math.max(0, Math.ceil(matching.length / 25) - 1),
    currentPage = Math.min(page, lastPage)
  const lastDiagnostic = Math.max(0, Math.ceil((preview?.length ?? 0) / 50) - 1),
    currentDiagnostic = Math.min(diagnosticPage, lastDiagnostic)
  const changeInput = (text: string) => {
    assertProjectSession(epoch)
    setInput(text)
    setPreview(null)
  }
  return (
    <Modal
      onClose={onClose}
      busy={busy}
      titleId="media-title"
      descriptionId="media-description"
      className="sheet nrt-locale-sheet"
    >
      <section className="nrt-locale nrt-media">
        <h2 id="media-title">Recording and prepared media</h2>
        <p id="media-description">
          Export approved, locked wording for recording or external TTS. Import verified PCM WAV and
          optional timing sidecars. Earlier takes remain linked to their exact source and
          translation revisions.
        </p>
        <label>
          Recording locale{' '}
          <input
            value={locale}
            disabled={busy}
            onChange={(e) => {
              setLocale(canonicalLocale(e.target.value))
              setAudition(null)
              setPreview(null)
            }}
          />
        </label>
        {query.isPending ? (
          <p role="status">Reading recording readiness…</p>
        ) : query.isError ? (
          <p role="alert">{errorMessage(query.error)}</p>
        ) : (
          view && (
            <>
              <fieldset disabled={busy}>
                <legend>Recording handoff</legend>
                <InterchangeFormat
                  label="Recording format"
                  csv={csv}
                  onChange={(value) => {
                    setCsv(value)
                    setPreview(null)
                  }}
                />
                <button
                  className="btn"
                  onClick={() =>
                    void perform(async () => {
                      const destination = await save({
                        title: 'New recording manifest',
                        defaultPath: `recording-${locale}.${csv ? 'csv' : 'json'}`,
                      })
                      assertProjectSession(epoch)
                      if (destination) {
                        changeInput(await narrativeMediaExport(locale, csv, destination))
                        setMessage(
                          'Recording manifest exported. Place WAV files at its expected relative paths.',
                        )
                      }
                    })
                  }
                >
                  Export recording script…
                </button>
                <label>
                  Recording manifest{' '}
                  <input
                    type="file"
                    accept=".csv,.json"
                    onChange={(e) => {
                      const file = e.target.files?.[0]
                      if (file) void perform(async () => changeInput(await file.text()))
                    }}
                  />
                </label>
                <label>
                  Manifest contents{' '}
                  <textarea value={input} rows={5} onChange={(e) => changeInput(e.target.value)} />
                </label>
                <p>
                  Static takes need no parameters. For a template, set every named parameter to its
                  exact preformatted spoken value in the manifest. That take only matches those
                  values; other values require another recording or explicitly permitted text
                  fallback.
                </p>
                <button
                  className="btn"
                  onClick={() =>
                    void perform(async () => {
                      const path = await open({
                        directory: true,
                        multiple: false,
                        title: 'Prepared WAV and timing directory',
                      })
                      assertProjectSession(epoch)
                      if (typeof path === 'string') {
                        setDirectory(path)
                        setPreview(null)
                      }
                    })
                  }
                >
                  Choose prepared files directory…
                </button>
                <p>{directory || 'No prepared directory selected'}</p>
                <p>
                  16-bit PCM mono/stereo WAV, 8–96 kHz, at most 32 MiB and ten minutes. Timing JSON
                  uses milliseconds and binds the audio hash. Paths are relative to the selected
                  directory.
                </p>
                <button
                  className="btn"
                  disabled={!input.trim() || !directory}
                  onClick={() =>
                    void perform(async () => {
                      setPreview(await narrativeMediaPreview(input, csv, directory))
                      setDiagnosticPage(0)
                    })
                  }
                >
                  Preview media import
                </button>
                <button
                  className="btn"
                  disabled={readOnly || preview === null}
                  onClick={() =>
                    void perform(async () => {
                      const report = await narrativeMediaImport(input, csv, directory)
                      setPreview(report.diagnostics)
                      setMessage(
                        `${report.applied.length} takes imported; ${Object.keys(report.conflicts).length} write conflicts. ${Object.values(report.conflicts).join(' ')}`,
                      )
                      await refresh()
                    })
                  }
                >
                  Import eligible takes
                </button>
              </fieldset>
              {preview && (
                <section aria-label="Media import preview">
                  <p>
                    {preview.length} diagnostics. Invalid rows are skipped; missing rows retain
                    their takes. Apply rechecks the frozen script and current media guard.
                  </p>
                  <ul>
                    {preview
                      .slice(currentDiagnostic * 50, (currentDiagnostic + 1) * 50)
                      .map((d, i) => (
                        <li key={i}>
                          {d.key}: {d.code} — {d.message}
                        </li>
                      ))}
                  </ul>
                  {lastDiagnostic > 0 && (
                    <PageControls
                      currentPage={currentDiagnostic}
                      lastPage={lastDiagnostic}
                      onPage={setDiagnosticPage}
                    />
                  )}
                </section>
              )}
              <fieldset disabled={busy || readOnly}>
                <legend>Release recording policy</legend>
                <label>
                  <input
                    type="checkbox"
                    checked={locale in view.policy.required}
                    onChange={(e) =>
                      void perform(async () => {
                        const required = { ...view.policy.required }
                        if (e.target.checked) required[locale] = false
                        else delete required[locale]
                        await narrativeMediaPolicy(
                          {
                            ...view.policy,
                            required,
                            timing: view.policy.timing.filter((tag) => tag in required),
                          },
                          view.policy_guard,
                        )
                        await refresh()
                      })
                    }
                  />{' '}
                  Require recordings for this locale in Release
                </label>
                <label>
                  <input
                    type="checkbox"
                    disabled={!(locale in view.policy.required)}
                    checked={view.policy.required[locale] ?? false}
                    onChange={(e) =>
                      void perform(async () => {
                        await narrativeMediaPolicy(
                          {
                            ...view.policy,
                            required: { ...view.policy.required, [locale]: e.target.checked },
                          },
                          view.policy_guard,
                        )
                        await refresh()
                      })
                    }
                  />{' '}
                  Explicitly allow text-only fallback for missing or outdated takes
                </label>
                <label>
                  <input
                    type="checkbox"
                    disabled={!(locale in view.policy.required)}
                    checked={view.policy.timing.includes(locale)}
                    onChange={(e) =>
                      void perform(async () => {
                        const timing = view.policy.timing.filter((tag) => tag !== locale)
                        if (e.target.checked) timing.push(locale)
                        await narrativeMediaPolicy({ ...view.policy, timing }, view.policy_guard)
                        await refresh()
                      })
                    }
                  />{' '}
                  Require timing / lip-sync sidecars for this locale
                </label>
                <p>
                  Translated recording locales must also be configured in Localisation. Development
                  permits text-only playback.
                </p>
              </fieldset>
              <label>
                Media readiness{' '}
                <select
                  value={status}
                  onChange={(e) => {
                    setStatus(e.target.value)
                    setPage(0)
                  }}
                >
                  <option value="">All</option>
                  <option value="missing">Missing recording</option>
                  <option value="out_of_date">Out of date</option>
                  <option value="unavailable">Unavailable file</option>
                  <option value="current">Current recording</option>
                </select>
              </label>
              <label>
                Search recordings{' '}
                <input
                  value={search}
                  onChange={(e) => {
                    setSearch(e.target.value)
                    setPage(0)
                  }}
                />
              </label>
              <p role="status">
                {matching.length} matching of {view.rows.length} recording rows
              </p>
              <PageControls currentPage={currentPage} lastPage={lastPage} onPage={setPage} />
              <p>
                Readiness checks saved script links and file metadata. Content hashes and timing are
                verified on audition and export.
              </p>
              {audition && (
                <MediaAudition
                  key={`${audition.path}/${audition.take.row.source.guard}`}
                  audition={audition}
                />
              )}
              {view.diagnostics.length > 0 && (
                <p role="alert">
                  {view.diagnostics.length} media files are unavailable or invalid. Their histories
                  are retained.
                </p>
              )}
              {matching.slice(currentPage * 25, (currentPage + 1) * 25).map((row) => (
                <MediaLine
                  key={mediaKey(row.key)}
                  row={row}
                  draftKey={`${projectKey}/${mediaKey(row.key)}`}
                  policy={view.policy}
                  guard={view.policy_guard}
                  binding={view.bindings[mediaKey(row.key)]}
                  status={mediaStatus(
                    row,
                    view.bindings[mediaKey(row.key)],
                    invalid.has(mediaKey(row.key)),
                  )}
                  disabled={busy || readOnly}
                  onNotes={(draft) =>
                    void perform(async () => {
                      await narrativeMediaPolicy(
                        {
                          ...draft.policy,
                          notes: { ...draft.policy.notes, [mediaKey(row.key)]: draft.notes },
                        },
                        draft.guard,
                      )
                      useMediaNotes.getState().update(`${projectKey}/${mediaKey(row.key)}`, null)
                      await refresh()
                    })
                  }
                  onAudition={(history) =>
                    void perform(async () =>
                      setAudition(await narrativeMediaAudition(row.key, history)),
                    )
                  }
                />
              ))}
            </>
          )
        )}
        {message && <p role="status">{message}</p>}
        {error && <p role="alert">{error}</p>}
        <button className="btn" disabled={busy} onClick={onClose}>
          Close recording
        </button>
      </section>
    </Modal>
  )
}
