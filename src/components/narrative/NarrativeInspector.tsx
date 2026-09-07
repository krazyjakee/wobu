import { useUI } from '../../store/ui'
import { Icon } from '../Icon'
import { TipButton } from '../Tooltip'
import { NARRATIVE_UNAVAILABLE } from './narrativeModel'

/**
 * The context pane: what the selected element is allowed to know, and who is in
 * it.
 *
 * The inspector is a *writer* of the shared selection as well as a reader. Its
 * breadcrumb walks back up the path — pressing Scene from a selected line drops
 * the beat and the line — and because that goes through the same
 * `selectNarrative` every other surface uses, the canvas and Script follow it
 * without the inspector knowing they exist.
 *
 * Ids are shown rather than names on purpose. There is no narrative model in
 * this build to resolve a name from, and printing a plausible title here would
 * be inventing one.
 */
export function NarrativeInspector() {
  const selection = useUI((s) => s.narrative)
  const selectNarrative = useUI((s) => s.selectNarrative)
  const { sceneId, beatId, lineId } = selection

  return (
    <aside className="nrt-inspector" aria-label="Narrative context">
      <h2>Context</h2>

      {sceneId === null ? (
        <p className="nrt-note">
          Nothing is selected. Choosing a scene, a beat or a line anywhere in this workspace fills
          this pane with what that element is allowed to draw on.
        </p>
      ) : (
        <nav className="nrt-crumbs" aria-label="Selected element">
          <TipButton
            className={beatId === null ? 'chip is-on' : 'chip'}
            aria-current={beatId === null ? 'true' : undefined}
            tip="Select the scene, and nothing inside it"
            onClick={() => selectNarrative({ sceneId }, 'inspector')}
          >
            Scene <code>{sceneId}</code>
          </TipButton>
          {beatId !== null && (
            <TipButton
              className={lineId === null ? 'chip is-on' : 'chip'}
              aria-current={lineId === null ? 'true' : undefined}
              tip="Select the beat, and no line within it"
              onClick={() => selectNarrative({ sceneId, beatId }, 'inspector')}
            >
              Beat <code>{beatId}</code>
            </TipButton>
          )}
          {lineId !== null && (
            <span className="chip is-on" aria-current="true">
              Line <code>{lineId}</code>
            </span>
          )}
        </nav>
      )}

      <section className="nrt-context">
        <h3>What will appear here</h3>
        <p className="nrt-note">
          <Icon name="lock" size="sm" />
          {NARRATIVE_UNAVAILABLE.source}
        </p>
        {/* Named rather than left blank: a writer deciding whether this pane is
            worth opening should be able to see what it is *for* before there is
            a project that fills it. */}
        <ul>
          <li>Participants, and what each of them intends in this beat</li>
          <li>Known facts, and where each character learned them</li>
          <li>Beliefs and relationships, including the mistaken ones</li>
          <li>Recent events the scene may refer back to</li>
          <li>What this beat must convey, and what it must not reveal</li>
          <li>The selected variant, and why it was the one included</li>
        </ul>
        <TipButton
          className="btn"
          disabledReason={NARRATIVE_UNAVAILABLE.source}
          tip="Show the exact context and prompt a generation would be sent"
        >
          Inspect generation request
        </TipButton>
      </section>
    </aside>
  )
}
