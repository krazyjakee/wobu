import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { LinkEdge, NodeSummary } from '../../lib/api'
import { kindDef, kindIndex, summary } from '../../test/fixtures'
import { layoutRelationships, RelationshipGraph } from './RelationshipGraph'
import { GRAPH_NODE_LIMIT } from './relationshipGraphLimit'

const kinds = kindIndex([
  kindDef('species', { label: 'Species', plural: 'Species', layer: 'ancestry' }),
  kindDef('culture', { label: 'Culture', plural: 'Cultures', layer: 'culture' }),
  kindDef('character', { label: 'Character', plural: 'Characters' }),
])

const nodes: NodeSummary[] = [
  summary({ id: 'vashk', kind: 'species', name: 'Vashk' }),
  summary({ id: 'guild', kind: 'culture', name: 'Ember Guild', parentId: null }),
  summary({ id: 'inner', kind: 'culture', name: 'Inner Circle', parentId: 'guild' }),
  summary({ id: 'kael', kind: 'character', name: 'Kael', summary: 'ashwalker' }),
]

const links: LinkEdge[] = [
  { fromId: 'kael', toId: 'vashk', role: 'species_of', weight: 0.8, enabled: true },
  { fromId: 'kael', toId: 'guild', role: 'member_of', weight: 1, enabled: false },
]

describe('relationship layout', () => {
  it('combines implicit parent edges with explicit cross-kind influences', () => {
    const layout = layoutRelationships(nodes, links, kinds)
    expect(layout.edges.map((edge) => [edge.kind, edge.fromId, edge.toId])).toEqual([
      ['parent', 'inner', 'guild'],
      ['influence', 'kael', 'vashk'],
      ['influence', 'kael', 'guild'],
    ])
    expect(layout.groups.map((group) => group.kind)).toEqual(['species', 'culture', 'character'])
  })

  it('counts broken endpoints without drawing misleading edges', () => {
    const layout = layoutRelationships(
      nodes,
      [...links, { fromId: 'kael', toId: 'missing', role: 'located_in', weight: 1, enabled: true }],
      kinds,
    )
    expect(layout.edges).toHaveLength(3)
    expect(layout.dangling).toBe(1)
  })
})

describe('read-only relationship graph', () => {
  it('explains its non-editing role and selects a node for navigation', () => {
    const onSelect = vi.fn()
    render(
      <RelationshipGraph
        nodes={nodes}
        links={links}
        kinds={kinds}
        selectedId="kael"
        filter=""
        loading={false}
        error={null}
        onSelect={onSelect}
      />,
    )

    expect(screen.getByText('Read-only map')).toBeTruthy()
    expect(screen.getByText('Select a node to open it. Edit links in Relations.')).toBeTruthy()
    expect(screen.getByRole('img', { name: '4 nodes and 3 relationships' })).toBeTruthy()
    expect(screen.getByText('Kael inherits from Vashk · Species · 0.80')).toBeTruthy()
    expect(
      screen.getByText('Kael inherits from Ember Guild · Member of · 1.00 · muted'),
    ).toBeTruthy()

    const guild = screen.getByRole('button', { name: 'Open Ember Guild, Culture' })
    expect(guild.getAttribute('draggable')).toBeNull()
    fireEvent.click(guild)
    expect(onSelect).toHaveBeenCalledWith('guild')
  })

  it('uses the navigator filter to dim misses without destroying topology', () => {
    render(
      <RelationshipGraph
        nodes={nodes}
        links={links}
        kinds={kinds}
        selectedId={null}
        filter="guild"
        loading={false}
        error={null}
        onSelect={() => {}}
      />,
    )

    expect(
      screen.getByRole('button', { name: 'Open Ember Guild, Culture' }).className,
    ).not.toContain('is-dim')
    expect(screen.getByRole('button', { name: 'Open Kael, Character' }).className).toContain(
      'is-dim',
    )
  })

  it('clearly bounds graph rendering for a 10,000-node world', () => {
    const many = Array.from({ length: 10_000 }, (_, index) =>
      summary({
        id: `large-${index}`,
        name: `Large ${index}`,
        kind: 'character',
      }),
    )

    const view = render(
      <RelationshipGraph
        nodes={many}
        links={[]}
        kinds={kinds}
        selectedId="large-9999"
        filter="9999"
        loading={false}
        error={null}
        onSelect={() => {}}
      />,
    )

    expect(view.container.querySelectorAll('.graph-node')).toHaveLength(GRAPH_NODE_LIMIT)
    expect(screen.getByRole('button', { name: 'Open Large 9999, Character' })).toBeTruthy()
    expect(screen.getByRole('status')).toHaveTextContent('Showing 500 of 10,000 nodes')
    expect(screen.getByRole('status')).toHaveTextContent('Tree remains complete')
  })
})
