import type {
  MediaRow,
  MediaBinding,
  MediaNotes,
  MediaPolicy,
} from '../../../lib/api/narrativeMedia'
import { useMediaNotes, type NotesDraft } from './notesDrafts'
import { localeDirection } from '../../../lib/api/narrativeLocale'
export function MediaLine({
  row,
  draftKey,
  policy,
  guard,
  binding,
  status,
  disabled,
  onNotes,
  onAudition,
}: {
  row: MediaRow
  draftKey: string
  policy: MediaPolicy
  guard: string
  binding: MediaBinding | undefined
  status: string
  disabled: boolean
  onNotes: (draft: NotesDraft) => void
  onAudition: (history: number) => void
}) {
  const draft = useMediaNotes((state) => state.entries[draftKey])
  const notes = draft?.notes ?? row.notes
  const setNotes = (notes: MediaNotes) =>
    useMediaNotes
      .getState()
      .update(draftKey, { notes, policy: draft?.policy ?? policy, guard: draft?.guard ?? guard })
  return (
    <article className="nrt-media-line">
      <p>
        {row.source.speaker} · {row.source.context} · {row.key.locale}/{row.key.form}
      </p>
      <p dir={localeDirection(row.key.locale)}>{row.text || 'No selected translation'}</p>
      <code>{row.key.id}</code>
      <p>
        {status.replaceAll('_', ' ')}
        {!row.ready ? ' · source or translation not ready' : ''}
      </p>
      <details>
        <summary>Pronunciation and delivery notes</summary>
        <label>
          Pronunciation
          <textarea
            value={notes.pronunciation}
            onChange={(e) => setNotes({ ...notes, pronunciation: e.target.value })}
            disabled={disabled}
          />
        </label>
        <label>
          Delivery
          <textarea
            value={notes.delivery}
            onChange={(e) => setNotes({ ...notes, delivery: e.target.value })}
            disabled={disabled}
          />
        </label>
        <button
          className="btn"
          disabled={disabled || JSON.stringify(notes) === JSON.stringify(row.notes)}
          onClick={() => draft && onNotes(draft)}
        >
          Save recording notes
        </button>
        <p>
          Changing notes makes earlier takes out of date. Unsaved notes retain their original save
          guard across navigation.
        </p>
        {draft && (
          <button
            className="btn"
            disabled={disabled}
            onClick={() => useMediaNotes.getState().update(draftKey, null)}
          >
            Discard note draft
          </button>
        )}
      </details>
      <details>
        <summary>{binding?.history.length ?? 0} immutable takes</summary>
        {binding?.history.map((take, i) => (
          <div key={i}>
            <p>
              Take {i + 1}: {take.spoken_text} · {take.info.duration_ms} ms · source{' '}
              {take.row.source.revision}
            </p>
            <p>
              Audio hash: <code>{take.audio.hash}</code>
              {take.timing ? ' · timing attached' : ''}
            </p>
            <button className="btn" onClick={() => onAudition(i)}>
              Audition take {i + 1}
            </button>
          </div>
        ))}
      </details>
      {binding && (
        <button className="btn" onClick={() => onAudition(binding.history.length - 1)}>
          Audition latest take
        </button>
      )}
    </article>
  )
}
