import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ProjectSummary } from '../../lib/api'
import { qk } from '../../lib/queries/keys'
import { useUI } from '../../store/ui'
import { NarrativeMode } from './NarrativeMode'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

/**
 * The workspace with an empty project in it.
 *
 * The catalog is seeded rather than awaited, so these tests stay about the
 * shell — the panes, the tab strip, the one shared selection — rather than
 * about when a query settles. An empty catalog is the honest default here: it
 * is the state a new project is in, and it is the one state in which the Flow
 * tab draws the demonstration arc.
 */
function renderMode(project: ProjectSummary = defaultProject) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  qc.setQueryData(qk.narrativeScenes, { scenes: [], unreadable: [] })
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
    narrativeFilters: { needsText: false, needsReview: false, outOfDate: false },
  })
})

describe('the workspace shell', () => {
  it('draws the Library, the centre views and the context pane', () => {
    renderMode()
    expect(screen.getByRole('navigation', { name: 'Narrative library' })).toBeInTheDocument()
    expect(screen.getByRole('main', { name: 'Narrative editor' })).toBeInTheDocument()
    expect(screen.getByRole('complementary', { name: 'Narrative context' })).toBeInTheDocument()
    expect(screen.getByRole('tablist', { name: 'Narrative views' })).toBeInTheDocument()
  })

  it('honours the panel toggles the rest of the workspace already has', () => {
    // `[` and `]` are the existing shortcuts. Narrative reads the same two
    // flags rather than adding a second pair nobody has bound.
    useUI.setState({ navCollapsed: true, inspCollapsed: true })
    renderMode()
    expect(screen.queryByRole('navigation', { name: 'Narrative library' })).toBeNull()
    expect(screen.queryByRole('complementary', { name: 'Narrative context' })).toBeNull()
    expect(screen.getByRole('main', { name: 'Narrative editor' })).toBeInTheDocument()
  })

  it('refuses Review, Build and Export with the reason each one is refused for', () => {
    renderMode()
    for (const [name, expected] of [
      ['Review', /nothing has been generated/i],
      ['Build…', /needs compiled narrative source/i],
      ['Export…', /no compiler/i],
    ] as const) {
      const button = screen.getByRole('button', { name })
      expect(button).toHaveAttribute('aria-disabled', 'true')
      fireEvent.focusIn(button)
      expect(screen.getByRole('tooltip')).toHaveTextContent(expected)
      fireEvent.focusOut(button)
    }
  })

  it('keeps the search field focusable so it can say why it answers nothing', () => {
    // `disabled` would have made this unreachable by keyboard, and "why is the
    // search box dead" is precisely the question this build has to answer.
    renderMode()
    const search = screen.getByRole('searchbox', { name: 'Find a scene or a line' })
    expect(search).toHaveAttribute('aria-disabled', 'true')
    fireEvent.focusIn(search)
    expect(screen.getByRole('tooltip')).toHaveTextContent('not in it yet')
  })

  it('says what the diagnostics are, and what they are not', () => {
    // The status line is scoped to the selected scene, and it is explicit that
    // reading the source is not compiling it and not proving reachability.
    // "No problems found" from a check that cannot look is the reassuring
    // failure this workspace has to avoid.
    renderMode()
    const foot = screen.getByRole('contentinfo', { name: 'Narrative diagnostics' })
    expect(foot).toHaveTextContent(/Choose a scene to see what is wrong with it/i)
    expect(foot).toHaveTextContent(/not a compilation/i)
    expect(foot).toHaveTextContent(/not a reachability proof/i)
  })
})

describe('one selection, read by every view', () => {
  it('carries a beat chosen elsewhere across a tab switch', () => {
    // Written as the Flow canvas will write it. Script is a different component
    // that has never heard of Flow; they agree because they read one field.
    useUI.getState().selectNarrative({ sceneId: 'council', beatId: 'present' }, 'flow')
    renderMode()
    // Flow opens on the canvas. What it draws is its own business; what
    // matters here is that the one selection survives the switch.
    expect(screen.getByRole('tabpanel')).toHaveTextContent('Auto layout')

    fireEvent.click(screen.getByRole('tab', { name: 'Script' }))

    expect(screen.getByRole('tabpanel')).toHaveTextContent('Selected beat present')
    expect(useUI.getState().narrative).toEqual({
      sceneId: 'council',
      beatId: 'present',
      lineId: null,
    })
  })

  it('still has the selection after a round trip through every tab', () => {
    useUI.getState().selectNarrative({ sceneId: 'council', beatId: 'present' }, 'library')
    renderMode()
    for (const name of ['Script', 'Preview', 'Source', 'Flow']) {
      fireEvent.click(screen.getByRole('tab', { name }))
    }
    expect(useUI.getState().narrative.beatId).toBe('present')
    expect(screen.getByRole('tabpanel')).toHaveTextContent('Auto layout')
  })

  it('walks back up the path from the inspector', () => {
    useUI
      .getState()
      .selectNarrative({ sceneId: 'council', beatId: 'present', lineId: 'mira-1' }, 'script')
    renderMode()

    fireEvent.click(screen.getByRole('button', { name: 'Beat present' }))
    expect(useUI.getState().narrative).toEqual({
      sceneId: 'council',
      beatId: 'present',
      lineId: null,
    })

    fireEvent.click(screen.getByRole('button', { name: 'Scene council' }))
    expect(useUI.getState().narrative).toEqual({
      sceneId: 'council',
      beatId: null,
      lineId: null,
    })
  })

  it('says nothing is selected rather than showing an empty context', () => {
    renderMode()
    expect(screen.getByRole('complementary', { name: 'Narrative context' })).toHaveTextContent(
      /Nothing is selected/i,
    )
    expect(screen.getByRole('heading', { name: 'No scene selected' })).toBeInTheDocument()
  })
})

describe('the centre tabs', () => {
  it('moves with the arrow keys and wraps, as a tab strip does', () => {
    renderMode()
    const strip = screen.getByRole('tablist', { name: 'Narrative views' })
    fireEvent.keyDown(strip, { key: 'ArrowRight' })
    expect(useUI.getState().narrativeTab).toBe('script')
    fireEvent.keyDown(strip, { key: 'ArrowLeft' })
    fireEvent.keyDown(strip, { key: 'ArrowLeft' })
    expect(useUI.getState().narrativeTab).toBe('source')
    fireEvent.keyDown(strip, { key: 'Home' })
    expect(useUI.getState().narrativeTab).toBe('flow')
  })

  it('leaves exactly one tab selected and one tab stop in the strip', () => {
    renderMode()
    const tabs = screen.getAllByRole('tab')
    expect(tabs.filter((tab) => tab.getAttribute('aria-selected') === 'true')).toHaveLength(1)
    expect(tabs.filter((tab) => tab.getAttribute('tabindex') === '0')).toHaveLength(1)
  })

  it('says which of Preview and Source is missing, and does not pretend to load', () => {
    renderMode()
    fireEvent.click(screen.getByRole('tab', { name: 'Preview' }))
    expect(
      screen.getByRole('heading', { name: 'Preview is not in this build' }),
    ).toBeInTheDocument()
    fireEvent.click(screen.getByRole('tab', { name: 'Source' }))
    expect(
      screen.getByRole('heading', { name: 'Source editing is not in this build' }),
    ).toBeInTheDocument()
  })
})
