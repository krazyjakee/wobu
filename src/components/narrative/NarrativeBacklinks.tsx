import './narrativeBacklinks.css'
import { useEffect, useRef, useState } from 'react'
import { errorMessage } from '../../lib/api'
import { useNarrativeWorld } from '../../lib/queries/narrativeWorld'
import { useUI } from '../../store/ui'
import { narrativeBacklinks } from './narrativeBacklinkModel'

export function NarrativeBacklinks({
  entityId,
  character,
  projectKey,
}: {
  entityId: string
  character: boolean
  projectKey: string
}) {
  const world = useNarrativeWorld()
  const summary = useRef<HTMLElement>(null)
  const reveal = useUI((state) => state.narrativeEntityTarget)
  useEffect(() => {
    if (reveal?.projectKey !== projectKey || reveal.entityId !== entityId) return
    summary.current?.focus()
    if (document.activeElement === summary.current)
      useUI.getState().finishNarrativeEntityReveal(reveal.seq)
  }, [entityId, projectKey, reveal])
  const [page, setPage] = useState(0)
  const links = world.data ? narrativeBacklinks(world.data.document, entityId, character) : []
  const current = Math.min(page, Math.max(0, Math.ceil(links.length / 50) - 1))
  return (
    <details className="nrt-entity-backlinks">
      <summary ref={summary}>Narrative records{world.data ? ` (${links.length})` : ''}</summary>
      {world.isPending && <p role="status">Loading narrative links…</p>}
      {world.isError && <p role="alert">{errorMessage(world.error)}</p>}
      {world.data && !links.length && <p>No narrative records refer to this entity.</p>}
      <ul>
        {links.slice(current * 50, current * 50 + 50).map(({ collection, category, record }) => (
          <li key={record.id}>
            <button
              className="btn"
              onClick={() =>
                useUI.getState().openNarrativeWorld({ projectKey, collection, recordId: record.id })
              }
            >
              {record.name} · {category}
            </button>
          </li>
        ))}
      </ul>
      {links.length > 50 && (
        <nav aria-label="Narrative link pages">
          <button className="btn" disabled={!current} onClick={() => setPage(current - 1)}>
            Previous links
          </button>
          <button
            className="btn"
            disabled={(current + 1) * 50 >= links.length}
            onClick={() => setPage(current + 1)}
          >
            Next links
          </button>
        </nav>
      )}
    </details>
  )
}
