import type { ContextFragment } from '../../lib/api/narrativeContext'
import type { Fact, KnowledgeClaim, Relationship, WorldEvent } from '../../lib/api/narrativeWorld'
import { useUI } from '../../store/ui'

/** Human-facing account first; exact attributed source data remains available below it. */
export function ContextFragmentView({
  fragment,
  nameOf,
}: {
  fragment: ContextFragment
  nameOf: (id: string) => string
}) {
  const { kind, data, source, required } = fragment
  let content
  if (kind === 'knowledge') {
    const { claim, canonical_fact: fact } = data as { claim: KnowledgeClaim; canonical_fact: Fact }
    const provenance = claim.provenance
    const origin =
      typeof provenance === 'string'
        ? 'Witnessed directly'
        : 'told' in provenance
          ? `Told by ${nameOf(provenance.told.by)}`
          : 'rumour' in provenance
            ? `Rumour: ${provenance.rumour.source}`
            : `Inferred: ${provenance.inferred.reason}`
    content = (
      <>
        <strong>{fact.name}</strong>
        <p>{fact.assertion}</p>
        <p>
          {claim.belief === 'true'
            ? 'Believes this is true'
            : claim.belief === 'false'
              ? 'Believes this is false'
              : 'Does not know whether this is true'}{' '}
          · {origin}
        </p>
        {!!fact.sources.length && (
          <p className="nrt-note">Canonical sources: {fact.sources.join(', ')}</p>
        )}
      </>
    )
  } else if (kind === 'voice') {
    const character = data as { id: string; name: string; voice: string | null }
    content = (
      <>
        <strong>{character.name}</strong>
        <p>{character.voice || 'No narrative voice authored.'}</p>
        <button
          className="chip"
          onClick={() => {
            useUI.getState().select(character.id)
            useUI.getState().setMode('library')
          }}
        >
          Edit character voice
        </button>
      </>
    )
  } else if (kind === 'relationship') {
    const relationship = data as Relationship
    content = (
      <p>
        {nameOf(relationship.from)} → {nameOf(relationship.to)} · {relationship.kind}:{' '}
        {String(relationship.value)}
      </p>
    )
  } else if (kind === 'event') {
    const event = data as WorldEvent
    content = (
      <>
        <strong>{event.name}</strong>
        <p>{event.summary}</p>
      </>
    )
  } else if (kind === 'future_restriction') {
    const restriction = data as { restriction: { name: string }; forbidden_fact: Fact | null }
    content = (
      <>
        <strong>{restriction.restriction.name}</strong>
        <p>Must not reveal: {restriction.forbidden_fact?.assertion ?? 'Missing fact source'}</p>
      </>
    )
  } else if (kind === 'required_meaning' || kind === 'forbidden_revelations') {
    const items = data as string[]
    content = items.length ? (
      <ul>
        {items.map((item, index) => (
          <li key={index}>{item}</li>
        ))}
      </ul>
    ) : (
      <p className="nrt-note">None authored.</p>
    )
  }
  return (
    <article className="nrt-context-fragment">
      <h4>
        {kind.replaceAll('_', ' ')} {required ? '· required' : ''}
      </h4>
      {content}
      <code>{source}</code>
      <details>
        <summary>Inspect source data</summary>
        <pre>{JSON.stringify(data, null, 2)}</pre>
      </details>
    </article>
  )
}
