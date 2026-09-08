import { useMemo, useState, type CSSProperties } from 'react'
import type { ProjectSummary } from '../../lib/api'
import { useCreateScene, useSceneDiagnostics, useSceneFiles, useScenes } from '../../lib/queries'
import { useUI, type NarrativeTarget } from '../../store/ui'
import { Icon } from '../Icon'
import { TipButton } from '../Tooltip'
import { NarrativeCentre } from './NarrativeCentre'
import { NarrativeInspector } from './NarrativeInspector'
import { NarrativeWorldPane } from './NarrativeWorldPane'
import { useNarrativeWorld } from '../../lib/queries/narrativeWorld'
import { NarrativeLibrary } from './NarrativeLibrary'
import { useNarrativeNames } from './flow/useNarrativeNames'
import { NARRATIVE_UNAVAILABLE } from './narrativeModel'
import { useSceneLibrary } from './sceneLibraryStore'

export function NarrativeMode({ project }: { project: ProjectSummary }) {
  return <NarrativeWorkspace key={project.path} project={project} />
}

function NarrativeWorkspace({ project }: { project: ProjectSummary }) {
  const navWidth = useUI((s) => s.navWidth)
  const navCollapsed = useUI((s) => s.navCollapsed)
  const inspCollapsed = useUI((s) => s.inspCollapsed)
  const selection = useUI((s) => s.narrative)
  const selectNarrative = useUI((s) => s.selectNarrative)
  const setTab = useUI((s) => s.setNarrativeTab)
  const [libraryOpen, setLibraryOpen] = useState(true)
  const [worldOpen, setWorldOpen] = useState(false)
  const [editorOpened, setEditorOpened] = useState(false)
  const world = useNarrativeWorld()
  const catalog = useScenes()
  const ids = useMemo(() => (catalog.data?.scenes ?? []).map((one) => one.id), [catalog.data])
  const files = useSceneFiles(ids)
  const createScene = useCreateScene()
  const { nameOf } = useNarrativeNames()
  const rows = (catalog.data?.scenes ?? []).map((summary, index) => ({
    summary,
    scene: files[index]?.data?.scene,
    error: files[index]?.isError ? String(files[index]?.error) : undefined,
  }))
  const selected = rows.find((row) => row.summary.id === selection.sceneId)?.scene
  const open = (target: NarrativeTarget, tab: 'flow' | 'script', variantId?: string) => {
    useSceneLibrary.setState({
      searchVariant:
        target.sceneId && target.lineId && variantId
          ? { sceneId: target.sceneId, slotId: target.lineId, variantId }
          : null,
    })
    selectNarrative(target, 'library')
    setTab(tab)
    setEditorOpened(true)
    setLibraryOpen(false)
    setWorldOpen(false)
  }
  const editorStyle: CSSProperties = {
    gridTemplateColumns: [
      navCollapsed ? null : `${navWidth}px`,
      'minmax(0, 1fr)',
      inspCollapsed ? null : 'var(--insp)',
    ]
      .filter(Boolean)
      .join(' '),
  }
  return (
    <div className="narrative-mode">
      <header className="nrt-head">
        <h1>
          {project.name} <span aria-hidden>/</span> <b>Narrative</b>
        </h1>
        {(!libraryOpen || worldOpen) && (
          <button
            type="button"
            className="btn"
            onClick={() => {
              setLibraryOpen(true)
              setWorldOpen(false)
            }}
          >
            Back to scenes
          </button>
        )}
        <div className="nrt-head-actions">
          <button
            type="button"
            className="btn"
            aria-pressed={worldOpen}
            onClick={() => setWorldOpen(true)}
          >
            World state
          </button>
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
      <div className="nrt-library-view" hidden={!libraryOpen || worldOpen}>
        <NarrativeLibrary
          projectKey={project.path}
          rows={rows}
          quests={world.data?.document.quests}
          questsLoading={world.isLoading}
          questsError={world.isError ? String(world.error) : undefined}
          catalog={catalog.data}
          loading={catalog.isPending}
          error={catalog.isError ? String(catalog.error) : undefined}
          readOnly={project.readOnly}
          navCollapsed={navCollapsed}
          nameOf={nameOf}
          onOpen={open}
          onCreateScene={() =>
            createScene.mutate('New scene', {
              onSuccess: (file) => open({ sceneId: file.scene.id }, 'flow'),
            })
          }
        />
      </div>
      {editorOpened && (
        <div className="nrt-editor-view" style={editorStyle} hidden={libraryOpen || worldOpen}>
          {!navCollapsed && (
            <nav className="nrt-scene-outline" aria-label="Current scene outline">
              <h3>{selected?.name ?? 'Selected scene'}</h3>
              {(selected?.beats ?? []).map((beat) => (
                <button
                  key={beat.id}
                  type="button"
                  aria-current={selection.beatId === beat.id ? 'true' : undefined}
                  onClick={() =>
                    selectNarrative({ sceneId: selected!.id, beatId: beat.id }, 'library')
                  }
                >
                  {beat.title}
                </button>
              ))}
            </nav>
          )}
          <NarrativeCentre readOnly={project.readOnly} projectKey={project.path} />
          {!inspCollapsed && <NarrativeInspector />}
        </div>
      )}
      {worldOpen && <NarrativeWorldPane projectKey={project.path} readOnly={project.readOnly} />}
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
