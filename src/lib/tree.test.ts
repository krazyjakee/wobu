import { describe, expect, it } from 'vitest'
import {
  ancestorsOf,
  buildGroups,
  descendantsOf,
  filterTree,
  indexNodes,
  influenceDependentsOf,
} from './tree'
import type { TreeNode } from './tree'
import type { LinkEdge } from './api'
import { kindDef, kindIndex, summary } from '../test/fixtures'

const defs = kindIndex([kindDef('species'), kindDef('character'), kindDef('setting')])

/** Ids in render order, one flattened line per row, so order is visible. */
function flatten(roots: TreeNode[]): string[] {
  const out: string[] = []
  const walk = (ts: TreeNode[]) => {
    for (const t of ts) {
      out.push(`${'  '.repeat(t.depth)}${t.node.id}`)
      walk(t.children)
    }
  }
  walk(roots)
  return out
}

describe('buildGroups', () => {
  it('groups by kind and nests within a kind only', () => {
    const groups = buildGroups(
      [
        summary({ id: 'vashk', kind: 'species' }),
        summary({ id: 'kell', kind: 'character', parentId: 'vashk' }),
      ],
      ['species', 'character'],
      defs,
    )

    // `kell` names a species as its parent. Nesting is same-kind by design, so
    // it must surface as a root of its own group rather than vanish into the
    // species tree — a node the user cannot see is a node they cannot fix.
    expect(groups.map((g) => g.kind)).toEqual(['species', 'character'])
    expect(flatten(groups[0]!.roots)).toEqual(['vashk'])
    expect(flatten(groups[1]!.roots)).toEqual(['kell'])
  })

  it('follows the registry order, then appends kinds it was not told about', () => {
    const groups = buildGroups(
      [
        summary({ id: 'a', kind: 'setting' }),
        summary({ id: 'b', kind: 'species' }),
        summary({ id: 'c', kind: 'character' }),
      ],
      ['species', 'character'],
      defs,
    )
    // An unregistered kind is still shown — last, but shown. The alternative is
    // a node on disk with no row anywhere in the app.
    expect(groups.map((g) => g.kind)).toEqual(['species', 'character', 'setting'])
  })

  it('omits kinds with no nodes rather than showing empty groups', () => {
    const groups = buildGroups(
      [summary({ id: 'a', kind: 'species' })],
      ['species', 'character'],
      defs,
    )
    expect(groups.map((g) => g.kind)).toEqual(['species'])
  })

  it('sorts siblings case-insensitively and stamps depth', () => {
    const groups = buildGroups(
      [
        summary({ id: 'root', kind: 'species', name: 'Root' }),
        summary({ id: 'zeta', kind: 'species', name: 'zeta', parentId: 'root' }),
        summary({ id: 'alpha', kind: 'species', name: 'Alpha', parentId: 'root' }),
        summary({ id: 'deep', kind: 'species', name: 'Deep', parentId: 'alpha' }),
      ],
      ['species'],
      defs,
    )
    expect(flatten(groups[0]!.roots)).toEqual(['root', '  alpha', '    deep', '  zeta'])
  })

  it('counts every node in the kind, not just the roots', () => {
    const groups = buildGroups(
      [
        summary({ id: 'root', kind: 'species' }),
        summary({ id: 'kid', kind: 'species', parentId: 'root' }),
      ],
      ['species'],
      defs,
    )
    expect(groups[0]!.count).toBe(2)
  })

  it('keeps a node whose parent is missing entirely', () => {
    const groups = buildGroups(
      [summary({ id: 'orphan', kind: 'species', parentId: 'deleted-yesterday' })],
      ['species'],
      defs,
    )
    expect(flatten(groups[0]!.roots)).toEqual(['orphan'])
  })

  it('does not lose a node that is its own parent', () => {
    // Hand-edited frontmatter can say this. It must not become an invisible
    // self-referencing cycle.
    const groups = buildGroups(
      [summary({ id: 'self', kind: 'species', parentId: 'self' })],
      ['species'],
      defs,
    )
    expect(flatten(groups[0]!.roots)).toEqual(['self'])
  })
})

describe('filterTree', () => {
  const roots = buildGroups(
    [
      summary({ id: 'root', kind: 'species', name: 'Vashk', summary: 'ash-dwellers' }),
      summary({ id: 'kid', kind: 'species', name: 'Deepwalker', parentId: 'root' }),
      summary({ id: 'other', kind: 'species', name: 'Aurelian' }),
    ],
    ['species'],
    defs,
  )[0]!.roots

  it('returns the tree untouched for an empty or blank query', () => {
    expect(filterTree(roots, '')).toBe(roots)
    expect(filterTree(roots, '   ')).toBe(roots)
  })

  it('keeps a parent whose only match is a descendant', () => {
    // Otherwise the match is hidden inside a collapsed branch that was filtered
    // away, and the search looks broken.
    expect(flatten(filterTree(roots, 'deepwalker'))).toEqual(['root', '  kid'])
  })

  it('drops non-matching children of a matching parent', () => {
    expect(flatten(filterTree(roots, 'vashk'))).toEqual(['root'])
  })

  it('matches the summary as well as the name, case-insensitively', () => {
    expect(flatten(filterTree(roots, 'ASH-DWELL'))).toEqual(['root'])
  })

  it('returns nothing when nothing matches', () => {
    expect(filterTree(roots, 'no such thing')).toEqual([])
  })

  it('does not mutate the tree it was given', () => {
    const before = flatten(roots)
    filterTree(roots, 'vashk')
    expect(flatten(roots)).toEqual(before)
  })
})

