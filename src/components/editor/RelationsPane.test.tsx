import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { kindDef, kindIndex, node, summary } from '../../test/fixtures'
import { RelationsPane } from './RelationsPane'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const kinds = kindIndex([
  kindDef('species', { layer: 'ancestry' }),
  kindDef('culture', { layer: 'culture' }),
  kindDef('setting', { layer: 'place' }),
  kindDef('character', {
    layer: 'subject',
    defaultLinkRoles: ['species_of', 'member_of', 'located_in'],
  }),
])
const subject = node({ id: 'hero', kind: 'character', name: 'Hero' })

beforeEach(() => {
  h.invoke.mockReset()
  h.invoke.mockImplementation((command: string) => {
    if (command === 'node_list') {
      return Promise.resolve([
        summary({ id: 'hero', kind: 'character', name: 'Hero' }),
        summary({ id: 'human', kind: 'species', name: 'Human' }),
        summary({ id: 'guild', kind: 'culture', name: 'Guild' }),
        summary({ id: 'harbour', kind: 'setting', name: 'Harbour' }),
      ])
    }
    if (command === 'node_backlinks') return Promise.resolve([])
    return Promise.resolve(null)
  })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('influence relation target picker', () => {
  it('offers only nodes in the layer selected by the first dropdown', async () => {
    render(
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <RelationsPane
          node={subject}
          def={kinds.get('character')}
          kinds={kinds}
          readOnly={false}
          onJump={() => {}}
        />
      </QueryClientProvider>,
    )

    const role = (await screen.findByLabelText('Relation role')) as HTMLSelectElement
    const target = screen.getByLabelText('Relation target')
    expect(within(target).getByRole('option', { name: 'Human' })).toBeInTheDocument()
    expect(within(target).queryByRole('option', { name: 'Guild' })).not.toBeInTheDocument()

    fireEvent.change(role, { target: { value: 'member_of' } })
    expect(within(target).getByRole('option', { name: 'Guild' })).toBeInTheDocument()
    expect(within(target).queryByRole('option', { name: 'Human' })).not.toBeInTheDocument()

    fireEvent.change(role, { target: { value: 'located_in' } })
    expect(within(target).getByRole('option', { name: 'Harbour' })).toBeInTheDocument()
    expect(within(target).queryByRole('option', { name: 'Guild' })).not.toBeInTheDocument()
  })
})
