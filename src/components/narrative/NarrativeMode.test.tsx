import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ProjectSummary } from '../../lib/api'
import { qk } from '../../lib/queries/keys'
import { useUI } from '../../store/ui'
import { NarrativeMode } from './NarrativeMode'
import { libraryPage, libraryRows, libraryScene } from './sceneLibrary.fixture'
import { useScriptDrafts, sceneEditKey } from './scriptDrafts'
import { useSceneLibrary } from './sceneLibraryStore'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))
vi.mock('./NarrativeCentre', () => ({
  NarrativeCentre: () => (
    <main aria-label="Narrative editor">
      <input aria-label="Unsaved draft" defaultValue="" />
    </main>
  ),
}))
// Stood in for rather than driven, because what is under test is where the
// workspace lands when a diagnostic hands back a line — not the review queue's
// own paging, filters and guarded writes, which have their own tests.
vi.mock('./NarrativeReview', () => ({
  NarrativeReview: ({ onSource }: { onSource: (target: Record<string, unknown>) => void }) => (
    <button
      type="button"
      onClick={() => onSource({ scene: 'council', beat: 'evidence', slot: 'line', variant: null })}
    >
      Open the source of this line
    </button>
  ),
}))

function renderMode(project: ProjectSummary = defaultProject) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity }, mutations: { retry: false } },
  })
  qc.setQueryData(qk.narrativeScenes, {
    scenes: libraryRows.map((row) => row.summary),
    unreadable: [],
  })
  qc.setQueryData(qk.narrativeScene(libraryScene.id), {
    scene: libraryScene,
    slug: 'council',
    rel: 'council.yaml',
    stamp: null,
  })
  qc.setQueryData(qk.nodes, [])
  qc.setQueryData(qk.narrativeState, {
    document: { schema_version: 1, variables: [] },
    stamp: null,
  })
  qc.setQueryData(qk.projectCurrent, project)
  qc.setQueryData(['narrative_world'], {
    document: {
      schema_version: 2,
      facts: [],
      knowledge: [],
      relationships: [],
      events: [],
      quests: [],
      restrictions: [],
    },
    diagnostics: [],
    stamp: null,
  })
  return render(
    <QueryClientProvider client={qc}>
      <NarrativeMode project={project} />
    </QueryClientProvider>,
  )
}
const defaultProject: ProjectSummary = {
  id: 'project',
  name: 'Ashfall',
  path: '/project',
  onNetworkShare: false,
  readOnly: false,
  lastOpenedAt: null,
}
beforeEach(() => {
  localStorage.clear()
  useScriptDrafts.setState({ drafts: {} })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  h.invoke.mockReset()
  h.invoke.mockImplementation((command) =>
    Promise.resolve(
      command === 'narrative_library_query'
        ? libraryPage
        : command === 'narrative_texts'
          ? { assets: [], unreadable: [] }
          : [],
    ),
  )
  useUI.setState({
    mode: 'narrative',
    navWidth: 272,
    navCollapsed: false,
    inspCollapsed: false,
    bands: {},
    narrative: { sceneId: null, beatId: null, lineId: null },
    narrativeReveal: null,
    narrativeTab: 'flow',
  })
})