describe('ancestorsOf', () => {
  const byId = indexNodes([
    summary({ id: 'a' }),
    summary({ id: 'b', parentId: 'a' }),
    summary({ id: 'c', parentId: 'b' }),
  ])

  it('runs outermost first — the breadcrumb reads left to right', () => {
    expect(ancestorsOf('c', byId).map((n) => n.id)).toEqual(['a', 'b'])
  })

  it('is empty for a root, and for an id that is not there', () => {
    expect(ancestorsOf('a', byId)).toEqual([])
    expect(ancestorsOf('ghost', byId)).toEqual([])
  })

  it('stops at a missing ancestor instead of returning a partial lie', () => {
    const broken = indexNodes([summary({ id: 'x', parentId: 'gone' })])
    expect(ancestorsOf('x', broken)).toEqual([])
  })

  it('terminates on a cycle', () => {
    // Two files that each name the other as parent. Without the `seen` set this
    // hangs the render thread, which is a frozen app, not a wrong breadcrumb.
    const cyclic = indexNodes([
      summary({ id: 'p', parentId: 'q' }),
      summary({ id: 'q', parentId: 'p' }),
    ])
    expect(ancestorsOf('p', cyclic).map((n) => n.id)).toEqual(['q'])
  })
})

describe('descendantsOf', () => {
  const nodes = [
    summary({ id: 'a' }),
    summary({ id: 'b', parentId: 'a' }),
    summary({ id: 'c', parentId: 'b' }),
    summary({ id: 'd', parentId: 'a' }),
    summary({ id: 'unrelated' }),
  ]

  it('collects the whole subtree, not just direct children', () => {
    expect([...descendantsOf('a', nodes)].sort()).toEqual(['b', 'c', 'd'])
  })

  it('excludes the node itself', () => {
    expect(descendantsOf('a', nodes).has('a')).toBe(false)
  })

  it('is empty for a leaf and for an unknown id', () => {
    expect(descendantsOf('c', nodes).size).toBe(0)
    expect(descendantsOf('ghost', nodes).size).toBe(0)
  })

  it('terminates on a cycle', () => {
    const cyclic = [summary({ id: 'p', parentId: 'q' }), summary({ id: 'q', parentId: 'p' })]
    expect([...descendantsOf('p', cyclic)].sort()).toEqual(['p', 'q'])
  })
})

