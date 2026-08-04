import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useUI } from '../store/ui'
import { GUIDE_PAGES, useGuide } from './GuideContent'
import { GuideLink, GuideRailButton } from './GuideLink'
import { GuideOverlay } from './GuideOverlay'

const h = vi.hoisted(() => ({ openUrl: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: h.openUrl }))

/*
 * The guide's job is to be reachable from where the reader is stuck, and to
 * work with nothing else available — no network, no provider, no project. So
 * what is tested here is reachability and containment: it opens at the page a
 * surface asked for, it moves between pages without leaving the window, and the
 * only thing that ever goes outside is a URL, handed to the system browser
 * rather than followed inside a webview that would replace the application.
 */

beforeEach(() => {
  h.openUrl.mockReset()
  h.openUrl.mockResolvedValue(undefined)
  useGuide.setState({ slug: null, anchor: null })
  useUI.setState({ shortcutsOpen: false, mode: 'library' })
})

describe('the in-app guide', () => {
  it('stays shut until asked for', () => {
    render(<GuideOverlay />)
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('opens at the page a surface asked for, not at the front', () => {
    useGuide.setState({ slug: 'influence', anchor: null })
    render(<GuideOverlay />)
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('The influence stack')
  })

  it('lists every page in its contents, and marks the open one', () => {
    useGuide.setState({ slug: 'workspace', anchor: null })
    render(<GuideOverlay />)
    const contents = screen.getByRole('navigation', { name: 'Guide contents' })
    for (const page of GUIDE_PAGES) {
      expect(within(contents).getByRole('button', { name: page.title })).toBeInTheDocument()
    }
    expect(
      within(contents).getByRole('button', { name: 'The workspace' }).getAttribute('aria-current'),
    ).toBe('page')
  })

  it('moves between pages from the contents and from the foot of a page', () => {
    useGuide.setState({ slug: 'index', anchor: null })
    render(<GuideOverlay />)

    const contents = screen.getByRole('navigation', { name: 'Guide contents' })
    fireEvent.click(within(contents).getByRole('button', { name: 'Generating' }))
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Generating')

    fireEvent.click(screen.getByRole('button', { name: /Previous/ }))
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('The influence stack')
  })

  it('follows a cross-reference inside the window', () => {
    useGuide.setState({ slug: 'index', anchor: null })
    render(<GuideOverlay />)
    const page = screen.getByRole('article')
    fireEvent.click(within(page).getByRole('button', { name: 'The workspace' }))
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('The workspace')
    expect(h.openUrl).not.toHaveBeenCalled()
  })

  it('hands a repository document to the system browser instead of following it', () => {
    useGuide.setState({ slug: 'reference', anchor: null })
    render(<GuideOverlay />)
    fireEvent.click(screen.getByRole('button', { name: 'Influence engine' }))
    expect(h.openUrl).toHaveBeenCalledWith('../04-influence-engine.md')
  })

  it('opens the live shortcuts reference rather than printing chords as fact', () => {
    useGuide.setState({ slug: 'reference', anchor: null })
    render(<GuideOverlay />)
    fireEvent.click(screen.getByRole('button', { name: 'shortcuts reference' }))
    expect(useUI.getState().shortcutsOpen).toBe(true)
    expect(useGuide.getState().slug).toBeNull()
  })

  it('searches its own text and jumps to the section that matched', () => {
    useGuide.setState({ slug: 'index', anchor: null })
    render(<GuideOverlay />)

    fireEvent.change(screen.getByLabelText('Search the guide'), {
      target: { value: 'Credential Manager' },
    })
    const results = screen.getByRole('navigation', { name: 'Guide contents' })
    fireEvent.click(within(results).getByRole('button', { name: /Providers and keys/ }))

    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Providers and keys')
    expect(useGuide.getState().anchor).toBe('how-keys-are-stored')
  })

  it('says so when nothing matches', () => {
    useGuide.setState({ slug: 'index', anchor: null })
    render(<GuideOverlay />)
    fireEvent.change(screen.getByLabelText('Search the guide'), {
      target: { value: 'zzzzznothing' },
    })
    expect(screen.getByText('No matches')).toBeInTheDocument()
  })
})

describe('the affordances that open it', () => {
  it('opens the guide at the page beside the feature', () => {
    render(<GuideLink page="influence" anchor="layer-cards" />)
    fireEvent.click(screen.getByRole('button', { name: 'Learn more' }))
    expect(useGuide.getState()).toMatchObject({ slug: 'influence', anchor: 'layer-cards' })
  })

  it('opens it at the front from the rail', () => {
    render(<GuideRailButton />)
    fireEvent.click(screen.getByRole('button', { name: 'Guide' }))
    expect(useGuide.getState()).toMatchObject({ slug: 'index', anchor: null })
  })
})
