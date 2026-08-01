import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { BOARD_ASSET_MIME } from '../../lib/board'
import { buildGroups, indexNodes } from '../../lib/tree'
import { kindDef, kindIndex, summary } from '../../test/fixtures'
import { useUI } from '../../store/ui'
import { Navigator } from './Navigator'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))

const nodes = [summary({ id: 'kael', name: 'Kael', kind: 'character' })]
const kinds = kindIndex([kindDef('character', { label: 'Character', plural: 'Characters' })])

function transfer(type: string, value: string) {
  return {
    types: [type],
    effectAllowed: 'all',
    dropEffect: 'none',
    setData: vi.fn(),
    getData: (requested: string) => (requested === type ? value : ''),
  }
}

beforeEach(() => {
  h.invoke.mockReset()
  h.invoke.mockImplementation((command: string) =>
    Promise.resolve(command === 'node_links' ? [] : null),
  )
  useUI.setState({ filter: '', selectedId: null, collapsedNodes: {}, closedGroups: {} })
})

describe('Navigator board drops', () => {
  it('accepts board assets on a node without routing them through node reparenting', () => {
    const onAssetDrop = vi.fn()
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={qc}>
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
          onAssetDrop={onAssetDrop}
        />
      </QueryClientProvider>,
    )
    const row = screen.getByRole('button', { name: /Kael/ })
    const dataTransfer = transfer(BOARD_ASSET_MIME, 'asset-7')
    fireEvent.dragOver(row, { dataTransfer })
    fireEvent.drop(row, { dataTransfer })

    expect(onAssetDrop).toHaveBeenCalledWith('asset-7', 'kael')
    expect(h.invoke).not.toHaveBeenCalledWith('node_move', expect.anything())
  })
})