describe('influenceDependentsOf', () => {
  const nodes = [
    summary({ id: 'style', kind: 'style_guide' }),
    summary({ id: 'world', kind: 'world_bible' }),
    summary({ id: 'vashk', kind: 'species' }),
    summary({ id: 'deep-vashk', kind: 'species', parentId: 'vashk' }),
    summary({ id: 'guild', kind: 'culture' }),
    summary({ id: 'kael', kind: 'character' }),
    summary({ id: 'friend', kind: 'character' }),
  ]
  const links = [
    { fromId: 'kael', toId: 'deep-vashk', role: 'species_of' as const, weight: 1, enabled: true },
    { fromId: 'kael', toId: 'guild', role: 'member_of' as const, weight: 1, enabled: true },
    { fromId: 'friend', toId: 'kael', role: 'related_to' as const, weight: 1, enabled: true },
  ]

  it('includes transitive inheritance and project roots, but stops beyond lateral sources', () => {
    expect(influenceDependentsOf('vashk', nodes, links).map((node) => node.id)).toEqual([
      'deep-vashk',
      'kael',
    ])
    expect(influenceDependentsOf('guild', nodes, links).map((node) => node.id)).toEqual(['kael'])
    expect(influenceDependentsOf('world', nodes, links).map((node) => node.id)).toEqual([
      'style',
      'vashk',
      'deep-vashk',
      'guild',
      'kael',
      'friend',
    ])
  })

  it('walks children and enabled links transitively while ignoring muted and dangling edges', () => {
    const source = summary({ id: 'source', kind: 'culture' })
    const child = summary({ id: 'child', kind: 'culture', parentId: source.id })
    const grandchild = summary({ id: 'grandchild', kind: 'culture', parentId: child.id })
    const linked = summary({ id: 'linked', kind: 'character' })
    const muted = summary({ id: 'muted', kind: 'character' })
    const unrelated = summary({ id: 'unrelated', kind: 'character' })

    expect(
      influenceDependentsOf(
        source.id,
        [source, unrelated, grandchild, muted, child, linked],
        [
          {
            fromId: linked.id,
            toId: grandchild.id,
            role: 'member_of',
            weight: 1,
            enabled: true,
          },
          {
            fromId: muted.id,
            toId: source.id,
            role: 'member_of',
            weight: 1,
            enabled: false,
          },
          {
            fromId: 'missing-subject',
            toId: source.id,
            role: 'member_of',
            weight: 1,
            enabled: true,
          },
          {
            fromId: unrelated.id,
            toId: 'missing-source',
            role: 'member_of',
            weight: 1,
            enabled: true,
          },
        ],
      ).map((node) => node.id),
    ).toEqual(['grandchild', 'child', 'linked'])
  })

  it('allows related_to into the source but never uses it deeper in the reverse walk', () => {
    const lateralSource = summary({ id: 'lateral-source', kind: 'culture' })
    const direct = summary({ id: 'direct', kind: 'culture' })
    const throughDirect = summary({ id: 'through-direct', kind: 'character' })
    const normalMiddle = summary({ id: 'normal-middle', kind: 'culture' })
    const stopped = summary({ id: 'stopped', kind: 'character' })
    const lateral = (fromId: string, toId: string): LinkEdge => ({
      fromId,
      toId,
      role: 'related_to',
      weight: 1,
      enabled: true,
    })
    const inherited = (fromId: string, toId: string): LinkEdge => ({
      fromId,
      toId,
      role: 'member_of',
      weight: 1,
      enabled: true,
    })

    expect(
      influenceDependentsOf(
        lateralSource.id,
        [lateralSource, stopped, throughDirect, normalMiddle, direct],
        [
          lateral(direct.id, lateralSource.id),
          inherited(throughDirect.id, direct.id),
          inherited(normalMiddle.id, lateralSource.id),
          lateral(stopped.id, normalMiddle.id),
        ],
      ).map((node) => node.id),
    ).toEqual(['through-direct', 'normal-middle', 'direct'])
  })

  it('makes each singleton root influence every other node', () => {
    const ordinary = summary({ id: 'ordinary', kind: 'character' })
    const world = summary({ id: 'world', kind: 'world_bible' })
    const style = summary({ id: 'style', kind: 'style_guide' })
    const all = [ordinary, world, style]

    expect(influenceDependentsOf(style.id, all, []).map((node) => node.id)).toEqual([
      'ordinary',
      'world',
    ])
    expect(influenceDependentsOf(world.id, all, []).map((node) => node.id)).toEqual([
      'ordinary',
      'style',
    ])
  })

  it('matches the indexed reverse rule when a root and lateral subject meet', () => {
    const source = summary({ id: 'source', kind: 'culture' })
    const middle = summary({ id: 'middle', kind: 'culture' })
    const style = summary({ id: 'style', kind: 'style_guide' })
    const lateral = summary({ id: 'lateral', kind: 'character' })
    const ordinary = summary({ id: 'ordinary', kind: 'character' })
    const links: LinkEdge[] = [
      {
        fromId: style.id,
        toId: middle.id,
        role: 'styled_by',
        weight: 1,
        enabled: true,
      },
      {
        fromId: lateral.id,
        toId: middle.id,
        role: 'related_to',
        weight: 1,
        enabled: true,
      },
      {
        fromId: middle.id,
        toId: source.id,
        role: 'member_of',
        weight: 1,
        enabled: true,
      },
    ]

    // `Index::dependents_of` applies the singleton shortcut only when the
    // singleton itself is the queried source. From any other source it admits
    // related_to on the first reverse hop only, so neither `lateral` nor an
    // unrelated subject is part of this downstream set.
    expect(
      influenceDependentsOf(source.id, [lateral, source, ordinary, style, middle], links).map(
        (node) => node.id,
      ),
    ).toEqual(['style', 'middle'])
  })

  it('terminates on mixed parent and link cycles without including the source itself', () => {
    const source = summary({ id: 'source', kind: 'culture', parentId: 'parent' })
    const parent = summary({ id: 'parent', kind: 'culture', parentId: source.id })
    const dependent = summary({ id: 'dependent', kind: 'character' })
    const links: LinkEdge[] = [
      {
        fromId: dependent.id,
        toId: parent.id,
        role: 'member_of',
        weight: 1,
        enabled: true,
      },
      {
        fromId: parent.id,
        toId: dependent.id,
        role: 'located_in',
        weight: 1,
        enabled: true,
      },
    ]

    expect(
      influenceDependentsOf(source.id, [dependent, source, parent], links).map((node) => node.id),
    ).toEqual(['dependent', 'parent'])
  })

  it('returns an empty list for an unknown source', () => {
    expect(influenceDependentsOf('missing', nodes, links)).toEqual([])
  })

  it('handles a few-thousand-node reverse traversal as one lookup', () => {
    const count = 5_000
    const large = Array.from({ length: count }, (_, index) =>
      summary({
        id: `node-${index}`,
        kind: 'culture',
        parentId: index === 0 ? null : `node-${index - 1}`,
      }),
    )

    const result = influenceDependentsOf(large[0]!.id, large, [])
    expect(result).toHaveLength(count - 1)
    expect(result[0]!.id).toBe('node-1')
    expect(result.at(-1)!.id).toBe(`node-${count - 1}`)
  })
})

describe('indexNodes', () => {
  it('handles undefined, which is what a pending query hands it', () => {
    expect(indexNodes(undefined).size).toBe(0)
  })
})
