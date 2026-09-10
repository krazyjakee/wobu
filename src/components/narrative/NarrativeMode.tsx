import { useEffect, useRef, useState, type CSSProperties } from 'react'
import type { ProjectSummary } from '../../lib/api'
import {
  useCreateScene,
  useDiagnoseScene,
  useRenameScene,
  useSceneDiagnostics,
  useScene,
  useScenes,
} from '../../lib/queries'
import { useUI, type NarrativeTarget } from '../../store/ui'
import { Icon } from '../Icon'
import { NarrativeCentre } from './NarrativeCentre'
import { NarrativeInspector } from './NarrativeInspector'
import { NarrativeVariants } from './NarrativeVariants'
import { NarrativeBuild } from './NarrativeBuild'
import { NarrativeScenarioTests } from './NarrativeScenarioTests'
import { NarrativeRecovery } from './NarrativeRecovery'
import { NarrativeReview } from './NarrativeReview'
import { NarrativeGeneration } from './NarrativeGeneration'
import { NarrativeExport } from './NarrativeExport'
import { NarrativeWording } from './NarrativeWording'
import { NarrativeWorldPane } from './NarrativeWorldPane'
import { NarrativeSourcePane } from './NarrativeSourcePane'
import { NarrativeExample } from './NarrativeExample'
import { NarrativeLibrary } from './NarrativeLibrary'
import { NarrativeTextLibrary } from './NarrativeTextLibrary'
import { useNarrativeTexts } from '../../lib/queries/narrativeText'
import type { ReviewTarget } from '../../lib/api/narrativeReview'
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
  const [textTarget, setTextTarget] = useState<ReviewTarget | undefined>()
  const [exampleOpen, setExampleOpen] = useState(false)
  const [exportOpen, setExportOpen] = useState(false)
  const [wordingOpen, setWordingOpen] = useState(false)
  const [recoveryOpen, setRecoveryOpen] = useState(false)
  const [generationOpen, setGenerationOpen] = useState(false)
  const [reviewOpen, setReviewOpen] = useState(false)
  const [buildOpen, setBuildOpen] = useState(false)
  const [initialBuildId, setInitialBuildId] = useState<string>()
  const [variantsOpen, setVariantsOpen] = useState(false)
  const [scenarioTestsOpen, setScenarioTestsOpen] = useState(false)
  const [sidePane, setSidePane] = useState<'outline' | 'context' | null>(null)
  const sidePaneButtons = useRef<HTMLDivElement>(null)
  const editorRoot = useRef<HTMLDivElement>(null)
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
  const textCatalog = useNarrativeTexts(project.path, reviewOpen || textLibraryOpen)
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
          {editorOpened && !libraryOpen && !worldOpen && !textLibraryOpen && (
            <div className="nrt-compact-pane-actions" ref={sidePaneButtons}>
              {(['outline', 'context'] as const).map((pane) => (
                <button
                  key={pane}
                  type="button"
                  className="btn"
                  data-pane-toggle={pane}
                  aria-expanded={sidePane === pane}
                  onClick={() => {
                    setSidePane(sidePane === pane ? null : pane)
                    const selector = pane === 'outline' ? '.nrt-scene-outline' : '.nrt-inspector'
                    if (sidePane !== pane)
                      requestAnimationFrame(() =>
                        editorRoot.current
                          ?.querySelector<HTMLElement>(`${selector} :is(button, input, select)`)
                          ?.focus(),
                      )
                  }}
                >
                  {pane === 'outline' ? 'Scene outline' : 'Context'}
                </button>
              ))}
            </div>
          )}
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
          <button
            type="button"
            className="btn"
            aria-pressed={wordingOpen}
            onClick={() => setWordingOpen(true)}
          >
            Repeated wording
          </button>
          <button
            className="btn"
            disabled={!!draft || !file.data?.scene.beats?.length}
            title={draft ? 'Save the scene draft before planning variants' : undefined}
            onClick={() => setVariantsOpen(true)}
          >
            Variants…
          </button>
          <button
            className="btn"
            onClick={() => {
              setInitialBuildId(undefined)
              setBuildOpen(true)
            }}
          >
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
          sceneName={(id) =>
            catalog.data?.scenes.find((scene) => scene.id === id)?.name ??
            textCatalog.data?.assets.find((asset) => asset.id === id)?.name ??
            id
          }
          speakerName={(id) => nameOf(id) ?? id}
          onClose={() => setReviewOpen(false)}
          onSource={(target) => {
            setReviewOpen(false)
            if (textCatalog.data?.assets.some((asset) => asset.id === target.scene)) {
              setTextTarget(target)
              setTextLibraryOpen(true)
              closeWorld()
              return
            }
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
      {variantsOpen && file.data && (
        <NarrativeVariants
          projectKey={project.path}
          scene={file.data.scene}
          beatId={selection.beatId ?? file.data.scene.beats?.[0]?.id ?? ''}
          readOnly={project.readOnly}
          onClose={() => setVariantsOpen(false)}
          onBuild={(id) => {
            setVariantsOpen(false)
            setInitialBuildId(id)
            setBuildOpen(true)
          }}
        />
      )}
      {buildOpen && (
        <NarrativeBuild
          projectKey={project.path}
          currentScene={selected?.id}
          initialBuildId={initialBuildId}
          readOnly={project.readOnly}
          onClose={() => setBuildOpen(false)}
          onScenarios={() => {
            setBuildOpen(false)
            setScenarioTestsOpen(true)
          }}
          onSource={(target, asset) => {
            setBuildOpen(false)
            if (asset) {
              setTextTarget(target)
              setTextLibraryOpen(true)
              closeWorld()
              return
            }
            open(
              { sceneId: target.scene, beatId: target.beat, lineId: target.slot },
              'script',
              target.variant ?? undefined,
            )
          }}
        />
      )}
      {scenarioTestsOpen && (
        <NarrativeScenarioTests
          readOnly={project.readOnly}
          onClose={() => setScenarioTestsOpen(false)}
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
      {wordingOpen && (
        <NarrativeWording readOnly={project.readOnly} onClose={() => setWordingOpen(false)} />
      )}
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
          ref={editorRoot}
          data-side-pane={sidePane ?? undefined}
          onKeyDown={(event) => {
            if (event.key !== 'Escape' || event.defaultPrevented || !sidePane) return
            const selector = sidePane === 'outline' ? '.nrt-scene-outline' : '.nrt-inspector'
            if (!(event.target as HTMLElement).closest(selector)) return
            event.preventDefault()
            event.stopPropagation()
            sidePaneButtons.current
              ?.querySelector<HTMLElement>(`[data-pane-toggle="${sidePane}"]`)
              ?.focus()
            setSidePane(null)
          }}
          style={editorStyle}
          hidden={libraryOpen || worldOpen || textLibraryOpen}
        >
          {(!navCollapsed || sidePane === 'outline') && (
            <nav className="nrt-scene-outline" aria-label="Current scene outline">
              <h3>{selected?.name ?? 'Selected scene'}</h3>
              {(selected?.beats ?? []).map((beat) => (
                <button
                  key={beat.id}
                  type="button"
                  aria-current={selection.beatId === beat.id ? 'true' : undefined}
                  onClick={() => {
                    selectNarrative({ sceneId: selected!.id, beatId: beat.id }, 'library')
                    setSidePane(null)
                  }}
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
          {(!inspCollapsed || sidePane === 'context') && (
            <NarrativeInspector projectKey={project.path} readOnly={project.readOnly} />
          )}
        </div>
      )}
      {textLibraryOpen && !worldOpen && (
        <NarrativeTextLibrary
          key={textTarget ? JSON.stringify(textTarget) : 'library'}
          initialTarget={textTarget}
          projectKey={project.path}
          readOnly={project.readOnly}
          nameOf={nameOf}
          onClose={() => setTextLibraryOpen(false)}
        />
      )}
      {worldOpen && <NarrativeWorldPane projectKey={project.path} readOnly={project.readOnly} />}
      <SceneDiagnosticsFooter projectKey={project.path} />
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
function SceneDiagnosticsFooter({ projectKey }: { projectKey: string }) {
  const sceneId = useUI((s) => s.narrative.sceneId)
  const draft = useScriptDrafts((state) =>
    sceneId ? state.drafts[sceneEditKey(projectKey, sceneId)]?.scene : undefined,
  )
  const found = useSceneDiagnostics(sceneId)
  const check = useDiagnoseScene()
  const diagnose = check.mutate
  useEffect(() => {
    if (draft && sceneId) diagnose({ sceneId, scene: draft })
  }, [draft, sceneId, diagnose])
  const current = check.variables?.scene === draft
  const checking = draft ? !current || check.isPending : found.isPending
  const error = draft ? (current && check.isError ? check.error : null) : found.error
  const count = (draft ? (current ? check.data?.length : 0) : found.data?.length) ?? 0
  return (
    <footer className="nrt-foot" aria-label="Narrative diagnostics">
      <Icon name={count > 0 ? 'x' : 'check'} size="sm" />
      {sceneId === null
        ? 'Choose a scene to see what is wrong with it. '
        : checking
          ? `Checking this ${draft ? 'unsaved ' : ''}scene… `
          : error
            ? `Could not read this scene’s diagnostics: ${String(error)}. `
            : `${count} problem${count === 1 ? '' : 's'} in this ${draft ? 'unsaved ' : ''}scene. `}
      {NARRATIVE_UNAVAILABLE.diagnostics}
    </footer>
  )
}
