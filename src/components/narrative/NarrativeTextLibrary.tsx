import { useState } from 'react'
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

/**
 * The Text library: authoring the six supporting kinds (#167).
 *
 * Deliberately one pane and one form rather than a second workspace. A bark has
 * no canvas, no branches and no cursor, so the Flow/Script/Preview split that a
 * scene needs would be three tabs where two of them have nothing to show. What
 * it does share with a scene is the part that matters: the same whole-document
 * guarded save, the same wording identities, the same diagnostics rendered from
 * the backend's own wording, and the same refusal to invent a revision on this
 * side.
 *
 * What this surface does *not* do yet, stated plainly so it is not mistaken for
 * a gap nobody noticed: it has no search or paging (the assets are not in the
 * SQLite projection), no per-line review controls (the review queue is
 * scene-keyed), and no Generate action (the frozen generation request names a
 * scene slot). The model, compiler, runtime and export all handle supporting
 * text; those three surfaces are the remaining wiring.
 */
export function NarrativeTextLibrary(props: {
  projectKey: string
  readOnly: boolean
  /** Resolves a world entity id to a name, or `undefined` when nothing can. */
  nameOf: (entity: string) => string | undefined
  onClose: () => void
}) {
  return <TextLibrary key={props.projectKey} {...props} />
}

function TextLibrary({
  projectKey,
  readOnly,
  nameOf,
  onClose,
}: {
  projectKey: string
  readOnly: boolean
  nameOf: (entity: string) => string | undefined
  onClose: () => void
}) {
  const client = useQueryClient()
  const catalog = useNarrativeTexts(projectKey)
  const [selected, setSelected] = useState<TextAssetId | null>(null)
  const file = useNarrativeText(projectKey, selected)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const loaded = file.data

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
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="ntl-view" aria-label="Text library">
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
          <ul>
            {(catalog.data?.assets ?? []).map((asset) => (
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
              // Remounted per document revision rather than synchronised by an
              // effect: the draft is initialised from the file it belongs to, so
              // a save that returns re-sealed wording replaces the form instead
              // of leaving the editor holding pre-seal text.
              key={`${loaded.rel}:${loaded.stamp?.hash ?? 'new'}`}
              projectKey={projectKey}
              file={loaded}
              readOnly={readOnly}
              busy={busy}
              nameOf={nameOf}
              onSave={(asset) =>
                run(async () =>
                  narrativeTextSave(
                    await prepareTextAsset(loaded.asset, asset),
                    preconditionOf(loaded.stamp),
                  ),
                )
              }
              onDelete={() =>
                run(async () => {
                  await narrativeTextDelete(loaded.asset.id)
                  setSelected(null)
                })
              }
            />
          ) : (
            <p className="ntl-empty">Select an asset to edit it.</p>
          )}
        </main>
      </div>
    </div>
  )
}

/**
 * The form for one asset.
 *
 * Its own component so the draft can be initialised from the file it belongs to
 * and thrown away when a different revision arrives, which is what the `key` on
 * the call site is for. A single shared draft synchronised by an effect would
 * have to answer "what happens to unsaved edits when the file changes
 * underneath", and every answer to that is worse than starting again from the
 * document that is actually on disk.
 */
function AssetEditor({
  projectKey,
  file,
  readOnly,
  busy,
  nameOf,
  onSave,
  onDelete,
}: {
  projectKey: string
  file: TextFile
  readOnly: boolean
  busy: boolean
  nameOf: (entity: string) => string | undefined
  onSave: (asset: TextAsset) => void
  onDelete: () => void
}) {
  const [draft, setDraft] = useState<TextAsset>(() => structuredClone(file.asset))
  const diagnostics = useNarrativeTextDiagnostics(projectKey, file.asset.id, draft)
  const template = templateOf(draft.kind)
  const dirty = JSON.stringify(draft) !== JSON.stringify(file.asset)
  return (
    <>
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

      <p className="ntl-hint">{template.hint}</p>

      <ol className="ntl-entries">
        {(draft.entries ?? []).map((entry, index) => (
          <EntryFields
            key={entry.id}
            entry={entry}
            index={index}
            cast={template.cast}
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
          onClick={() => onSave(draft)}
        >
          Save
        </button>
        <button type="button" className="btn" disabled={readOnly || busy} onClick={onDelete}>
          Delete
        </button>
      </div>

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
  sequence,
  readOnly,
  nameOf,
  onChange,
  onRemove,
}: {
  entry: TextEntry
  index: number
  cast: boolean
  sequence: boolean
  readOnly: boolean
  nameOf: (entity: string) => string | undefined
  onChange: (entry: TextEntry) => void
  onRemove: () => void
}) {
  const lines = entry.lines ?? []
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
      {lines.map((slot, position) => (
        <div key={slot.id} className="ntl-line">
          <span className="ntl-speaker">{speakerLabel(slot.speaker, nameOf)}</span>
          <textarea
            rows={2}
            aria-label={`Entry ${index + 1}, line ${position + 1}`}
            value={slot.variants?.[0]?.text.body ?? ''}
            disabled={readOnly || slot.policy === 'locked'}
            onChange={(changed) =>
              onChange({
                ...entry,
                lines: lines.map((one) =>
                  one.id === slot.id
                    ? {
                        ...one,
                        variants: (one.variants ?? []).map((variant, variantIndex) =>
                          variantIndex === 0
                            ? {
                                ...variant,
                                text: { ...variant.text, body: changed.target.value },
                              }
                            : variant,
                        ),
                      }
                    : one,
                ),
              })
            }
          />
        </div>
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
 * The speaker, named where a name exists.
 *
 * An id that nothing can name is left showing rather than given a plausible
 * one, matching `useNarrativeNames`: a made-up name is indistinguishable from a
 * real one and wrong.
 */
function speakerLabel(speaker: Speaker, nameOf: (entity: string) => string | undefined): string {
  if (speaker === 'narrator') return 'Narrator'
  if (speaker === 'player') return 'Player'
  return nameOf(speaker.entity) ?? speaker.entity
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
