import { useState } from 'react'
import type { ReviewTarget } from '../../lib/api/narrativeReview'
import { textDraftKey, useTextDrafts } from './textDrafts'
import { useQueryClient } from '@tanstack/react-query'
import { errorMessage, preconditionOf } from '../../lib/api'
import type { Speaker } from '../../lib/api'
import {
  narrativeTextCreate,
  narrativeTextDelete,
  narrativeTextSave,
  type TextAsset,
  type TextAssetId,
  type TextEntry,
  type TextKind,
  type TextFile,
} from '../../lib/api/narrativeText'
import {
  useNarrativeText,
  useNarrativeTextDiagnostics,
  useNarrativeTexts,
} from '../../lib/queries/narrativeText'
import { assertProjectSession, projectSessionEpoch } from '../../lib/projectSession'
import { mintId } from './sceneIdentity'
import { REPEAT_LABELS, TEXT_TEMPLATES, prepareTextAsset, templateOf } from './textLibraryModel'
import './textLibrary.css'
import { TextEditorial } from './TextEditorial'
import { TextAssetContext } from './TextAssetContext'
import { TextLineFields } from './TextLineFields'
import { TypedCondition } from './TypedCondition'
import { useNarrativeState } from '../../lib/queries'

/** The six text kinds share canonical slots, generation and guarded review. */
export function NarrativeTextLibrary(props: {
  projectKey: string
  initialAssetId?: TextAssetId
  initialTarget?: ReviewTarget
  readOnly: boolean
  /** Resolves a world entity id to a name, or `undefined` when nothing can. */
  nameOf: (entity: string) => string | undefined
  onClose: () => void
}) {
  return <TextLibrary key={props.projectKey} {...props} />
}