describe('Narrative discovery and editor handoff', () => {
  it('opens on the main library and enters a scene without cloning its source', async () => {
    renderMode()
    expect(screen.getByRole('main', { name: 'Scene library' })).toBeInTheDocument()
    expect(screen.queryByRole('main', { name: 'Narrative editor' })).toBeNull()
    fireEvent.click(await screen.findByRole('button', { name: 'Open Council hearing in Flow' }))
    expect(screen.getByRole('main', { name: 'Narrative editor' })).toBeInTheDocument()
    expect(screen.getByRole('navigation', { name: 'Current scene outline' })).toHaveTextContent(
      'Evidence',
    )
    expect(useUI.getState().narrative.sceneId).toBe('council')
    expect(h.invoke.mock.calls.some(([command]) => String(command).includes('save'))).toBe(false)
    expect(
      h.invoke.mock.calls.filter(([command]) => command === 'narrative_scene_get'),
    ).toHaveLength(0)
  })
  it('restores query and preserves mounted unsaved edits on a library round trip', async () => {
    renderMode()
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'attack' } })
    fireEvent.click(await screen.findByRole('button', { name: 'Open Council hearing in Script' }))
    expect(useUI.getState().narrative).toEqual({
      sceneId: 'council',
      beatId: 'evidence',
      lineId: 'line',
      variantId: 'high',
    })
    expect(useSceneLibrary.getState().searchVariant).toEqual({
      sceneId: 'council',
      slotId: 'line',
      variantId: 'high',
    })
    fireEvent.change(screen.getByLabelText('Unsaved draft'), { target: { value: 'Still writing' } })
    const back = screen.getByRole('button', { name: 'Back to scenes' })
    fireEvent.click(back)
    expect(screen.getByRole('searchbox')).toHaveValue('attack')
    fireEvent.click(await screen.findByRole('button', { name: 'Open Council hearing in Script' }))
    expect(screen.getByLabelText('Unsaved draft')).toHaveValue('Still writing')
  })
  it('honours collapsed panels in both library and scene editor', async () => {
    useUI.setState({ navCollapsed: true, inspCollapsed: true })
    renderMode()
    expect(screen.queryByRole('navigation', { name: 'Narrative library' })).toBeNull()
    fireEvent.click(await screen.findByRole('button', { name: 'Open Council hearing in Flow' }))
    expect(screen.queryByRole('navigation', { name: 'Current scene outline' })).toBeNull()
    expect(screen.queryByRole('complementary', { name: 'Narrative context' })).toBeNull()
  })
  it('opens compact auxiliary panes and restores the toggle focus on Escape without losing edits', async () => {
    useUI.setState({ navCollapsed: true, inspCollapsed: true })
    renderMode()
    fireEvent.click(await screen.findByRole('button', { name: 'Open Council hearing in Flow' }))
    fireEvent.change(screen.getByLabelText('Unsaved draft'), { target: { value: 'Keep this' } })
    const outline = screen.getByRole('button', { name: 'Scene outline' })
    fireEvent.click(outline)
    const beat = within(
      screen.getByRole('navigation', { name: 'Current scene outline' }),
    ).getByRole('button', { name: 'Evidence' })
    await waitFor(() => expect(beat).toHaveFocus())
    fireEvent.keyDown(screen.getByLabelText('Unsaved draft'), { key: 'Escape' })
    expect(outline).toHaveAttribute('aria-expanded', 'true')
    const consumeEscape = (event: Event) => event.preventDefault()
    beat.addEventListener('keydown', consumeEscape)
    fireEvent.keyDown(beat, { key: 'Escape' })
    expect(outline).toHaveAttribute('aria-expanded', 'true')
    beat.removeEventListener('keydown', consumeEscape)
    fireEvent.keyDown(beat, { key: 'Escape' })
    expect(outline).toHaveFocus()
    expect(outline).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByRole('navigation', { name: 'Current scene outline' })).toBeNull()
    const context = screen.getByRole('button', { name: 'Context' })
    fireEvent.click(context)
    expect(screen.getByRole('complementary', { name: 'Narrative context' })).toBeInTheDocument()
    fireEvent.keyDown(screen.getByRole('complementary', { name: 'Narrative context' }), {
      key: 'Escape',
    })
    expect(context).toHaveFocus()
    expect(screen.getByLabelText('Unsaved draft')).toHaveValue('Keep this')
  })
  it('shows an unfinished shared draft in the outline and context inspector', async () => {
    const scene = {
      ...libraryScene,
      name: 'Unfinished hearing',
      beats: [{ ...libraryScene.beats![0]!, title: 'Unfinished evidence' }],
    }
    useScriptDrafts.getState().put(sceneEditKey(defaultProject.path, scene.id), {
      file: { scene: libraryScene, slug: 'council', rel: 'council.yaml', stamp: null },
      scene,
    })
    renderMode()
    fireEvent.click(await screen.findByRole('button', { name: 'Open Council hearing in Flow' }))
    expect(screen.getByRole('navigation', { name: 'Current scene outline' })).toHaveTextContent(
      'Unfinished evidence',
    )
    expect(screen.getByRole('complementary', { name: 'Narrative context' })).toHaveTextContent(
      'Unfinished hearing',
    )
    expect(screen.getByText('Unsaved scene draft.')).toBeInTheDocument()
  })
  it('returns to the scenes the Back button names, from any full-width surface', async () => {
    renderMode()
    await screen.findByRole('button', { name: 'Open Council hearing in Flow' })
    fireEvent.click(screen.getByRole('button', { name: 'Text library' }))
    expect(await screen.findByRole('region', { name: 'Text library' })).toBeInTheDocument()
    expect(screen.queryByRole('main', { name: 'Scene library' })).toBeNull()
    // World state and the Text library are siblings rather than layers, so the
    // header's one Back button has to clear whichever of them is showing.
    fireEvent.click(screen.getByRole('button', { name: 'World state' }))
    fireEvent.click(screen.getByRole('button', { name: 'Back to scenes' }))
    expect(screen.getByRole('main', { name: 'Scene library' })).toBeInTheDocument()
    expect(screen.queryByRole('region', { name: 'Text library' })).toBeNull()
  })
  it('opens a scene in front of the text library rather than behind it', async () => {
    renderMode()
    await screen.findByRole('button', { name: 'Open Council hearing in Flow' })
    fireEvent.click(screen.getByRole('button', { name: 'Text library' }))
    await screen.findByRole('region', { name: 'Text library' })
    fireEvent.click(screen.getByRole('button', { name: 'Review' }))
    fireEvent.click(screen.getByRole('button', { name: 'Open the source of this line' }))
    expect(screen.getByRole('main', { name: 'Narrative editor' })).toBeInTheDocument()
    expect(screen.queryByRole('region', { name: 'Text library' })).toBeNull()
    expect(useUI.getState().narrative.beatId).toBe('evidence')
  })
  it('enables review and native export while keeping analysis limits explicit', () => {
    renderMode()
    expect(screen.getByRole('button', { name: 'Review' })).toBeEnabled()
    expect(screen.getByRole('button', { name: 'Export…' })).toBeEnabled()
    expect(screen.getByRole('contentinfo', { name: 'Narrative diagnostics' })).toHaveTextContent(
      'Branch reachability has not been checked',
    )
  })
})
