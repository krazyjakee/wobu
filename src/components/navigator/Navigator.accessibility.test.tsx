import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { buildGroups, indexNodes } from '../../lib/tree'
import { useUI } from '../../store/ui'
import { kindDef, kindIndex, summary } from '../../test/fixtures'
import { Navigator } from './Navigator'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))

const nodes = [summary({ id: 'kael', name: 'Kael', kind: 'character' })]
const kinds = kindIndex([kindDef('character', { label: 'Character', plural: 'Characters' })])

beforeEach(() => {
  h.invoke.mockReset()
  h.invoke.mockImplementation((command: string) =>
    Promise.resolve(command === 'node_links' ? [] : null),
  )
  useUI.setState({ filter: '', selectedId: null, collapsedNodes: {}, closedGroups: {} })
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
        onNewNode={() => {}}
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
      'Duplicate',
      'Delete',
    ])
    expect(items[0]).toHaveFocus()

    fireEvent.keyDown(items[0] as HTMLElement, { key: 'End' })
    expect(items[3]).toHaveFocus()
    fireEvent.keyDown(items[3] as HTMLElement, { key: 'ArrowDown' })
    expect(items[0]).toHaveFocus()
    fireEvent.keyDown(items[0] as HTMLElement, { key: 'ArrowUp' })
    expect(items[3]).toHaveFocus()
    fireEvent.keyDown(items[3] as HTMLElement, { key: 'Home' })
    expect(items[0]).toHaveFocus()

    fireEvent.keyDown(items[0] as HTMLElement, { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
    expect(opener).toHaveFocus()
  })
})
