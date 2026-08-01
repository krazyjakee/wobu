import { useEffect, useMemo, useState } from 'react'
import type { SectionDef, SectionValue, WobuDescription } from '../../lib/api'
import type { EnhanceSession } from './useEnhanceSession'

type SectionMap = WobuDescription['sections']

export function EnhanceReview({
  current,
  definitions,
  session,
}: {
  current: WobuDescription | null
  definitions: SectionDef[]
  session: EnhanceSession
}) {
  const candidate = session.candidate
  const effectiveCurrent = session.refusedNode?.description ?? current
  const [accepted, setAccepted] = useState<Set<string>>(() => new Set())

  useEffect(() => {
    setAccepted(new Set())
  }, [candidate?.jobId])

  const sections = useMemo(
    () =>
      orderedSections(
        definitions,
        effectiveCurrent?.sections ?? {},
        candidate?.description.sections ?? {},
      ),
    [definitions, effectiveCurrent, candidate],
  )

  const choose = (key: string, useNew: boolean) => {
    setAccepted((currentAccepted) => {
      const next = new Set(currentAccepted)
      if (useNew) next.add(key)
      else next.delete(key)
      return next
    })
  }

  const acceptSelected = () => {
    if (!candidate || accepted.size === 0) return
    session.accept({
      sections: mergeSections(
        effectiveCurrent?.sections ?? {},
        candidate.description.sections,
        accepted,
      ),
    })
  }

  return (
    <div className="enhance-review" aria-label="Enhance review">
      <div className="enhance-review-head">
        <div>
          <h3>{session.complete ? 'Review enhanced canon' : 'Enhance draft'}</h3>
          <p>{reviewStatus(session, candidate !== null)}</p>
        </div>
        {session.running && (
          <button className="btn" type="button" onClick={session.stop}>
            Stop
          </button>
        )}
      </div>

      {candidate?.questions.length ? (
        <aside className="enhance-questions" aria-label="Questions from Enhance">
          <h3>Questions for you</h3>
          <p>The model left these open instead of inventing an answer.</p>
          <ul>
            {candidate.questions.map((question, index) => (
              <li key={`question-${index}`}>{question}</li>
            ))}
          </ul>
        </aside>
      ) : null}

      {session.refusedNode && (
        <div className="enhance-guard" role="alert">
          <b>The current description was hand-edited.</b>
          <span>
            It may have changed since Enhance started. The Current column now shows the guarded
            version from disk. Replace it only if the reviewed draft should win.
          </span>
          <button
            className="btn btn-ai"
            type="button"
            disabled={session.accepting}
            onClick={session.forceAccept}
          >
            Replace hand-edited description
          </button>
        </div>
      )}

      {!candidate ? (
        <div className="enhance-waiting">Waiting for the first structured section…</div>
      ) : (
        <div className="enhance-diffs">
          {sections.map((section) => {
            const before = effectiveCurrent?.sections[section.key]
            const after = candidate.description.sections[section.key]
            const changed = !sameValue(before, after)
            const useNew = accepted.has(section.key)
            return (
              <section
                className={`enhance-diff${changed ? ' is-changed' : ' is-unchanged'}`}
                key={section.key}
              >
                <div className="enhance-diff-title">
                  <h3>{section.label}</h3>
                  <span>{changed ? changeLabel(before, after) : 'unchanged'}</span>
                </div>
                <div className="enhance-diff-cols">
                  <div>
                    <b>Current</b>
                    <SectionPreview sectionKey={section.key} value={before} />
                  </div>
                  <div>
                    <b>New</b>
                    <SectionPreview sectionKey={section.key} value={after} />
                  </div>
                </div>
                {session.complete && changed && (
                  <div className="enhance-section-actions">
                    <button
                      type="button"
                      aria-pressed={!useNew}
                      disabled={session.accepting}
                      onClick={() => choose(section.key, false)}
                    >
                      Keep current
                    </button>
                    <button
                      type="button"
                      aria-pressed={useNew}
                      disabled={session.accepting}
                      onClick={() => choose(section.key, true)}
                    >
                      Use new
                    </button>
                  </div>
                )}
              </section>
            )
          })}
        </div>
      )}

      {candidate && (
        <div className="enhance-review-actions">
          {session.complete ? (
            <>
              <button
                className="btn"
                type="button"
                disabled={session.accepting || session.discarding}
                onClick={session.reject}
              >
                Reject
              </button>
              <button
                className="btn"
                type="button"
                disabled={accepted.size === 0 || session.accepting || session.discarding}
                onClick={acceptSelected}
              >
                Accept selected ({accepted.size})
              </button>
              <button
                className="btn btn-ai"
                type="button"
                disabled={session.accepting || session.discarding}
                onClick={() => session.accept(candidate.description)}
              >
                Accept all
              </button>
            </>
          ) : !session.running ? (
            <button className="btn" type="button" onClick={session.reject}>
              Dismiss local draft
            </button>
          ) : null}
        </div>
      )}
      {!candidate && !session.running && !session.starting && (
        <div className="enhance-review-actions">
          <button className="btn" type="button" onClick={session.reject}>
            Dismiss
          </button>
        </div>
      )}
    </div>
  )
}

