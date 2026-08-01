import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { InfluenceStack, KindDef, LayerCard, NodeSummary, WobuNode } from '../../lib/api'
import { kindDef, kindIndex, node as buildNode, summary } from '../../test/fixtures'
import { useUI } from '../../store/ui'
import { Editor } from './Editor'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const selected: NodeSummary = summary({ id: 'kael', name: 'Kael Vantris' })
const selectedNode: WobuNode = buildNode({ id: selected.id, name: selected.name })
const kinds: KindDef[] = [
  kindDef('character', { label: 'Character' }),
  kindDef('species', { label: 'Species', layer: 'ancestry' }),
  kindDef('culture', { label: 'Culture', layer: 'culture' }),
  kindDef('setting', { label: 'Setting', layer: 'place' }),
  kindDef('style_guide', { label: 'Art Style', layer: 'style' }),
  kindDef('world_bible', { label: 'World Canon', layer: 'world' }),
]

function card(over: Partial<LayerCard> & Pick<LayerCard, 'layer' | 'nodeId' | 'name'>): LayerCard {
  return {
    kind: null,
    reached: 'root',
    distance: 0,
    weight: 1,
    slider: 1,
    fragments: [],
    ...over,
  }
}

const stack: InfluenceStack = {
  subjectId: selected.id,
  preset: {
    id: 'character-sheet',
    label: 'Character sheet',
    kinds: ['character'],
    defaultFor: ['character'],
    priorities: [],
    framing: '',
    aspect: '1:1',
    images: 1,
    views: [],
    imageConstraints: null,
  },
  layers: [
    card({ layer: 'style', nodeId: 'style', name: 'Ashfall style', kind: 'style_guide' }),
    card({ layer: 'world', nodeId: 'world', name: 'Ashfall', kind: 'world_bible' }),
    card({ layer: 'ancestry', nodeId: 'vashk', name: 'Vashk', kind: 'species' }),
    card({ layer: 'culture', nodeId: 'guild', name: 'Ember Guild', kind: 'culture' }),
    card({ layer: 'place', nodeId: 'bay', name: 'Cinder Bay', kind: 'setting' }),
    card({
      layer: 'subject',
      nodeId: selected.id,
      name: selected.name,
      kind: 'character',
      reached: 'subject',
    }),
  ],
}

function renderEditor(onJump = vi.fn(), readOnly = false) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={qc}>
      <Editor
        selected={selected}
        kinds={kindIndex(kinds)}
        readOnly={readOnly}
        onJump={onJump}
        hasNodes
        loading={false}
      />
    </QueryClientProvider>,
  )
  return onJump
}

beforeEach(() => {
  h.invoke.mockReset()
  h.invoke.mockImplementation((cmd: string) =>
    Promise.resolve(cmd === 'node_get' ? selectedNode : cmd === 'influence_resolve' ? stack : null),
  )
  useUI.setState({ tab: 'notes' })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('the editor influence breadcrumb', () => {
  it('does not ask for mesh metadata until the 3D tab is opened', async () => {
    renderEditor()
    await screen.findByDisplayValue('Kael Vantris')
    expect(h.invoke.mock.calls.some(([command]) => command === 'mesh_concepts')).toBe(false)

    fireEvent.click(screen.getByRole('button', { name: '3D' }))
    await screen.findByText('No meshes yet')
    expect(h.invoke).toHaveBeenCalledWith('mesh_concepts', { nodeId: selected.id })
  })

  it('shows the resolved hierarchy in stack order and every chip jumps to its node', async () => {
    const onJump = renderEditor()
    await screen.findByRole('button', { name: 'Vashk' }, { timeout: 5_000 })

    const chain = document.querySelector<HTMLElement>('.chain')
    if (!chain) throw new Error('breadcrumb was not rendered')
    const chips = within(chain).getAllByRole('button')
    expect(chips.map((chip) => chip.textContent)).toEqual(['Vashk', 'Ember Guild', 'Cinder Bay'])
    expect(within(chain).queryByRole('button', { name: 'Ashfall style' })).toBeNull()
    expect(within(chain).queryByRole('button', { name: 'Ashfall' })).toBeNull()

    for (const chip of chips) fireEvent.click(chip)
    expect(onJump.mock.calls.map(([id]) => id)).toEqual(['vashk', 'guild', 'bay'])
  })

  it('states when the resolved stack has no hierarchy layers', async () => {
    h.invoke.mockImplementation((cmd: string) =>
      Promise.resolve(
        cmd === 'node_get'
          ? selectedNode
          : cmd === 'influence_resolve'
            ? { ...stack, layers: stack.layers.slice(-1) }
            : null,
      ),
    )
    renderEditor()

    expect(await screen.findByText('nothing')).toBeTruthy()
  })
})

describe('Enhance shortcut', () => {
  it.each([
    ['Command', { metaKey: true }],
    ['Control', { ctrlKey: true }],
  ])('starts Enhance with %s-E for the selected node', async (_platform, modifier) => {
    renderEditor()
    const enhance = await screen.findByRole('button', { name: 'Enhance' })
    await waitFor(() => expect(enhance).toBeEnabled())

    const dispatched = fireEvent.keyDown(window, { key: 'e', ...modifier })
    expect(dispatched).toBe(false)

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('enhance_start', { nodeId: selected.id }),
    )
  })

  it('does not Enhance while the shortcut originates in an editable control', async () => {
    renderEditor()
    const notes = await screen.findByPlaceholderText(/Messy is fine/)

    fireEvent.keyDown(notes, { key: 'e', metaKey: true })

    expect(h.invoke).not.toHaveBeenCalledWith('enhance_start', expect.anything())
  })

  it('does not Enhance when the visible action is unavailable on a read-only project', async () => {
    renderEditor(vi.fn(), true)
    await screen.findByDisplayValue('Kael Vantris')
    expect(screen.getByRole('button', { name: 'Enhance' })).toBeDisabled()

    fireEvent.keyDown(window, { key: 'e', ctrlKey: true })

    expect(h.invoke).not.toHaveBeenCalledWith('enhance_start', expect.anything())
  })
})
