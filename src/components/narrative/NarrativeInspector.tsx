import type { Speaker } from '../../lib/api'
import { errorMessage } from '../../lib/api'
import { useScene } from '../../lib/queries'
import { useUI } from '../../store/ui'
import { TipButton } from '../Tooltip'
import { conditionText } from './flow/source'
import { useNarrativeNames } from './flow/useNarrativeNames'
import { NarrativeContext } from './NarrativeContext'
import './narrativeInspector.css'
import { sceneEditKey, useScriptDrafts } from './scriptDrafts'
import { SceneOrganization } from './SceneOrganization'

/** The current scene draft, shared by Flow, Script and outline selection. */
export function NarrativeInspector({
  projectKey = '',
  readOnly = false,
}: {
  projectKey?: string
  readOnly?: boolean
}) {
  const { sceneId, beatId, lineId } = useUI((s) => s.narrative)
  const selectNarrative = useUI((s) => s.selectNarrative)
  const query = useScene(sceneId)
  const { nameOf } = useNarrativeNames()
  const draft = useScriptDrafts((state) =>
    sceneId ? state.drafts[sceneEditKey(projectKey, sceneId)] : undefined,
  )
  const scene = draft?.scene ?? query.data?.scene
  const beat = scene?.beats?.find((item) => item.id === beatId)
  const slot = beat?.dialogue?.find((item) => item.id === lineId)
  const speakerName = (speaker: Speaker) =>
    typeof speaker === 'string'
      ? speaker === 'player'
        ? 'Player'
        : 'Narrator'
      : (nameOf(speaker.entity) ?? speaker.entity)

  return (
    <aside className="nrt-inspector" aria-label="Narrative context">
      <h2>Context</h2>
      {sceneId === null ? (
        <p className="nrt-note">Choose a scene or beat to inspect its authored context.</p>
      ) : (
        <>
          <nav className="nrt-crumbs" aria-label="Selected element">
            <TipButton
              className={beatId === null ? 'chip is-on' : 'chip'}
              aria-current={beatId === null ? 'true' : undefined}
              tip="Select the scene, and nothing inside it"
              onClick={() => selectNarrative({ sceneId }, 'inspector')}
            >
              Scene {scene?.name ?? sceneId}
            </TipButton>
            {beatId !== null && (
              <TipButton
                className={lineId === null ? 'chip is-on' : 'chip'}
                aria-current={lineId === null ? 'true' : undefined}
                tip="Select the beat, and no line within it"
                onClick={() => selectNarrative({ sceneId, beatId }, 'inspector')}
              >
                Beat {beat?.title ?? beatId}
              </TipButton>
            )}
            {lineId !== null && <span className="chip is-on">Line {lineId}</span>}
          </nav>
          {query.isPending && <p role="status">Loading scene context…</p>}
          {query.isError && <p role="alert">{errorMessage(query.error)}</p>}
        </>
      )}
      {scene && (
        <>
          <p className="nrt-note">{draft ? 'Unsaved scene draft.' : 'Saved scene context.'}</p>
          {query.data && (
            <SceneOrganization file={query.data} projectKey={projectKey} readOnly={readOnly} />
          )}
          {scene.summary && <p>{scene.summary}</p>}
          <section className="nrt-context">
            <h3>Participants</h3>
            {scene.participants?.length ? (
              <ul>
                {scene.participants.map((participant) => (
                  <li key={participant.entity}>
                    {nameOf(participant.entity) ?? participant.entity}
                    {participant.role ? ` · ${participant.role}` : ''}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="nrt-note">No participants assigned.</p>
            )}
          </section>
          {beatId && !beat && <p role="status">The selected beat is no longer in this scene.</p>}
          {beat && (
            <>
              <section className="nrt-context">
                <h3>Intent</h3>
                {beat.intents?.length ? (
                  <ul>
                    {beat.intents.map((intent, index) => (
                      <li key={index}>
                        <strong>{speakerName(intent.subject)}</strong>: {intent.intent}
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="nrt-note">No intent authored.</p>
                )}
              </section>
              <ContextList title="Must convey" items={beat.must_convey} />
              <ContextList title="Must not reveal" items={beat.must_not_reveal} />
            </>
          )}
          {slot && (
            <section className="nrt-context">
              <h3>{speakerName(slot.speaker)} · Dialogue</h3>
              <p className="nrt-note">Slot policy: {slot.policy ?? 'edited'}</p>
              {slot.variants?.length ? (
                slot.variants.map((variant) => (
                  <article className="nrt-context-variant" key={variant.id}>
                    <p className="nrt-note">{conditionText(variant.when)}</p>
                    <p className="nrt-context-text">{variant.text.body || 'Empty wording'}</p>
                    <p className="nrt-note">
                      {variant.text.lifecycle?.policy ?? 'edited'} ·{' '}
                      {variant.text.lifecycle?.review ?? 'draft'} ·{' '}
                      {variant.text.lifecycle?.freshness === 'out_of_date'
                        ? 'out of date'
                        : 'current'}
                    </p>
                  </article>
                ))
              ) : (
                <p className="nrt-note">Missing text</p>
              )}
            </section>
          )}
        </>
      )}
      {sceneId && beatId && lineId && slot && !draft ? (
        <NarrativeContext
          key={`${sceneId}/${beatId}/${lineId}`}
          selection={{ scene: sceneId, beat: beatId, slot: lineId }}
          slot={slot}
        />
      ) : (
        <p className="nrt-note">
          {draft
            ? 'Save the scene draft to resolve attributed generation context.'
            : 'Select a dialogue line to resolve attributed generation context.'}
        </p>
      )}
    </aside>
  )
}

function ContextList({ title, items }: { title: string; items?: string[] }) {
  if (!items?.length) return null
  return (
    <section className="nrt-context">
      <h3>{title}</h3>
      <ul>
        {items.map((item, index) => (
          <li key={index}>{item}</li>
        ))}
      </ul>
    </section>
  )
}
