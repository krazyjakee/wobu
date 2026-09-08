import { useMemo, useState } from 'react'
import { collapse, diffLines, hasChanges } from '../../../lib/diff'

/** Comparison only: the diff never merges or authorises a write. */
export function ReviewDiff({
  current,
  candidate,
  currentRevision,
  baseRevision,
  baseWording,
}: {
  current: string
  candidate: string
  currentRevision: string | null
  baseRevision: string | null
  baseWording?: string | null
}) {
  const [whole, setWhole] = useState(false)
  const rows = useMemo(() => diffLines(current, candidate), [current, candidate])
  const shown = whole ? rows : collapse(rows, 2)
  return (
    <section className="nrt-review-diff" aria-label="Wording comparison">
      <div className="nrt-review-revisions">
        <p>
          Current revision: <code>{currentRevision ?? 'No accepted wording'}</code>
        </p>
        <p>
          Proposal base revision: <code>{baseRevision ?? 'Originally empty slot'}</code>
        </p>
      </div>
      {baseRevision && currentRevision !== baseRevision && (
        <p className="nrt-review-notice">
          Current wording has changed since this proposal was requested.
        </p>
      )}
      {hasChanges(rows) ? (
        <>
          <button className="btn" aria-pressed={whole} onClick={() => setWhole((value) => !value)}>
            {whole ? 'Show changes only' : 'Show full wording'}
          </button>
          <table>
            <caption>Current wording compared with proposed wording</caption>
            <thead>
              <tr>
                <th scope="col">Change</th>
                <th scope="col">Current wording</th>
                <th scope="col">Proposed wording</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((row, index) =>
                row.kind === 'gap' ? (
                  <tr key={index}>
                    <td colSpan={3}>{row.count} unchanged lines</td>
                  </tr>
                ) : (
                  <tr key={index} className={`is-${row.kind}`}>
                    <th scope="row">
                      {row.kind === 'same'
                        ? 'Unchanged'
                        : row.kind === 'added'
                          ? 'Added'
                          : row.kind === 'removed'
                            ? 'Removed'
                            : 'Changed'}
                    </th>
                    <td>{row.left ?? '—'}</td>
                    <td>{row.right ?? '—'}</td>
                  </tr>
                ),
              )}
            </tbody>
          </table>
        </>
      ) : (
        <p>The current and proposed wording are identical.</p>
      )}
      {baseRevision && (
        <details>
          <summary>Wording at the proposal’s base revision</summary>
          {baseWording != null ? (
            <pre>{baseWording}</pre>
          ) : (
            <p>
              The base revision is retained, but its wording was not included in the frozen context.
            </p>
          )}
        </details>
      )}
    </section>
  )
}
