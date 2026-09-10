import { useNarrativeAffected } from '../../lib/queries'
import { NARRATIVE_UNAVAILABLE } from './narrativeModel'

/**
 * Why the selected line is out of date: source field → context → line (#168).
 *
 * Renders the backend's own explanations verbatim rather than deriving a
 * sentence from a code. The tracker is the only thing that knows *which* field
 * moved, and a second vocabulary for staleness invented here would be a second
 * thing to keep in step with a dependency model that will keep growing.
 *
 * Scoped to the slot rather than to a variant, because the freshness badge a
 * reader is looking at is on a variant but the question they are asking is
 * about the line — and a slot's variants share almost all of their
 * dependencies, so splitting the list would repeat the same row several times.
 */
export function WhyAffected({ slotId }: { slotId: string }) {
  const affected = useNarrativeAffected()
  const rows = (affected.data ?? [])
    .filter((line) => line.slot === slotId && line.kind === 'changed')
    .flatMap((line) => line.explanations)

  if (affected.isPending || rows.length === 0) return null
  return (
    <section className="nrt-context" aria-label="Why affected">
      <h3>Why affected</h3>
      <ul className="nrt-affected">
        {rows.map((row, index) => (
          <li key={`${row.source}/${row.context}/${index}`}>
            <code>{row.source}</code> <span aria-hidden="true">→</span>{' '}
            <span className="nrt-note">{row.context}</span>
            <p className="nrt-note">{row.message}</p>
          </li>
        ))}
      </ul>
      <p className="nrt-note">{NARRATIVE_UNAVAILABLE.affectedRebuild}</p>
    </section>
  )
}