function SectionPreview({
  sectionKey,
  value,
}: {
  sectionKey: string
  value: SectionValue | undefined
}) {
  if (!value || (value.type === 'text' && value.value.trim() === '')) {
    return <span className="enhance-empty">Not present</span>
  }
  if (value.type === 'text') return <p>{value.value}</p>
  if (sectionKey === 'palette') {
    return (
      <div className="enhance-palette">
        {value.value.map((colour, index) => (
          <span key={`${colour}-${index}`} style={{ backgroundColor: colour }} title={colour} />
        ))}
      </div>
    )
  }
  if (value.value.length === 0) return <span className="enhance-empty">Not present</span>
  return (
    <ul>
      {value.value.map((item, index) => (
        <li key={`${item}-${index}`}>{item}</li>
      ))}
    </ul>
  )
}

function orderedSections(
  definitions: SectionDef[],
  current: SectionMap,
  proposed: SectionMap,
): SectionDef[] {
  const ordered = [...definitions]
  const seen = new Set(ordered.map((section) => section.key))
  for (const [key, value] of [...Object.entries(current), ...Object.entries(proposed)]) {
    if (seen.has(key)) continue
    seen.add(key)
    ordered.push({ key, label: key.replace(/_/g, ' '), valueKind: value.type })
  }
  return ordered
}

function mergeSections(
  current: SectionMap,
  proposed: SectionMap,
  accepted: Set<string>,
): SectionMap {
  const merged = { ...current }
  for (const key of accepted) {
    const value = proposed[key]
    if (value) merged[key] = value
    else delete merged[key]
  }
  return merged
}

function sameValue(left: SectionValue | undefined, right: SectionValue | undefined): boolean {
  if (!left || !right) return left === right
  if (left.type !== right.type) return false
  if (left.type === 'text' && right.type === 'text') return left.value === right.value
  if (left.type === 'list' && right.type === 'list') {
    return (
      left.value.length === right.value.length &&
      left.value.every((item, index) => item === right.value[index])
    )
  }
  return false
}

function changeLabel(before: SectionValue | undefined, after: SectionValue | undefined): string {
  if (!before) return 'added'
  if (!after) return 'removed'
  return 'changed'
}

function reviewStatus(session: EnhanceSession, hasDraft: boolean): string {
  if (session.complete) return 'Nothing is written until you accept this review.'
  if (session.stopped) return 'Stopped. The partial draft stays here locally and is not saved.'
  if (session.failure) {
    return hasDraft
      ? `${session.failure.message} The partial draft remains local and unsaved.`
      : session.failure.message
  }
  return 'Streaming structured sections. The current canon remains untouched.'
}
