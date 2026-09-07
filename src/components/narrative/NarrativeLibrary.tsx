import { useDeferredValue, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { Scene, SceneCatalog } from '../../lib/api'
import type { NarrativeTarget } from '../../store/ui'
import {
  DEFAULT_LIBRARY_VIEW,
  findScenes,
  type LibraryRow,
  type LibraryView,
  type SceneMatch,
} from './sceneLibraryModel'
import { readPreferences, writePreferences } from './sceneLibraryPreferences'
import './sceneLibrary.css'

const PAGE_SIZE = 25

/** Main discovery surface. Only a page of results is mounted at a time. */
export function NarrativeLibrary({
  projectKey,
  rows,
  catalog,
  loading,
  error,
  readOnly,
  navCollapsed,
  nameOf,
  onOpen,
  onCreateScene,
}: {
  projectKey: string
  rows: LibraryRow[]
  catalog?: SceneCatalog
  loading: boolean
  error?: string
  readOnly: boolean
  navCollapsed: boolean
  nameOf: (id: string) => string | undefined
  onOpen: (target: NarrativeTarget, tab: 'flow' | 'script', variantId?: string) => void
  onCreateScene: () => void
}) {
  const [prefs, setPrefs] = useState(() => readPreferences(projectKey))
  const [viewName, setViewName] = useState('')
  const [matchIndices, setMatchIndices] = useState<Record<string, number>>({})
  const scroll = useRef<HTMLDivElement>(null)
  const deferredView = useDeferredValue(prefs.view)
  const results = useMemo(() => findScenes(rows, deferredView), [rows, deferredView])
  const participants = [
    ...new Set(rows.flatMap((row) => row.scene?.participants?.map((p) => p.entity) ?? [])),
  ].sort((a, b) => (nameOf(a) ?? a).localeCompare(nameOf(b) ?? b))
  const pages = Math.max(1, Math.ceil(results.length / PAGE_SIZE))
  const page = Math.min(prefs.page, pages - 1)
  const visible = results.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE)
  const pending = rows.filter((row) => !row.scene && !row.error).length
  const failed = rows.filter((row) => row.error).length
  useEffect(() => writePreferences(projectKey, prefs), [projectKey, prefs])
  useLayoutEffect(() => {
    if (scroll.current) scroll.current.scrollTop = prefs.scroll
    // Restore once. Subsequent scrolling is owned by the browser while the
    // library stays mounted behind the editor.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  const updateView = (patch: Partial<LibraryView>) => {
    setMatchIndices({})
    setPrefs((p) => ({ ...p, view: { ...p.view, ...patch }, page: 0, scroll: 0 }))
    if (scroll.current) scroll.current.scrollTop = 0
  }
  const open = (target: NarrativeTarget, tab: 'flow' | 'script', variantId?: string) => {
    if (!target.sceneId) return
    setPrefs((p) => ({
      ...p,
      selected: target.sceneId,
      recent: [target.sceneId!, ...p.recent.filter((id) => id !== target.sceneId)].slice(0, 8),
    }))
    onOpen(target, tab, variantId)
  }
  const jumpList = (label: string, ids: string[]) => (
    <section>
      <h3>{label}</h3>
      {ids.length === 0 && <p className="nrt-note">No {label.toLocaleLowerCase()} yet.</p>}
      {ids.map((id) => {
        const row = rows.find((one) => one.summary.id === id)
        return row ? (
          <button
            key={id}
            type="button"
            className="nsl-nav-item"
            onClick={() => open({ sceneId: id }, 'flow')}
          >
            {row.summary.name}
          </button>
        ) : null
      })}
    </section>
  )
  return (
    <div
      className="nsl-workspace"
      style={{
        gridTemplateColumns: navCollapsed
          ? 'minmax(0, 1fr)'
          : 'minmax(160px, var(--nav, 240px)) minmax(0, 1fr)',
      }}
    >
      {!navCollapsed && (
        <nav className="nsl-nav" aria-label="Narrative library">
          <button type="button" className="btn" onClick={() => updateView(DEFAULT_LIBRARY_VIEW)}>
            All scenes
          </button>
          <section>
            <h3>Saved views</h3>
            {prefs.saved.map((saved) => (
              <div className="nsl-saved" key={saved.name}>
                <button
                  type="button"
                  className="nsl-nav-item"
                  onClick={() => updateView(saved.view)}
                >
                  {saved.name}
                </button>
                <button
                  type="button"
                  aria-label={`Delete view ${saved.name}`}
                  onClick={() =>
                    setPrefs((p) => ({ ...p, saved: p.saved.filter((s) => s.name !== saved.name) }))
                  }
                >
                  ×
                </button>
              </div>
            ))}
            {!prefs.saved.length && (
              <p className="nrt-note">Save a search and filters for later.</p>
            )}
          </section>
          {jumpList('Pinned scenes', prefs.pins)}
          {jumpList('Recent scenes', prefs.recent)}
        </nav>
      )}
      <main className="nsl-main" aria-label="Scene library">
        <header className="nsl-title">
          <div>
            <h2>Scenes</h2>
            <p className="nrt-note">Find a scene, then open its structure or dialogue.</p>
          </div>
          <button
            type="button"
            className="btn btn-primary"
            disabled={readOnly}
            title={readOnly ? 'This project is read-only' : undefined}
            onClick={onCreateScene}
          >
            {rows.length ? 'New scene' : 'Create first scene'}
          </button>
        </header>
        <label className="nsl-search-label">
          Find a scene or a line
          <input
            className="nrt-search"
            type="search"
            value={prefs.view.query}
            placeholder="Search titles, intent and dialogue…"
            onChange={(e) => updateView({ query: e.target.value })}
          />
        </label>
        <div className="nsl-filters">
          <label>
            Participant
            <select
              value={prefs.view.participant}
              onChange={(e) => updateView({ participant: e.target.value })}
            >
              <option value="">All participants</option>
              {participants.map((id) => (
                <option key={id} value={id}>
                  {nameOf(id) ?? id}
                </option>
              ))}
            </select>
          </label>
          <label>
            Policy
            <select
              value={prefs.view.policy}
              onChange={(e) => updateView({ policy: e.target.value as LibraryView['policy'] })}
            >
              <option value="">Any policy</option>
              <option value="generated">Generated</option>
              <option value="edited">Edited</option>
              <option value="locked">Locked</option>
            </select>
          </label>
          <label>
            Approval
            <select
              value={prefs.view.review}
              onChange={(e) => updateView({ review: e.target.value as LibraryView['review'] })}
            >
              <option value="">Any approval</option>
              <option value="draft">Draft</option>
              <option value="approved">Approved</option>
            </select>
          </label>
          <label>
            Freshness
            <select
              value={prefs.view.freshness}
              onChange={(e) =>
                updateView({ freshness: e.target.value as LibraryView['freshness'] })
              }
            >
              <option value="">Any freshness</option>
              <option value="current">Current</option>
              <option value="out_of_date">Out of date</option>
            </select>
          </label>
          <label>
            <input
              type="checkbox"
              checked={prefs.view.missing}
              onChange={(e) => updateView({ missing: e.target.checked })}
            />
            Needs text
          </label>
          <label>
            <input
              type="checkbox"
              checked={prefs.view.includeDrafts}
              onChange={(e) => updateView({ includeDrafts: e.target.checked })}
            />
            Include generated drafts
          </label>
          <button type="button" className="btn" onClick={() => updateView(DEFAULT_LIBRARY_VIEW)}>
            Clear filters
          </button>
        </div>
        <form
          className="nsl-save"
          onSubmit={(event) => {
            event.preventDefault()
            const name = viewName.trim()
            if (!name) return
            setPrefs((p) => ({
              ...p,
              saved: [...p.saved.filter((s) => s.name !== name), { name, view: { ...p.view } }],
            }))
            setViewName('')
          }}
        >
          <label>
            View name
            <input value={viewName} onChange={(e) => setViewName(e.target.value)} maxLength={80} />
          </label>
          <button className="btn" type="submit" disabled={!viewName.trim()}>
            Save view
          </button>
          <label>
            Sort
            <select
              value={prefs.view.sort}
              onChange={(e) => updateView({ sort: e.target.value as LibraryView['sort'] })}
            >
              <option value="name">Title A–Z</option>
              <option value="nameDescending">Title Z–A</option>
            </select>
          </label>
        </form>
        <p className="nrt-note">
          Act, quest and tags are not available yet. Status reflects recorded source values;
          automatic freshness tracking is not available.
        </p>
        <p role="status" aria-live="polite">
          {loading ? 'Reading scenes…' : `${results.length} matching of ${rows.length} scenes`}
          {pending > 0 && ` · Reading text from ${pending} scenes; results are incomplete.`}
          {failed > 0 && ` · ${failed} scenes could not be read; text results are incomplete.`}
        </p>
        {error && <p role="alert">Could not read the scene catalog: {error}</p>}
        {(catalog?.unreadable ?? []).map((one) => (
          <p role="alert" key={one.rel}>
            {one.rel} could not be read: {one.reason}. It is still on disk.
          </p>
        ))}
        <div
          className="nsl-table-scroll"
          ref={scroll}
          onScroll={(e) => {
            const top = e.currentTarget.scrollTop
            setPrefs((p) => ({ ...p, scroll: top }))
          }}
        >
          <table className="nsl-table">
            <caption className="sr-only">Scene search results</caption>
            <thead>
              <tr>
                <th scope="col">Scene</th>
                <th scope="col">Participants</th>
                <th scope="col">Text coverage</th>
                <th scope="col">Recorded status</th>
                <th scope="col">Open</th>
                <th scope="col">Pin</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((row) => {
                const matchIndex = Math.min(
                  matchIndices[row.summary.id] ?? 0,
                  Math.max(0, row.matches.length - 1),
                )
                const match = row.matches[matchIndex]
                const target: SceneMatch = match ?? { sceneId: row.summary.id, snippet: '' }
                return (
                  <tr
                    key={row.summary.id}
                    className={prefs.selected === row.summary.id ? 'is-selected' : undefined}
                  >
                    <th scope="row">
                      <span>{row.summary.name}</span>
                      {row.error && <p role="alert">Could not read: {row.error}</p>}
                      {match && (
                        <p className="nsl-snippet">
                          {match.draft && <b>Generated draft: </b>}
                          {match.snippet}
                        </p>
                      )}
                      {row.matches.length > 1 && (
                        <label>
                          Matching passage
                          <select
                            aria-label={`Matching passage in ${row.summary.name}`}
                            value={matchIndex}
                            onChange={(e) =>
                              setMatchIndices((all) => ({
                                ...all,
                                [row.summary.id]: Number(e.target.value),
                              }))
                            }
                          >
                            {row.matches.map((one, index) => (
                              <option key={index} value={index}>
                                {index + 1}: {one.snippet}
                              </option>
                            ))}
                          </select>
                        </label>
                      )}
                    </th>
                    <td>
                      {row.scene
                        ? row.scene.participants
                            ?.map((p) => nameOf(p.entity) ?? p.entity)
                            .join(', ') || 'None declared'
                        : 'Not read'}
                    </td>
                    <td>{row.scene ? `${row.filled} / ${row.slots} slots` : 'Not read'}</td>
                    <td>{row.scene ? <SceneStatus scene={row.scene} /> : 'Not read'}</td>
                    <td>
                      <div className="nsl-open">
                        <button
                          type="button"
                          className="btn"
                          onClick={() => open(target, 'flow', target.variantId)}
                          aria-label={`Open ${row.summary.name} in Flow`}
                        >
                          Flow
                        </button>
                        <button
                          type="button"
                          className="btn"
                          onClick={() => open(target, 'script', target.variantId)}
                          aria-label={`Open ${row.summary.name} in Script`}
                        >
                          Script
                        </button>
                      </div>
                    </td>
                    <td>
                      <button
                        type="button"
                        className="btn"
                        aria-label={`Pin ${row.summary.name}`}
                        aria-pressed={prefs.pins.includes(row.summary.id)}
                        onClick={() =>
                          setPrefs((p) => ({
                            ...p,
                            pins: p.pins.includes(row.summary.id)
                              ? p.pins.filter((id) => id !== row.summary.id)
                              : [...p.pins, row.summary.id],
                          }))
                        }
                      >
                        {prefs.pins.includes(row.summary.id) ? 'Pinned' : 'Pin'}
                      </button>
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
          {!loading && !error && !results.length && (
            <p className="nsl-empty">
              {rows.length
                ? 'No matching scenes. Change or clear the filters.'
                : 'No scenes yet. Create a scene to start writing.'}
            </p>
          )}
        </div>
        <div className="nsl-pages">
          <button
            className="btn"
            type="button"
            disabled={page === 0}
            onClick={() => setPrefs((p) => ({ ...p, page: page - 1 }))}
          >
            Previous page
          </button>
          <span>
            Page {page + 1} of {pages}
          </span>
          <button
            className="btn"
            type="button"
            disabled={page + 1 >= pages}
            onClick={() => setPrefs((p) => ({ ...p, page: page + 1 }))}
          >
            Next page
          </button>
        </div>
      </main>
    </div>
  )
}

function SceneStatus({ scene }: { scene: Scene }) {
  const slots = scene.beats?.flatMap((beat) => beat.dialogue ?? []) ?? []
  const texts = slots.flatMap((slot) =>
    (slot.variants ?? []).map((variant) => ({
      policy:
        slot.policy === 'locked'
          ? 'locked'
          : (variant.text.lifecycle?.policy ?? slot.policy ?? 'edited'),
      review: variant.text.lifecycle?.review ?? 'draft',
      freshness: variant.text.lifecycle?.freshness ?? 'current',
    })),
  )
  if (!texts.length) return <span>No text status</span>
  const policies = [...new Set(texts.map((text) => text.policy))].join(', ')
  const draft = texts.filter((text) => text.review === 'draft').length
  const stale = texts.filter((text) => text.freshness === 'out_of_date').length
  return (
    <span>
      {policies}
      <br />
      {draft} draft · {texts.length - draft} approved
      <br />
      {stale} out of date
    </span>
  )
}
