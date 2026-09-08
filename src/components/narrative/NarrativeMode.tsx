import { useRef, useState, type CSSProperties } from 'react'
import type { ProjectSummary } from '../../lib/api'
import {
  useCreateScene,
  useRenameScene,
  useSceneDiagnostics,
  useScene,
  useScenes,
} from '../../lib/queries'
import { useUI, type NarrativeTarget } from '../../store/ui'
import { Icon } from '../Icon'
import { NarrativeCentre } from './NarrativeCentre'
import { NarrativeInspector } from './NarrativeInspector'
import { NarrativeScenarioTests } from './NarrativeScenarioTests'
import { NarrativeRecovery } from './NarrativeRecovery'
import { NarrativeReview } from './NarrativeReview'
import { NarrativeGeneration } from './NarrativeGeneration'
import { NarrativeExport } from './NarrativeExport'
import { NarrativeWorldPane } from './NarrativeWorldPane'
import { NarrativeSourcePane } from './NarrativeSourcePane'
import { NarrativeExample } from './NarrativeExample'
import { NarrativeLibrary } from './NarrativeLibrary'
import { NarrativeTextLibrary } from './NarrativeTextLibrary'
import { useNarrativeNames } from './flow/useNarrativeNames'
import { NARRATIVE_UNAVAILABLE } from './narrativeModel'
import { useSceneLibrary } from './sceneLibraryStore'
import { sceneEditKey, useScriptDrafts } from './scriptDrafts'

export function NarrativeMode({ project }: { project: ProjectSummary }) {
  return <NarrativeWorkspace key={project.path} project={project} />
}

