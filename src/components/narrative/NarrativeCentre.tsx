import { useId, useRef, type KeyboardEvent } from 'react'
import { useScenes } from '../../lib/queries'
import { useUI, NARRATIVE_TABS, type NarrativeTab } from '../../store/ui'
import { NarrativeProjectFlow } from './NarrativeProjectFlow'
import { NarrativePlaceholder } from './NarrativePlaceholder'
import { NarrativeScriptPane } from './NarrativeScriptPane'
import { NarrativeSourcePane } from './NarrativeSourcePane'
import { NARRATIVE_UNAVAILABLE } from './narrativeModel'

const TAB_LABEL: Record<NarrativeTab, string> = {
  flow: 'Flow',
  script: 'Script',
  preview: 'Preview',
  source: 'Source',
}

/**
 * The centre of the Narrative workspace: four views of the one selected scene.
 *
 * A real `tablist` rather than the editor's row of pressed buttons. These four
 * are views of the *same* thing — the tab is a lens, not a destination — and
 * that is exactly what the tab pattern means to a screen reader, arrow keys
 * included. Switching between them changes `narrativeTab` and nothing else, so
 * the beat a writer was reading on the canvas is the beat Script opens on.
 */
export function NarrativeCentre({
  readOnly = false,
  projectKey = '',
}: {
  readOnly?: boolean
  projectKey?: string
}) {
  const tab = useUI((s) => s.narrativeTab)
  const setTab = useUI((s) => s.setNarrativeTab)
  const sceneId = useUI((s) => s.narrative.sceneId)
  const catalog = useScenes()
  // The scene's name when the catalog knows it, and its id when it does not.
  // Ids are ULIDs, so the fallback is unreadable — but it is the truth, and a
  // heading that invented a friendlier name would be naming a scene that is
  // not in the project.
  const sceneName = catalog.data?.scenes.find((one) => one.id === sceneId)?.name ?? sceneId
  const ids = useId()
  const tabs = useRef<(HTMLButtonElement | null)[]>([])

  const move = (to: number) => {
    // Wraps, because a tab strip is a ring: Left from the first should reach
    // Source rather than stop dead.
    const index = (to + NARRATIVE_TABS.length) % NARRATIVE_TABS.length
    setTab(NARRATIVE_TABS[index]!)
    tabs.current[index]?.focus()
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const at = NARRATIVE_TABS.indexOf(tab)
    switch (event.key) {
      case 'ArrowRight':
        event.preventDefault()
        move(at + 1)
        return
      case 'ArrowLeft':
        event.preventDefault()
        move(at - 1)
        return
      case 'Home':
        event.preventDefault()
        move(0)
        return
      case 'End':
        event.preventDefault()
        move(NARRATIVE_TABS.length - 1)
        return
      default:
    }
  }

  return (
    <main className="nrt-centre" aria-label="Narrative editor">
      <header className="nrt-centre-head">
        <h2>{sceneId ? <code>{sceneName}</code> : 'No scene selected'}</h2>
        <p>
          {sceneId
            ? 'Flow, Script, Preview and Source are four views of this one scene.'
            : 'Choose a scene in the Library. Whatever you select there opens in every view here.'}
        </p>
      </header>

      <div className="tabs" role="tablist" aria-label="Narrative views" onKeyDown={onKeyDown}>
        {NARRATIVE_TABS.map((value, index) => (
          <button
            key={value}
            ref={(node) => {
              tabs.current[index] = node
            }}
            type="button"
            role="tab"
            id={`${ids}-tab-${value}`}
            aria-controls={`${ids}-panel-${value}`}
            aria-selected={tab === value}
            tabIndex={tab === value ? 0 : -1}
            className={tab === value ? 'tab is-active' : 'tab'}
            onClick={() => setTab(value)}
          >
            {TAB_LABEL[value]}
          </button>
        ))}
      </div>

      {/* Only the open panel is mounted. The selection lives in the store
          rather than in any of them, so unmounting one loses nothing — which is
          the whole reason the selection is not component state. */}
      <div
        className="nrt-panel"
        role="tabpanel"
        id={`${ids}-panel-${tab}`}
        aria-labelledby={`${ids}-tab-${tab}`}
        tabIndex={0}
      >
        {/* Flow opens at the arc. A designer opens this workspace to find a
            scene, and the arc is the only view that shows where scenes sit in
            relation to each other; the scene canvas is one double-click in. */}
        {tab === 'flow' && <NarrativeProjectFlow readOnly={readOnly} />}
        {tab === 'script' && <NarrativeScriptPane readOnly={readOnly} projectKey={projectKey} />}
        {tab === 'preview' && (
          <NarrativePlaceholder
            title="Preview is not in this build"
            reason={NARRATIVE_UNAVAILABLE.preview}
          />
        )}
        {tab === 'source' && <NarrativeSourcePane readOnly={readOnly} projectKey={projectKey} />}
      </div>
    </main>
  )
}