function TextLibrary({
  projectKey,
  initialAssetId,
  initialTarget,
  readOnly,
  nameOf,
  onClose,
}: {
  projectKey: string
  initialAssetId?: TextAssetId
  initialTarget?: ReviewTarget
  readOnly: boolean
  nameOf: (entity: string) => string | undefined
  onClose: () => void
}) {
  const client = useQueryClient()
  const catalog = useNarrativeTexts(projectKey)
  const [selected, setSelected] = useState<TextAssetId | null>(
    initialTarget?.scene ?? initialAssetId ?? null,
  )
  const file = useNarrativeText(projectKey, selected)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const loaded = file.data
  const [query, setQuery] = useState('')
  const [kindFilter, setKindFilter] = useState('')
  const [page, setPage] = useState(0)
  const matching = (catalog.data?.assets ?? []).filter(
    (asset) =>
      (!kindFilter || asset.kind === kindFilter) &&
      `${asset.name} ${templateOf(asset.kind).label}`
        .toLocaleLowerCase()
        .includes(query.trim().toLocaleLowerCase()),
  )
  const lastPage = Math.max(0, Math.ceil(matching.length / 25) - 1)
  const currentPage = Math.min(page, lastPage)
  const visible = matching.slice(currentPage * 25, (currentPage + 1) * 25)

  const refresh = async () => {
    await client.invalidateQueries({ queryKey: ['narrative_texts', projectKey] })
    await client.invalidateQueries({ queryKey: ['narrative_text', projectKey] })
  }

  const run = async (work: () => Promise<TextFile | void>) => {
    const epoch = projectSessionEpoch()
    setBusy(true)
    setError(null)
    try {
      const result = await work()
      assertProjectSession(epoch)
      await refresh()
      if (result) setSelected(result.asset.id)
      return result
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setBusy(false)
    }
  }

  return (
    // A section rather than a div: an `aria-label` on a nameless element is
    // ignored, so the landmark this pane replaces the Scene library with would
    // have had no name at all.
    <section className="ntl-view" aria-label="Text library">
      <header className="ntl-head">
        <h2>Text library</h2>
        <p>
          Barks, ambient exchanges, companion reactions, codex entries, quest summaries and
          journals. They use the same wording identities, generation policy and export path as scene
          dialogue, and they need no player choice to exist.
        </p>
        <button type="button" className="btn" onClick={onClose}>
          Back to scenes
        </button>
      </header>

      {error && (
        <p role="alert" className="ntl-error">
          {error}
        </p>
      )}

      <div className="ntl-body">
        <nav className="ntl-list" aria-label="Supporting text assets">
          <CreateForm
            readOnly={readOnly || busy}
            onCreate={(kind, name, event) =>
              run(() => narrativeTextCreate(kind, name.trim(), event.trim()))
            }
          />
          {/* The catalog's own states, in the list rather than above it: a
              read that is still in flight and a read that failed are facts
              about this list, and putting them where the assets would be is
              what keeps an empty pane from reading as an empty project. */}
          {catalog.isPending && <p className="ntl-empty">Reading supporting text…</p>}
          {catalog.isError && (
            <p role="alert" className="ntl-error">
              Could not read supporting text: {errorMessage(catalog.error)}
            </p>
          )}
          <label>
            Search text library
            <input
              value={query}
              onChange={(event) => {
                setQuery(event.target.value)
                setPage(0)
              }}
            />
          </label>
          <label>
            Content type
            <select
              value={kindFilter}
              onChange={(event) => {
                setKindFilter(event.target.value)
                setPage(0)
              }}
            >
              <option value="">All types</option>
              {TEXT_TEMPLATES.map((template) => (
                <option key={template.kind} value={template.kind}>
                  {template.label}
                </option>
              ))}
            </select>
          </label>
          <p role="status">
            {matching.length} matching of {catalog.data?.assets.length ?? 0} assets
          </p>
          <ul>
            {visible.map((asset) => (
              <li key={asset.id}>
                <button
                  type="button"
                  data-text-asset={asset.id}
                  aria-current={asset.id === selected ? 'true' : undefined}
                  onClick={() => setSelected(asset.id)}
                >
                  <span className="ntl-kind">{templateOf(asset.kind).label}</span>
                  {asset.name}
                </button>
              </li>
            ))}
          </ul>
          {matching.length > 25 && (
            <div className="ntl-actions" aria-label="Text library pages">
              <button
                className="btn"
                disabled={currentPage === 0}
                onClick={() => setPage(currentPage - 1)}
              >
                Previous page
              </button>
              <span>
                Page {currentPage + 1} of {lastPage + 1}
              </span>
              <button
                className="btn"
                disabled={currentPage === lastPage}
                onClick={() => setPage(currentPage + 1)}
              >
                Next page
              </button>
            </div>
          )}
          {catalog.data?.assets.length === 0 && (
            <p className="ntl-empty">No supporting text yet. Choose a template above.</p>
          )}
          {(catalog.data?.unreadable ?? []).map((bad) => (
            <p key={bad.rel} role="alert" className="ntl-error">
              {bad.rel} could not be read: {bad.reason}
            </p>
          ))}
        </nav>

        <main className="ntl-editor" aria-label="Supporting text editor">
          {loaded ? (
            <AssetEditor
              // Keep the original revision while a dirty draft is open; background
              // refresh must not discard words or silently advance the save guard.
              key={loaded.asset.id}
              projectKey={projectKey}
              file={loaded}
              readOnly={readOnly || busy}
              busy={busy}
              initialTarget={initialTarget}
              nameOf={nameOf}
              onSave={(baseline, asset) =>
                run(async () => {
                  const epoch = projectSessionEpoch()
                  const prepared = await prepareTextAsset(baseline.asset, asset)
                  assertProjectSession(epoch)
                  return narrativeTextSave(prepared, preconditionOf(baseline.stamp))
                })
              }
              onDelete={() =>
                run(async () => {
                  await narrativeTextDelete(loaded.asset.id)
                  setSelected(null)
                })
              }
            />
          ) : !selected ? (
            <p className="ntl-empty">Select an asset to edit it.</p>
          ) : file.isError ? (
            // Named rather than silent. The asset is in the catalog, so the
            // list is offering something the editor cannot open, and "select an
            // asset" would be advice the writer has already followed.
            <p role="alert" className="ntl-error">
              Could not read this asset: {errorMessage(file.error)}
            </p>
          ) : (
            <p className="ntl-empty">Reading this asset…</p>
          )}
        </main>
      </div>
    </section>
  )
}