function NarrativeWorkspace({ project }: { project: ProjectSummary }) {
  const navWidth = useUI((s) => s.navWidth)
  const navCollapsed = useUI((s) => s.navCollapsed)
  const inspCollapsed = useUI((s) => s.inspCollapsed)
  const selection = useUI((s) => s.narrative)
  const selectionTab = useUI((s) => s.narrativeTab)
  const selectNarrative = useUI((s) => s.selectNarrative)
  const setTab = useUI((s) => s.setNarrativeTab)
  const [libraryOpen, setLibraryOpen] = useState(true)
  const [textLibraryOpen, setTextLibraryOpen] = useState(false)
  const [exampleOpen, setExampleOpen] = useState(false)
  const [exportOpen, setExportOpen] = useState(false)
  const [recoveryOpen, setRecoveryOpen] = useState(false)
  const [generationOpen, setGenerationOpen] = useState(false)
  const [reviewOpen, setReviewOpen] = useState(false)
  const [buildOpen, setBuildOpen] = useState(false)
  const [repairRel, setRepairRel] = useState<string | null>(null)
  const worldTarget = useUI((state) => state.narrativeWorldTarget)
  const [closedWorldSeq, setClosedWorldSeq] = useState<number | null>(null)
  const [localWorldOpen, setWorldOpen] = useState(false)
  const worldOpen =
    localWorldOpen ||
    (worldTarget?.projectKey === project.path && worldTarget.seq !== closedWorldSeq)
  const closeWorld = () => {
    setClosedWorldSeq(worldTarget?.seq ?? null)
    setWorldOpen(false)
  }
  const [editorOpened, setEditorOpened] = useState(false)
  const catalog = useScenes()
  const file = useScene(selection.sceneId)
  const draft = useScriptDrafts((state) =>
    selection.sceneId ? state.drafts[sceneEditKey(project.path, selection.sceneId)] : undefined,
  )
  const libraryRoot = useRef<HTMLDivElement>(null)
  const libraryReturn = useRef<HTMLElement | null>(null)
  const createScene = useCreateScene()
  const renameScene = useRenameScene()
  const { nameOf } = useNarrativeNames()
  const selected = draft?.scene ?? file.data?.scene
  const open = (target: NarrativeTarget, tab: 'flow' | 'script', variantId?: string) => {
    useSceneLibrary.setState({
      searchVariant:
        target.sceneId && target.lineId && variantId
          ? { sceneId: target.sceneId, slotId: target.lineId, variantId }
          : null,
    })
    if (libraryRoot.current?.contains(document.activeElement))
      libraryReturn.current = document.activeElement as HTMLElement
    selectNarrative({ ...target, ...(variantId ? { variantId, field: 'text' } : {}) }, 'library', {
      projectKey: project.path,
    })
    setRepairRel(null)
    setTab(tab)
    setEditorOpened(true)
    setLibraryOpen(false)
    // Every full-width surface is dismissed, not just the library. World state
    // and the Text library are siblings of the editor rather than layers over
    // it, so one left open would hide the scene this call just selected —
    // Review's "open the source of this line" would look like it did nothing.
    setTextLibraryOpen(false)
    closeWorld()
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
              // The same reason `open` clears it: this button says "scenes",
              // and returning from World state to a Text library nobody closed
              // is not that.
              setTextLibraryOpen(false)
              closeWorld()
              requestAnimationFrame(() => {
                const original = libraryReturn.current
                const fallback = Array.from(
                  libraryRoot.current?.querySelectorAll<HTMLElement>('[data-library-scene]') ?? [],
                ).find(
                  (button) =>
                    button.dataset.libraryScene === selection.sceneId &&
                    button.dataset.libraryTab === selectionTab,
                )
                ;(original?.isConnected ? original : fallback)?.focus()
              })
            }}
          >
            Back to scenes
          </button>
        )}
        <div className="nrt-head-actions">
          <button className="btn" onClick={() => setGenerationOpen(true)}>
            Generate…
          </button>
          <button
            type="button"
            className="btn"
            aria-pressed={worldOpen}
            onClick={() => setWorldOpen(true)}
          >
            World state
          </button>
          <button
            type="button"
            className="btn"
            aria-pressed={textLibraryOpen}
            onClick={() => {
              setTextLibraryOpen(true)
              closeWorld()
            }}
          >
            Text library
          </button>
          <button className="btn" onClick={() => setReviewOpen(true)}>
            Review
          </button>
          <button className="btn" onClick={() => setBuildOpen(true)}>
            Build…
          </button>
          <button className="btn" onClick={() => setRecoveryOpen(true)}>
            Recovery…
          </button>
          <button className="btn" onClick={() => setExportOpen(true)}>
            Export…
          </button>
        </div>
      </header>
      {exampleOpen && (
        <NarrativeExample
          projectKey={project.path}
          readOnly={project.readOnly}
          onClose={() => setExampleOpen(false)}
          onOpen={(sceneId) => open({ sceneId }, 'script')}
        />
      )}
      {reviewOpen && (
        <NarrativeReview
          projectKey={project.path}
          readOnly={project.readOnly}
          sceneName={(id) => catalog.data?.scenes.find((scene) => scene.id === id)?.name ?? id}
          speakerName={(id) => nameOf(id) ?? id}
          onClose={() => setReviewOpen(false)}
          onSource={(target) => {
            setReviewOpen(false)
            open(
              { sceneId: target.scene, beatId: target.beat, lineId: target.slot },
              'script',
              target.variant ?? undefined,
            )
          }}
        />
      )}
      {generationOpen && (
        <NarrativeGeneration
          projectKey={project.path}
          scene={selected}
          readOnly={project.readOnly}
          onClose={() => setGenerationOpen(false)}
        />
      )}
      {buildOpen && (
        <NarrativeScenarioTests
          readOnly={project.readOnly}
          onClose={() => setBuildOpen(false)}
          onSource={(site) =>
            open(
              {
                sceneId: site.scene,
                beatId: site.beat,
                ...(site.slot ? { lineId: site.slot } : {}),
              },
              'script',
            )
          }
        />
      )}
      {recoveryOpen && (
        <NarrativeRecovery readOnly={project.readOnly} onClose={() => setRecoveryOpen(false)} />
      )}
      {exportOpen && <NarrativeExport onClose={() => setExportOpen(false)} />}
      <div
        ref={libraryRoot}
        className="nrt-library-view"
        hidden={!libraryOpen || worldOpen || textLibraryOpen}
      >
        <NarrativeLibrary
          projectKey={project.path}
          readOnly={project.readOnly}
          navCollapsed={navCollapsed}
          nameOf={nameOf}
          onOpen={open}
          onOpenExample={() => setExampleOpen(true)}
          onRepair={(rel) => {
            setRepairRel(rel)
            setEditorOpened(true)
            setLibraryOpen(false)
            closeWorld()
          }}
          onCreateScene={() =>
            createScene.mutate('New scene', {
              onSuccess: (file) => open({ sceneId: file.scene.id }, 'flow'),
            })
          }
          // Renaming stays here rather than inside the table for the same
          // reason creating does: the library draws rows, and every write in
          // this workspace is owned by the one component that also owns the
          // project it is writing to.
          onRenameScene={(sceneId, name) => renameScene.mutate({ sceneId, name })}
        />
      </div>
      {editorOpened && (
        <div
          className="nrt-editor-view"
          style={editorStyle}
          hidden={libraryOpen || worldOpen || textLibraryOpen}
        >
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
          {repairRel ? (
            <NarrativeSourcePane
              rel={repairRel}
              readOnly={project.readOnly}
              projectKey={project.path}
            />
          ) : (
            <NarrativeCentre
              active={!libraryOpen && !worldOpen && !textLibraryOpen}
              readOnly={project.readOnly}
              projectKey={project.path}
            />
          )}
          {!inspCollapsed && (
            <NarrativeInspector projectKey={project.path} readOnly={project.readOnly} />
          )}
        </div>
      )}
      {textLibraryOpen && !worldOpen && (
        <NarrativeTextLibrary
          projectKey={project.path}
          readOnly={project.readOnly}
          nameOf={nameOf}
          onClose={() => setTextLibraryOpen(false)}
        />
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
