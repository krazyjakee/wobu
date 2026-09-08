import { useDeferredValue, useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { LibraryLabel, LibraryMatch, LibrarySceneRow } from '../../lib/api/narrativeLibrary'
import { useNarrativeLibraryQuery } from '../../lib/queries/narrativeLibrary'
import type { NarrativeTarget } from '../../store/ui'
import { DEFAULT_LIBRARY_VIEW, type LibraryView } from './sceneLibraryModel'
import { readPreferences, writePreferences } from './sceneLibraryPreferences'
import { sceneEditKey, useScriptDrafts } from './scriptDrafts'
import './sceneLibrary.css'

const PAGE_SIZE = 25

/** Main discovery surface. Only a page of results is mounted at a time. */
function LibrarySession({
  projectKey,
  readOnly,
  navCollapsed,
  nameOf,
  onOpen,
  onCreateScene,
  onRenameScene,
  onRepair,
  onOpenExample,
}: {
  projectKey: string
  readOnly: boolean
  navCollapsed: boolean
  nameOf: (id: string) => string | undefined
  onOpen: (target: NarrativeTarget, tab: 'flow' | 'script', variantId?: string) => void
  onCreateScene: () => void
  onRenameScene?: (sceneId: string, name: string) => void
  onRepair?: (rel: string) => void
  onOpenExample?: () => void
}) {
  const [prefs, setPrefs] = useState(() => readPreferences(projectKey))
  const [viewName, setViewName] = useState('')
  // Every scene's draft, not this row's: a hook cannot be called per row, and
  // the record's identity only changes when some draft actually does.
  const drafts = useScriptDrafts((state) => state.drafts)
  const [matchIndices, setMatchIndices] = useState<Record<string, number>>({})
  const scroll = useRef<HTMLDivElement>(null)
  const restoreScroll = useRef<number | null>(prefs.scroll)
  const deferredView = useDeferredValue(prefs.view)
  const [unreadableOffset, setUnreadableOffset] = useState(0)
  const [pinOffset, setPinOffset] = useState(0)
  const query = useNarrativeLibraryQuery(projectKey, {
    ...deferredView,
    offset: prefs.page * PAGE_SIZE,
    limit: PAGE_SIZE,
    revision: prefs.revision,
    unreadableOffset,
  })
  const lookupIds = [...new Set([...prefs.pins.slice(pinOffset, pinOffset + 8), ...prefs.recent])]
  const lookup = useNarrativeLibraryQuery(
    projectKey,
    {
      ...DEFAULT_LIBRARY_VIEW,
      offset: 0,
      limit: 16,
      revision: null,
      ids: lookupIds,
    },
    lookupIds.length > 0,
  )
  const data = query.data
  const visible = data?.rows ?? []
  const pages = Math.max(1, Math.ceil((data?.total ?? 0) / PAGE_SIZE))
  const page = prefs.page
  const loading = query.isFetching
  const error = query.error?.message
  const facet = (key: 'quest' | 'act' | 'arc' | 'tag', label: string, records: LibraryLabel[]) => (
    <label>
      {label}
      <select
        value={prefs.view[key]}
        onChange={(event) => updateView({ [key]: event.target.value })}
      >
        <option value="">All {label.toLowerCase()}s</option>
        {prefs.view[key] && !records.some((record) => record.id === prefs.view[key]) && (
          <option value={prefs.view[key]}>
            Unavailable {label.toLowerCase()} ({prefs.view[key]})
          </option>
        )}
        {records.map((record) => (
          <option key={record.id} value={record.id}>
            {record.name}
          </option>
        ))}
      </select>
    </label>
  )
  const restart = () => {
    restoreScroll.current = null
    if (scroll.current) scroll.current.scrollTop = 0
    setUnreadableOffset(0)
    setPrefs((p) => ({ ...p, page: 0, revision: null, scroll: 0 }))
    if (prefs.page === 0 && !prefs.revision && !unreadableOffset) void query.refetch()
  }
  useEffect(() => writePreferences(projectKey, prefs), [projectKey, prefs])
  useLayoutEffect(() => {
    // The initial query can resolve after mount. Restore only once rows exist;
    // setting scrollTop on the empty table would clamp the saved position to zero.
    if (data && scroll.current && restoreScroll.current !== null) {
      scroll.current.scrollTop = restoreScroll.current
      restoreScroll.current = null
    }
  }, [data])
  const updateView = (patch: Partial<LibraryView>) => {
    restoreScroll.current = null
    setMatchIndices({})
    setUnreadableOffset(0)
    setPrefs((p) => ({ ...p, view: { ...p.view, ...patch }, page: 0, revision: null, scroll: 0 }))
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
        const row = lookup.data?.rows.find((one) => one.summary.id === id)
        return row ? (
          <button
            key={id}
            type="button"
            className="nsl-nav-item"
            onClick={() => open({ sceneId: id }, 'flow')}
          >
            {row.summary.name}
          </button>
        ) : (
          <p key={id} className="nrt-note">
            {lookup.isFetching ? 'Reading saved scene…' : `Unavailable scene (${id})`}
          </p>
        )
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
          {jumpList('Pinned scenes', prefs.pins.slice(pinOffset, pinOffset + 8))}
          {prefs.pins.length > 8 && (
            <div className="nsl-pages">
              <button
                disabled={!pinOffset}
                onClick={() => setPinOffset(Math.max(0, pinOffset - 8))}
              >
                Previous pins
              </button>
              <button
                disabled={pinOffset + 8 >= prefs.pins.length}
                onClick={() => setPinOffset(pinOffset + 8)}
              >
                More pins
              </button>
            </div>
          )}
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
            {data?.sceneCount ? 'New scene' : 'Create first scene'}
          </button>
        </header>
        {onOpenExample && (
          <button type="button" className="btn" disabled={readOnly} onClick={onOpenExample}>
            Open Ashfall example
          </button>
        )}
        <label className="nsl-search-label">
          Find a scene or a line
          <input
            className="nrt-search"
            type="search"
            maxLength={500}
            value={prefs.view.query}
            placeholder="Search titles, intent and dialogue…"
            onChange={(e) => updateView({ query: e.target.value })}
          />
        </label>
        <div className="nsl-filters">
          {facet('quest', 'Quest', data?.facets.quests ?? [])}
          {facet('act', 'Act', data?.facets.acts ?? [])}
          {facet('arc', 'Arc', data?.facets.arcs ?? [])}
          {facet('tag', 'Tag', data?.facets.tags ?? [])}
          <label>
            Participant
            <select
              value={prefs.view.participant}
              onChange={(e) => updateView({ participant: e.target.value })}
            >
              <option value="">All participants</option>
              {prefs.view.participant &&
                !data?.facets.participants.includes(prefs.view.participant) && (
                  <option value={prefs.view.participant}>
                    Unavailable participant ({prefs.view.participant})
                  </option>
                )}
              {(data?.facets.participants ?? []).map((id) => (
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
            Search unapproved generated wording
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
          Status reflects recorded source values. Review verifies current approval and context.
          Search uses saved wording; unapproved generated wording is optional.
        </p>
        <p role="status" aria-live="polite">
          {loading
            ? 'Reading scenes…'
            : `${data?.total ?? 0} matching of ${data?.sceneCount ?? 0} scenes`}
          {Boolean(data?.unreadableTotal) &&
            ` · ${data!.unreadableTotal} unreadable or ambiguous files`}
        </p>
        {error && (
          <p role="alert">
            Could not read scene results: {error}{' '}
            <button type="button" className="btn" onClick={restart}>
              Restart results
            </button>
          </p>
        )}
        {(data?.unreadable ?? []).map((one) => (
          <p role="alert" key={one.rel}>
            {one.rel} could not be read: {one.reason}. It is still on disk.
            {onRepair && (
              <button type="button" className="btn" onClick={() => onRepair(one.rel)}>
                Repair source
              </button>
            )}
          </p>
        ))}
        {(unreadableOffset > 0 || data?.unreadableNextOffset != null) && (
          <div className="nsl-pages">
            <button
              type="button"
              disabled={!unreadableOffset || loading}
              onClick={() => {
                setUnreadableOffset(Math.max(0, unreadableOffset - PAGE_SIZE))
                setPrefs((p) => ({ ...p, revision: data?.revision ?? p.revision }))
              }}
            >
              Previous source errors
            </button>
            <button
              type="button"
              disabled={data?.unreadableNextOffset == null || loading}
              onClick={() => {
                setUnreadableOffset(data!.unreadableNextOffset!)
                setPrefs((p) => ({ ...p, revision: data!.revision }))
              }}
            >
              More source errors
            </button>
          </div>
        )}
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
                <th scope="col">Act</th>
                <th scope="col">Arc</th>
                <th scope="col">Tags</th>
                <th scope="col">Quests</th>
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
                const target: LibraryMatch = match ?? {
                  sceneId: row.summary.id,
                  snippet: '',
                  draft: false,
                }
                return (
                  <tr
                    key={row.summary.id}
                    className={prefs.selected === row.summary.id ? 'is-selected' : undefined}
                  >
                    <th scope="row">
                      <span>{row.summary.name}</span>
                      {onRenameScene && (
                        <SceneRename
                          name={row.summary.name}
                          readOnly={readOnly}
                          drafted={!!drafts[sceneEditKey(projectKey, row.summary.id)]}
                          onRename={(name) => onRenameScene(row.summary.id, name)}
                        />
                      )}

                      {match && (
                        <p className="nsl-snippet">
                          {match.draft && <b>Generated draft: </b>}
                          {match.snippet}
                        </p>
                      )}
                      {row.matchCount > row.matches.length && (
                        <p className="nrt-note">
                          Showing {row.matches.length} of {row.matchCount} passages. Refine the
                          search for another line.
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
                    <td>{row.act?.name ?? 'Unassigned'}</td>
                    <td>{row.arc?.name ?? 'Unassigned'}</td>
                    <td>{row.tags.map((tag) => tag.name).join(', ') || 'No tags'}</td>
                    <td className="nsl-quests">
                      {row.quests.length ? (
                        <ul aria-label={`Quests for ${row.summary.name}`}>
                          {row.quests.map((quest) => (
                            <li key={quest.id}>{quest.name}</li>
                          ))}
                        </ul>
                      ) : (
                        'No quests'
                      )}
                    </td>
                    <td>
                      {row.participants.map((id) => nameOf(id) ?? id).join(', ') || 'None declared'}
                    </td>
                    <td>
                      {row.filled} / {row.slots} slots
                    </td>
                    <td>
                      <SceneStatus row={row} />
                    </td>
                    <td>
                      <div className="nsl-open">
                        <button
                          type="button"
                          className="btn"
                          data-library-scene={row.summary.id}
                          data-library-tab="flow"
                          onClick={() => open(target, 'flow', target.variantId)}
                          aria-label={`Open ${row.summary.name} in Flow`}
                        >
                          Flow
                        </button>
                        <button
                          type="button"
                          className="btn"
                          data-library-scene={row.summary.id}
                          data-library-tab="script"
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
          {!loading && !error && !data?.total && (
            <p className="nsl-empty">
              {data?.sceneCount
                ? 'No matching scenes. Change or clear the filters.'
                : 'No scenes yet. Create a scene to start writing.'}
            </p>
          )}
        </div>
        <div className="nsl-pages">
          <button
            className="btn"
            type="button"
            disabled={page === 0 || loading || Boolean(error)}
            onClick={() => setPrefs((p) => ({ ...p, page: page - 1, revision: data!.revision }))}
          >
            Previous page
          </button>
          <span>
            Page {page + 1} of {pages}
          </span>
          <button
            className="btn"
            type="button"
            disabled={page + 1 >= pages || loading || Boolean(error)}
            onClick={() => setPrefs((p) => ({ ...p, page: page + 1, revision: data!.revision }))}
          >
            Next page
          </button>
        </div>
      </main>
    </div>
  )
}

/**
 * Rename one scene from its row.
 *
 * Offered here as well as on the Scene name field inside Script, because the
 * writer who has just read forty titles is the one who notices that two of them
 * say the same thing, and making them open a scene to fix its name is how a
 * library full of "New scene" stays that way. It goes through
 * `narrative_scene_rename`, which changes the display name and nothing else —
 * the slug, and therefore the file on the shared folder, is minted once at
 * create and never moves.
 *
 * Refused while that scene has an unsaved draft. The rename is a whole-document
 * save on the backend's side, so it would land underneath work the writer has
 * not saved and turn their next save into a conflict. The row says so rather
 * than hiding a disabled button, because "why can I not rename this" is the
 * question a greyed control leaves behind.
 *
 * Its own local state: the form belongs to one row, and lifting it would make
 * the whole table re-render on every keystroke of a name nobody else can see.
 */
function SceneRename({
  name,
  readOnly,
  drafted,
  onRename,
}: {
  name: string
  readOnly: boolean
  drafted: boolean
  onRename: (name: string) => void
}) {
  const [editing, setEditing] = useState<string | null>(null)
  const input = useRef<HTMLInputElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  const restoring = useRef(false)
  const isEditing = editing !== null
  useLayoutEffect(() => {
    if (isEditing) input.current?.focus()
    else if (restoring.current) {
      trigger.current?.focus()
      restoring.current = false
    }
  }, [isEditing])
  const finish = () => {
    restoring.current = true
    setEditing(null)
  }
  if (drafted)
    return <p className="nrt-note">Unsaved draft — save or discard it before renaming.</p>
  if (editing === null)
    return (
      <button
        ref={trigger}
        type="button"
        className="btn"
        disabled={readOnly}
        title={readOnly ? 'This project is read-only' : undefined}
        aria-label={`Rename ${name}`}
        onClick={() => setEditing(name)}
      >
        Rename
      </button>
    )
  return (
    <form
      className="nsl-rename"
      onKeyDown={(event) => {
        if (event.key !== 'Escape' || event.defaultPrevented) return
        event.preventDefault()
        finish()
      }}
      onSubmit={(event) => {
        event.preventDefault()
        const next = editing.trim()
        if (!next) return
        finish()
        if (next !== name) onRename(next)
      }}
    >
      <label>
        New name for {name}
        <input
          ref={input}
          value={editing}
          maxLength={200}
          onChange={(e) => setEditing(e.target.value)}
        />
      </label>
      <button type="submit" className="btn" disabled={!editing.trim()}>
        Save name
      </button>
      <button type="button" className="btn" onClick={finish}>
        Cancel rename
      </button>
    </form>
  )
}

function SceneStatus({ row }: { row: LibrarySceneRow }) {
  const { generated, edited, locked, needsReview, outOfDate } = row.counts
  const total = generated + edited + locked
  return (
    <span>
      {generated} generated · {edited} edited · {locked} locked
      <br />
      {needsReview} draft · {total - needsReview} approved
      <br />
      {outOfDate} out of date
    </span>
  )
}

export function NarrativeLibrary(props: Parameters<typeof LibrarySession>[0]) {
  return <LibrarySession key={props.projectKey} {...props} />
}
