import { useUI } from '../../store/ui'
import { NarrativePlaceholder } from './NarrativePlaceholder'
import { NARRATIVE_UNAVAILABLE } from './narrativeModel'

/**
 * Where the dialogue editor goes.
 *
 * The same seam as `NarrativeFlowPane`, from the other side: this pane reads
 * `narrative.beatId` to know which beat's lines to write, and calls
 * `selectNarrative({ sceneId, beatId, lineId }, 'script')` when the caret moves
 * to a different line — which is what makes clicking a line here highlight the
 * same beat on the canvas without either pane calling the other.
 *
 * Its selection is a *line* inside the beat it is already showing, so it passes
 * all three ids rather than the line alone.
 */
export function NarrativeScriptPane() {
  const beatId = useUI((s) => s.narrative.beatId)
  return (
    <div className="nrt-pane nrt-script">
      <NarrativePlaceholder title="No editor in this build" reason={NARRATIVE_UNAVAILABLE.script}>
        {beatId && (
          <p className="nrt-note">
            Selected beat <code>{beatId}</code>
          </p>
        )}
      </NarrativePlaceholder>
    </div>
  )
}
