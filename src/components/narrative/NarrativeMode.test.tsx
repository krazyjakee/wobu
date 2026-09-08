import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ProjectSummary } from '../../lib/api'
import { qk } from '../../lib/queries/keys'
import { useUI } from '../../store/ui'
import { NarrativeMode } from './NarrativeMode'
import { libraryRows, libraryScene } from './sceneLibrary.fixture'
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
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  h.invoke.mockReset()
  h.invoke.mockResolvedValue([])
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
  it('opens on the main library and enters a scene without cloning its source', () => {
    renderMode()
    expect(screen.getByRole('main', { name: 'Scene library' })).toBeInTheDocument()
    expect(screen.queryByRole('main', { name: 'Narrative editor' })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Open Council hearing in Flow' }))
    expect(screen.getByRole('main', { name: 'Narrative editor' })).toBeInTheDocument()
    expect(screen.getByRole('navigation', { name: 'Current scene outline' })).toHaveTextContent(
      'Evidence',
    )
    expect(useUI.getState().narrative.sceneId).toBe('council')
    expect(h.invoke.mock.calls.some(([command]) => String(command).includes('save'))).toBe(false)
  })
  it('restores query and preserves mounted unsaved edits on a library round trip', () => {
    renderMode()
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'attack' } })
    fireEvent.click(screen.getByRole('button', { name: 'Open Council hearing in Script' }))
    expect(useUI.getState().narrative).toEqual({
      sceneId: 'council',
      beatId: 'evidence',
      lineId: 'line',
    })
    expect(useSceneLibrary.getState().searchVariant).toEqual({
      sceneId: 'council',
      slotId: 'line',
      variantId: 'high',
    })
    fireEvent.change(screen.getByLabelText('Unsaved draft'), { target: { value: 'Still writing' } })
    fireEvent.click(screen.getByRole('button', { name: 'Back to scenes' }))
    expect(screen.getByRole('searchbox')).toHaveValue('attack')
    fireEvent.click(screen.getByRole('button', { name: 'Open Council hearing in Script' }))
    expect(screen.getByLabelText('Unsaved draft')).toHaveValue('Still writing')
  })
  it('honours collapsed panels in both library and scene editor', () => {
    useUI.setState({ navCollapsed: true, inspCollapsed: true })
    renderMode()
    expect(screen.queryByRole('navigation', { name: 'Narrative library' })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Open Council hearing in Flow' }))
    expect(screen.queryByRole('navigation', { name: 'Current scene outline' })).toBeNull()
    expect(screen.queryByRole('complementary', { name: 'Narrative context' })).toBeNull()
  })
  it('keeps unimplemented review and builds explicit while enabling native export', () => {
    renderMode()
    for (const name of ['Review'])
      expect(screen.getByRole('button', { name })).toHaveAttribute('aria-disabled', 'true')
    expect(screen.getByRole('button', { name: 'Export…' })).toBeEnabled()
    expect(screen.getByRole('contentinfo', { name: 'Narrative diagnostics' })).toHaveTextContent(
      'Branch reachability has not been checked',
    )
  })
})
