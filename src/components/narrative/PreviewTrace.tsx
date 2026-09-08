import type { ExecutionTrace, PreviewTraceSite } from '../../lib/api/narrativePreview'

export function PreviewTrace({
  entries,
  openSource,
}: {
  entries: { label: string; execution: ExecutionTrace }[]
  openSource: (site: PreviewTraceSite) => void
}) {
  return (
    <details>
      <summary>Playback trace ({entries.length} steps, latest 100 retained)</summary>
      <ol className="nrt-preview-trace">
        {entries.map((entry, index) => (
          <li key={index}>
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
