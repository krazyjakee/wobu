import { useMemo, type CSSProperties } from 'react'
import type { ProjectSummary } from '../../lib/api'
import { useCreateScene, useSceneDiagnostics, useSceneFiles, useScenes } from '../../lib/queries'
import { useUI } from '../../store/ui'
import { Icon } from '../Icon'
import { TipButton, Tooltip } from '../Tooltip'
import { NarrativeCentre } from './NarrativeCentre'
import { NarrativeInspector } from './NarrativeInspector'
import { NarrativeLibrary } from './NarrativeLibrary'
import { sceneNode } from './flow/arc/model'
import { sceneToFlow, summaryStatus } from './flow/source'
import {
  NARRATIVE_UNAVAILABLE,
  NO_NARRATIVE_SOURCE,
  type NarrativeListState,
  type NarrativeSectionId,
} from './narrativeModel'

/**
 * The Narrative workspace: Library, the centre views, and the context pane.
 *
 * Laid out inside one grid cell rather than as three siblings of the mode rail,
 * so the Library's column widths and collapse rules stay where they are. What
 * it does share is the store: the same `[` and `]` that hide the navigator and
 * the inspector in the Library hide them here, and the width the reader dragged
 * there is the width they get here. Those are facts about this person's screen
 * — machine-local, like every other layout preference in `store/ui.ts`, and
 * never written into the project folder where they would follow the world to
 * somebody else's desk.
 *
 * ── what fetches, and what still cannot ─────────────────────────────────────
 *
 * Scenes are real: the Scenes list is the project's own folder, read through
 * `useScenes`, and "Create first scene" writes one. The other three sections
 * are still refused with a reason, because the models behind them — facts and
 * knowledge (#155), quests, the text library (#167) — do not exist. An empty
 * list and "this build cannot answer" are different claims, and a workspace
 * that showed the second as the first would tell a writer their project was
 * empty when it is not.
 */
export function NarrativeMode({ project }: { project: ProjectSummary }) {
  const navWidth = useUI((s) => s.navWidth)
  const navCollapsed = useUI((s) => s.navCollapsed)
  const inspCollapsed = useUI((s) => s.inspCollapsed)
  const selectNarrative = useUI((s) => s.selectNarrative)
  const catalog = useScenes()
  const ids = useMemo(() => (catalog.data?.scenes ?? []).map((one) => one.id), [catalog.data])
  const files = useSceneFiles(ids)
  const createScene = useCreateScene()

  /*
   * The Scenes section, with each scene's outstanding work.
   *
   * The status is the same lossy summary the canvas's chips use, and for the
   * same reason: a row is one badge wide. The real per-dimension counts are on
   * the arc's scene nodes and on every beat, where there is room to show all
   * four without one hiding the others.
   */
  const sections: Record<NarrativeSectionId, NarrativeListState> = {
    ...NO_NARRATIVE_SOURCE,
    scenes: catalog.isPending
      ? { kind: 'loading' }
      : catalog.isError
        ? { kind: 'error', message: String(catalog.error) }
        : {
            kind: 'ready',
            items: (catalog.data?.scenes ?? []).map((summary, index) => {
              const file = files[index]?.data
              const counts = file ? sceneNode(sceneToFlow(file.scene)).counts : null
              return {
                id: summary.id,
                name: summary.name,
                // Left off until the document has actually been read: a row
                // that said "Ready" while its file was still loading would be
                // asserting something nobody has looked at.
                status: counts ? summaryStatus(counts) : undefined,
              }
            }),
          },
  }

  const style: CSSProperties = {
    gridTemplateColumns: [
      navCollapsed ? null : `${navWidth}px`,
      'minmax(0, 1fr)',
      inspCollapsed ? null : 'var(--insp)',
    ]
      .filter(Boolean)
      .join(' '),
  }

  return (
    <div className="narrative-mode" style={style}>
      <header className="nrt-head">
        <h1>
          {project.name} <span aria-hidden>/</span> <b>Narrative</b>
        </h1>

        {/* `readOnly` and `aria-disabled` rather than `disabled`, the same
            choice `Combobox` makes: a disabled field cannot be focused, so it
            cannot explain itself to a keyboard, and "why is the search box
            dead" is the question this build most needs to answer. */}
        <Tooltip tip={NARRATIVE_UNAVAILABLE.search} placement="bottom">
          <input
            className="nrt-search"
            type="search"
            readOnly
            aria-disabled="true"
            aria-label="Find a scene or a line"
            placeholder="Find scene or line…"
          />
        </Tooltip>

        <div className="nrt-head-actions">
          <TipButton
            className="btn"
            disabledReason={NARRATIVE_UNAVAILABLE.review}
            tip="Compare drafts against the text they would replace"
          >
            Review
          </TipButton>
          <TipButton
            className="btn"
            disabledReason={NARRATIVE_UNAVAILABLE.build}
            tip="See which content a change affected before anything runs"
          >
            Build…
          </TipButton>
          <TipButton
            className="btn"
            disabledReason={NARRATIVE_UNAVAILABLE.export}
            tip="Package the compiled story for a game engine"
          >
            Export…
          </TipButton>
        </div>
      </header>

      {!navCollapsed && (
        <NarrativeLibrary
          sections={sections}
          readOnly={project.readOnly}
          onCreateScene={
            project.readOnly
              ? undefined
              : () => {
                  createScene.mutate('New scene', {
                    onSuccess: (file) => selectNarrative({ sceneId: file.scene.id }, 'library'),
                  })
                }
          }
          sectionNotes={{ scenes: <UnreadableScenes catalog={catalog.data} /> }}
        />
      )}
      <NarrativeCentre readOnly={project.readOnly} />
      {!inspCollapsed && <NarrativeInspector />}

      <SceneDiagnosticsFooter />
    </div>
  )
}

