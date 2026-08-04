import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { chordParts } from '../../lib/keys'
import { buildGroups, indexNodes } from '../../lib/tree'
import { useUI } from '../../store/ui'
import { kindDef, kindIndex, summary } from '../../test/fixtures'
import { Navigator } from './Navigator'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))

const nodes = [summary({ id: 'kael', name: 'Kael', kind: 'character' })]
const kinds = kindIndex([kindDef('character', { label: 'Character', plural: 'Characters' })])
const newNode = vi.fn()

beforeEach(() => {
  h.invoke.mockReset()
  h.invoke.mockImplementation((command: string) =>
    Promise.resolve(command === 'node_links' ? [] : null),
  )
  newNode.mockReset()
  useUI.setState({ filter: '', selectedId: null, collapsedNodes: {}, closedGroups: {}, bands: {} })
})

function showNavigator() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <Navigator
        nodes={nodes}
        byId={indexNodes(nodes)}
        pinned={[]}
        groups={buildGroups(nodes, ['character'], kinds)}
        kinds={kinds}
        loading={false}
        error={null}
        readOnly={false}
        corrupt={[]}
        editedElsewhere={new Map()}
        projectPath="/project"
        onNewNode={newNode}
      />
    </QueryClientProvider>,
  )
}

describe('Navigator context menu accessibility', () => {
  it('uses menu-item roles, roving keys, Escape, and restores focus to its row', () => {
    showNavigator()
    const opener = screen.getByRole('button', { name: /Kael/ })
    expect(opener).not.toHaveAttribute('aria-current')
    fireEvent.click(opener)
    expect(opener).toHaveAttribute('aria-current', 'true')

    fireEvent.contextMenu(opener, { clientX: 40, clientY: 70 })

    const menu = screen.getByRole('menu', { name: 'Actions for Kael' })
    const items = within(menu).getAllByRole('menuitem')
    expect(items.map((item) => item.textContent?.trim())).toEqual([
      'New character',
      'New child of Kael',
      'Add to favourites',
      'Duplicate',
      'Delete',
    ])
    expect(items[0]).toHaveFocus()

    const last = items.length - 1
    fireEvent.keyDown(items[0] as HTMLElement, { key: 'End' })
    expect(items[last]).toHaveFocus()
    fireEvent.keyDown(items[last] as HTMLElement, { key: 'ArrowDown' })
    expect(items[0]).toHaveFocus()
    fireEvent.keyDown(items[0] as HTMLElement, { key: 'ArrowUp' })
    expect(items[last]).toHaveFocus()
    fireEvent.keyDown(items[last] as HTMLElement, { key: 'Home' })
    expect(items[0]).toHaveFocus()

    fireEvent.keyDown(items[0] as HTMLElement, { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
    expect(opener).toHaveFocus()
  })

  /*
   * The keyboard route to the same menu (#130).
   *
   * The row's star and its twist are deliberately not tab stops — a thousand
   * rows would be three thousand of them — so this menu is the *only* way to
   * favourite a node without a mouse. Which makes Shift+F10 and the Menu key
   * part of the feature rather than a nicety.
   */
  it('opens from Shift+F10 and from the Menu key, not only from the right button', () => {
    showNavigator()
    const row = screen.getByRole('button', { name: /Kael/ })
    row.focus()

    fireEvent.keyDown(row, { key: 'F10', shiftKey: true })
    expect(screen.getByRole('menu', { name: 'Actions for Kael' })).toBeInTheDocument()
    fireEvent.keyDown(screen.getAllByRole('menuitem')[0] as HTMLElement, { key: 'Escape' })
    expect(row).toHaveFocus()

    // The Menu key, which is the one a Windows keyboard has and Shift+F10 is
    // the fallback for. Choosing a row closes the menu and hands focus back.
    fireEvent.keyDown(row, { key: 'ContextMenu' })
    expect(screen.getByRole('menuitem', { name: 'Add to favourites' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('menuitem', { name: 'New child of Kael' }))
    expect(newNode).toHaveBeenCalledWith('character', 'kael')
    expect(screen.queryByRole('menu')).toBeNull()
    expect(row).toHaveFocus()
  })

  /*
   * A group heading used to *be* the action: right-clicking it opened the
   * new-entity sheet with no menu in between, which is the one gesture on this
   * pane that did something without asking. It now offers the two things a
   * heading can do, one of which has a chord — printed from the registry, so a
   * rebinding shows here without this menu knowing what it was rebound to.
   */
  it('offers a real menu on a group heading, with the live chord for the one that has one', () => {
    showNavigator()
    const heading = screen.getByRole('button', { name: /Characters/ })

    fireEvent.keyDown(heading, { key: 'F10', shiftKey: true })
    const menu = screen.getByRole('menu', { name: 'Actions for Characters' })
    expect(newNode).not.toHaveBeenCalled()
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent?.trim()),
    ).toEqual(['New character', expect.stringContaining('Collapse everything')])

    const collapse = within(menu).getByRole('menuitem', { name: /Collapse everything/ })
    for (const part of chordParts('Mod+Shift+C')) expect(collapse).toHaveTextContent(part)
    fireEvent.click(collapse)
    expect(useUI.getState().closedGroups.character).toBe(true)

    fireEvent.contextMenu(heading, { clientX: 5, clientY: 5 })
    fireEvent.click(screen.getByRole('menuitem', { name: 'New character' }))
    expect(newNode).toHaveBeenCalledWith('character', null)
  })
})
