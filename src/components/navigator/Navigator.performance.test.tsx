import { Profiler } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { NodeKind, NodeSummary } from '../../lib/api'
import { buildGroups, indexNodes } from '../../lib/tree'
import { useUI } from '../../store/ui'
import { kindDef, kindIndex, summary } from '../../test/fixtures'
import { Navigator } from './Navigator'
import { buildNavigatorRows, type NavigatorBuildStats } from './navigatorRows'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))

const kinds = kindIndex([
  kindDef('character', { label: 'Character', plural: 'Characters' }),
  kindDef('culture', { label: 'Culture', plural: 'Cultures' }),
  kindDef('setting', { label: 'Setting', plural: 'Settings' }),
])

function largeWorld(count: number): NodeSummary[] {
  return Array.from({ length: count }, (_, index) =>
    summary({
      id: `node-${String(index).padStart(5, '0')}`,
      name: `Node ${String(index).padStart(5, '0')}`,
      kind: 'character',
    }),
  )
}

beforeEach(() => {
  h.invoke.mockReset()
  h.invoke.mockImplementation((command: string) =>
    Promise.resolve(command === 'node_links' ? [] : null),
  )
  useUI.setState({ filter: '', selectedId: null, collapsedNodes: {}, closedGroups: {} })
})

describe('large Navigator rendering', () => {
  it('bounds a 10,000-node DOM and rerenders only the newly selected row', () => {
    const nodes = largeWorld(10_000)
    const groups = buildGroups(nodes, ['character'], kinds)
    const rowRender = vi.fn()
    const profiler = vi.fn()
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })

    const view = render(
      <QueryClientProvider client={client}>
        <Profiler id="large-navigator" onRender={profiler}>
          <Navigator
            nodes={nodes}
            byId={indexNodes(nodes)}
            pinned={[]}
            groups={groups}
            kinds={kinds}
            loading={false}
            error={null}
            readOnly={false}
            corrupt={[]}
            editedElsewhere={new Map()}
            projectPath="/project"
            onNewNode={() => {}}
            onRowRender={rowRender}
          />
        </Profiler>
      </QueryClientProvider>,
    )

    const mountedRows = view.container.querySelectorAll('.nav-virtual-window .node')
    expect(mountedRows.length).toBeGreaterThan(0)
    expect(mountedRows.length).toBeLessThan(50)
    expect(view.container.querySelectorAll('*').length).toBeLessThan(500)

    rowRender.mockClear()
    profiler.mockClear()
    fireEvent.click(screen.getByRole('button', { name: /^Node 00000/ }))

    expect(profiler).toHaveBeenCalled()
    expect(rowRender.mock.calls.map(([id]) => id)).toEqual(['node-00000'])
  })

  it('filters every group exactly once per flattened input', () => {
    const nodes = (['character', 'culture', 'setting'] as NodeKind[]).flatMap((kind, group) =>
      Array.from({ length: 100 }, (_, index) =>
        summary({ id: `${kind}-${index}`, name: `${kind} ${group}-${index}`, kind }),
      ),
    )
    const groups = buildGroups(nodes, ['character', 'culture', 'setting'], kinds)
    const stats: NavigatorBuildStats = { filteredGroups: 0 }

    const result = buildNavigatorRows({
      groups,
      filter: '2-9',
      closedGroups: {},
      collapsedNodes: {},
      bands: {},
      stats,
    })

    expect(stats.filteredGroups).toBe(groups.length)
    expect(result.hasMatches).toBe(true)
  })
})