/**
 * The status line: what is wrong with the scene in front of the writer.
 *
 * Scoped to the selection rather than to the project, and that is a limit worth
 * being explicit about. `narrative_diagnostics` answers about one scene, so a
 * project-wide count would mean a command per scene on every render — and a
 * project-wide *list* is the Validation view (#171), which is not built. The
 * sentence beside the count says what these diagnostics are and, more usefully,
 * what they are not.
 */
function SceneDiagnosticsFooter() {
  const sceneId = useUI((s) => s.narrative.sceneId)
  const found = useSceneDiagnostics(sceneId)
  const count = found.data?.length ?? 0
  return (
    <footer className="nrt-foot" aria-label="Narrative diagnostics">
      <Icon name={count > 0 ? 'x' : 'check'} size="sm" />
      {sceneId === null
        ? 'Choose a scene to see what is wrong with it. '
        : found.isPending
          ? 'Reading this scene’s diagnostics… '
          : found.isError
            ? `Could not read this scene’s diagnostics: ${String(found.error)}. `
            : `${count} problem${count === 1 ? '' : 's'} in this scene. `}
      {NARRATIVE_UNAVAILABLE.diagnostics}
    </footer>
  )
}

/**
 * Files in the scenes folder that could not be identified.
 *
 * Said rather than dropped. A scene a sync client copied half-written has no
 * readable id, so it cannot be a row in the list — and a list that quietly
 * omitted it would present somebody's file as deleted, which is the one wrong
 * answer here.
 */
function UnreadableScenes({
  catalog,
}: {
  catalog?: { unreadable: { rel: string; reason: string }[] }
}) {
  const bad = catalog?.unreadable ?? []
  if (bad.length === 0) return null
  return (
    <ul className="nrt-arc-diagnostics" aria-label="Scene files that could not be read">
      {bad.map((one) => (
        <li key={one.rel} className="is-bad">
          <Icon name="x" size="sm" />
          <span>
            <code>{one.rel}</code> could not be read: {one.reason}. It is still on disk.
          </span>
        </li>
      ))}
    </ul>
  )
}