/** One asset buffer keeps its original guard across navigation and background refresh. */
function AssetEditor({
  projectKey,
  file,
  initialTarget,
  readOnly,
  busy,
  nameOf,
  onSave,
  onDelete,
}: {
  projectKey: string
  file: TextFile
  initialTarget?: ReviewTarget
  readOnly: boolean
  busy: boolean
  nameOf: (entity: string) => string | undefined
  onSave: (baseline: TextFile, asset: TextAsset) => Promise<TextFile | void>
  onDelete: () => void
}) {
  const key = textDraftKey(projectKey, file.asset.id)
  const retained = useTextDrafts((state) => state.drafts[key])
  const baseline = retained?.file ?? file
  const draft = retained?.asset ?? file.asset
  const setDraft = (asset: TextAsset) => useTextDrafts.getState().put(key, baseline, asset)
  const [target, setTarget] = useState<ReviewTarget | undefined>(initialTarget)
  const diagnostics = useNarrativeTextDiagnostics(projectKey, file.asset.id, draft)
  const template = templateOf(draft.kind)
  const dirty = JSON.stringify(draft) !== JSON.stringify(baseline.asset)
  const changed = file.stamp?.hash !== baseline.stamp?.hash
  return (
    <>
      {changed && dirty && (
        <p role="alert">
          The saved asset changed. Your draft is retained; saving will check the original revision.
        </p>
      )}
      <div className="ntl-fields">
        <label>
          Name
          <input
            value={draft.name}
            disabled={readOnly}
            onChange={(changed) => setDraft({ ...draft, name: changed.target.value })}
          />
        </label>
        <label>
          Trigger
          <input
            value={draft.trigger.event}
            disabled={readOnly}
            onChange={(changed) =>
              setDraft({ ...draft, trigger: { ...draft.trigger, event: changed.target.value } })
            }
          />
        </label>
        <label>
          Selection
          <select
            value={draft.repeat ?? 'first'}
            disabled={readOnly}
            onChange={(changed) =>
              setDraft({
                ...draft,
                repeat: changed.target.value as NonNullable<TextAsset['repeat']>,
              })
            }
          >
            {Object.entries(REPEAT_LABELS).map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label className="ntl-wide">
          What this is for
          <textarea
            rows={2}
            value={draft.summary ?? ''}
            disabled={readOnly}
            onChange={(changed) => setDraft({ ...draft, summary: changed.target.value })}
          />
        </label>
        <label className="ntl-check">
          <input
            type="checkbox"
            checked={draft.policy === 'locked'}
            disabled={readOnly}
            onChange={(changed) =>
              setDraft({ ...draft, policy: changed.target.checked ? 'locked' : 'edited' })
            }
          />
          Locked — no generation job may write anywhere in this asset
        </label>
      </div>

      <TextAssetContext asset={draft} disabled={readOnly} onChange={setDraft} />
      <p className="ntl-hint">{template.hint}</p>

      <ol className="ntl-entries">
        {(draft.entries ?? []).map((entry, index) => (
          <EntryFields
            key={entry.id}
            entry={entry}
            index={index}
            cast={template.cast}
            participants={draft.participants ?? []}
            target={target}
            sequence={template.sequence}
            readOnly={readOnly}
            nameOf={nameOf}
            onChange={(next) =>
              setDraft({
                ...draft,
                entries: (draft.entries ?? []).map((one) => (one.id === entry.id ? next : one)),
              })
            }
            onRemove={() =>
              setDraft({
                ...draft,
                entries: (draft.entries ?? []).filter((one) => one.id !== entry.id),
              })
            }
          />
        ))}
      </ol>

      <div className="ntl-actions">
        <button
          type="button"
          className="btn"
          disabled={readOnly}
          onClick={() =>
            setDraft({ ...draft, entries: [...(draft.entries ?? []), blankEntry(template.cast)] })
          }
        >
          Add entry
        </button>
        <button
          type="button"
          className="btn btn-primary"
          disabled={readOnly || busy || !dirty}
          onClick={async () => {
            const saved = await onSave(baseline, draft)
            if (saved) {
              useTextDrafts.getState().clear(key, retained)
            }
          }}
        >
          Save
        </button>
        <button
          type="button"
          className="btn"
          disabled={!dirty || busy}
          onClick={() => useTextDrafts.getState().clear(key)}
        >
          Discard draft
        </button>
        <button
          type="button"
          className="btn"
          disabled={readOnly || busy || dirty}
          onClick={onDelete}
        >
          Delete
        </button>
      </div>

      <TextEditorial
        asset={baseline.asset}
        projectKey={projectKey}
        readOnly={readOnly}
        dirty={dirty}
        nameOf={nameOf}
        onSource={(next) => setTarget({ ...next })}
      />
      <section aria-label="Supporting text diagnostics" className="ntl-diagnostics">
        <h3>{diagnostics.data?.length ?? 0} problems</h3>
        <ul>
          {(diagnostics.data ?? []).map((problem, index) => (
            <li key={`${problem.code}-${index}`}>{problem.message}</li>
          ))}
        </ul>
      </section>
    </>
  )
}

/** A new asset needs a kind, a name and the moment it answers — nothing else. */
function CreateForm({
  readOnly,
  onCreate,
}: {
  readOnly: boolean
  onCreate: (kind: TextKind, name: string, event: string) => void
}) {
  const [kind, setKind] = useState<TextKind>('bark')
  const [name, setName] = useState('')
  const [event, setEvent] = useState(templateOf('bark').event)
  return (
    <form
      className="ntl-create"
      onSubmit={(submitted) => {
        submitted.preventDefault()
        if (name.trim() && event.trim()) onCreate(kind, name, event)
      }}
    >
      <label>
        Template
        <select
          value={kind}
          onChange={(changed) => {
            const next = changed.target.value as TextKind
            setKind(next)
            // The suggested trigger follows the template, because the moment a
            // codex page answers is not the moment a bark answers, and an
            // inherited name would read as a considered choice.
            setEvent(templateOf(next).event)
          }}
        >
          {TEXT_TEMPLATES.map((template) => (
            <option key={template.kind} value={template.kind}>
              {template.label}
            </option>
          ))}
        </select>
      </label>
      <label>
        Name
        <input value={name} onChange={(changed) => setName(changed.target.value)} />
      </label>
      <label>
        Trigger
        <input value={event} onChange={(changed) => setEvent(changed.target.value)} />
      </label>
      <button type="submit" className="btn" disabled={readOnly || !name.trim() || !event.trim()}>
        New
      </button>
    </form>
  )
}

function EntryFields({
  entry,
  index,
  cast,
  participants,
  target,
  sequence,
  readOnly,
  nameOf,
  onChange,
  onRemove,
}: {
  entry: TextEntry
  index: number
  cast: boolean
  participants: NonNullable<TextAsset['participants']>
  target?: ReviewTarget
  sequence: boolean
  readOnly: boolean
  nameOf: (entity: string) => string | undefined
  onChange: (entry: TextEntry) => void
  onRemove: () => void
}) {
  const lines = entry.lines ?? []
  const schema = useNarrativeState()
  return (
    <li className="ntl-entry">
      <label>
        Entry {index + 1}
        <input
          value={entry.label ?? ''}
          disabled={readOnly}
          onChange={(changed) => onChange({ ...entry, label: changed.target.value })}
        />
      </label>
      <TypedCondition
        label={`Entry ${index + 1} condition`}
        value={entry.when}
        variables={schema.data?.document.variables ?? []}
        disabled={readOnly}
        onChange={(when) => onChange({ ...entry, when })}
      />
      {lines.map((slot, position) => (
        <TextLineFields
          key={slot.id}
          slot={slot}
          target={target?.slot === slot.id ? target : undefined}
          label={`Entry ${index + 1}, line ${position + 1}`}
          cast={cast}
          participants={participants}
          disabled={readOnly}
          nameOf={nameOf}
          onChange={(next) =>
            onChange({ ...entry, lines: lines.map((one) => (one.id === slot.id ? next : one)) })
          }
        />
      ))}
      <div className="ntl-entry-actions">
        {sequence && (
          <button
            type="button"
            className="btn"
            disabled={readOnly}
            onClick={() => onChange({ ...entry, lines: [...lines, blankLine(cast)] })}
          >
            Add line
          </button>
        )}
        <button type="button" className="btn" disabled={readOnly} onClick={onRemove}>
          Remove entry
        </button>
      </div>
    </li>
  )
}

/**
 * A new line, with the wording placeholder the writer is about to replace.
 *
 * The revision is deliberately empty: this side never invents one. The save
 * path re-seals every changed body through `narrative_text_written`, so the
 * document that reaches disk carries a revision the model derived.
 */
function blankLine(cast: boolean) {
  return {
    id: mintId(),
    speaker: (cast ? 'player' : 'narrator') as Speaker,
    variants: [{ id: mintId(), text: { revision: '', body: '' } }],
  }
}

function blankEntry(cast: boolean): TextEntry {
  return { id: mintId(), label: '', lines: [blankLine(cast)] }
}
