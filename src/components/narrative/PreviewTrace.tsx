import { useEffect, useRef } from 'react'
import type { ExecutionTrace, PreviewTraceSite } from '../../lib/api/narrativePreview'

/**
 * The playback transcript: one row per recorded action, and what it observed.
 *
 * `focusStep` is the other half of the Flow overlay's cursor (#188). The two
 * panes are separate tabs, so a click on a highlighted node cannot call into
 * this list — it may not be mounted — and instead latches a step the pane hands
 * down here. The row is marked `aria-current` as well as scrolled, because a
 * list that only scrolled would leave a reader who was not watching with no
 * idea which row was asked for.
 */
export function PreviewTrace({
  entries,
  openSource,
  focusStep = null,
  onCentre,
}: {
  entries: { label: string; execution: ExecutionTrace }[]
  openSource: (site: PreviewTraceSite) => void
  /** The row the Flow canvas asked for, or null. */
  focusStep?: number | null
  /** Centre the Flow canvas on the box a record happened at. */
  onCentre?: (site: PreviewTraceSite) => void
}) {
  const rows = useRef<(HTMLLIElement | null)[]>([])
  const details = useRef<HTMLDetailsElement>(null)
  useEffect(() => {
    if (focusStep === null) return
    // Opened, then scrolled: a closed `details` has no laid-out row to reach.
    if (details.current) details.current.open = true
    rows.current[focusStep]?.scrollIntoView?.({ block: 'center' })
  }, [focusStep])
  return (
    <details ref={details}>
      <summary>Playback trace ({entries.length} steps, latest 100 retained)</summary>
      <ol className="nrt-preview-trace">
        {entries.map((entry, index) => (
          <li
            key={index}
            ref={(node) => {
              rows.current[index] = node
            }}
            aria-current={index === focusStep ? 'step' : undefined}
          >
            <strong>{entry.label}</strong>
            {!entry.execution.committed && <p>Rolled back: {entry.execution.error}</p>}
            {!!entry.execution.omitted && (
              <p>{entry.execution.omitted} additional records omitted.</p>
            )}
            <ol>
              {entry.execution.records.map(({ site, event }, recordIndex) => (
                <li key={recordIndex}>
                  <button className="btn" onClick={() => openSource(site)}>
                    Open{' '}
                    {site.choice
                      ? 'choice'
                      : site.outcome
                        ? 'outcome'
                        : site.slot
                          ? 'line'
                          : 'beat'}{' '}
                    in Script
                  </button>
                  {onCentre && (
                    <button className="btn" onClick={() => onCentre(site)}>
                      Centre in Flow
                    </button>
                  )}
                  {event.kind === 'condition' && (
                    <p>
                      Condition {event.passed ? 'passed' : 'failed'}
                      {event.path.length > 0 ? ` at expression ${event.path.join('.')}` : ''}
                    </p>
                  )}
                  {event.kind === 'transition' && <p>Took this authored route</p>}
                  {event.kind === 'effect' && (
                    <p>Effect {event.index + 1}: values before → after</p>
                  )}
                  {event.kind === 'command_result' && (
                    <p>Host result{event.repeated ? ' (already acknowledged)' : ''}</p>
                  )}
                  <pre>{JSON.stringify(event, null, 2)}</pre>
                </li>
              ))}
            </ol>
          </li>
        ))}
      </ol>
    </details>
  )
}
